//! The durable event outbox (Task 12, J11): the write-ahead file that makes
//! audit and mesh pulses survive the bounded bus.
//!
//! The bus itself is lossy by design — a bounded queue with drop-oldest, and
//! a 250 ms per-subscriber timeout after which a slow subscriber is
//! abandoned. For two of its consumers that loss is silent data loss: the
//! audit log (`guard_event_log`) has no way to reconstruct a guard call that
//! left no row, and the mesh outbox (`mesh_outbox`) can only reconcile what
//! it is told about. So a pulse that either of those subscribers is
//! interested in is *durability-qualified*: before the bus dispatches it,
//! `emit` appends it here — synchronously, `write` + `sync_data` — and two
//! per-class delivery workers (one audit, one mesh, in `bus.rs`) deliver
//! from this file rather than from the queue.
//!
//! ## Contract: at-least-once, never exactly-once
//!
//! A worker advances its class offset only *after* that class's subscriber
//! returns. A crash between the subscriber's side effect and the offset
//! write therefore redelivers the entry on the next start, and both
//! subscribers are built to absorb the duplicate: the mesh worker keys
//! records by id (a redelivery re-applies the same record), and the audit
//! log is append-only (a duplicate row is a faithful record of a
//! redelivery). No text in this module claims exactly-once delivery.
//!
//! ## Layout
//!
//! Two files in a daemon-owned directory (the daemon passes its config
//! directory, beside the token files):
//!
//! - `event-outbox.jsonl` — one JSON object per entry: `{seq, class,
//!   pulse}`. `seq` is a single sequence across both classes, so each
//!   class's offset is a high-water mark in one shared order.
//! - `event-outbox.offsets.json` — `{"audit": N, "mesh": M}`, the last
//!   sequence each class's worker has acknowledged. Rewritten (tmp +
//!   rename + `sync_data`) on every advance.
//!
//! ## Bounds
//!
//! Compaction runs when the outbox is opened: when the file exceeds
//! 8 MiB, or more than half its entries are past both class offsets, it is
//! rewritten keeping only entries their own class has not yet acknowledged.
//! The sequence base is preserved across the rewrite — offsets are
//! high-water marks, so the next sequence is recovered as one past the
//! greatest sequence ever recorded (entry or offset), never restarted at 1.

use super::ThalamicPulse;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The append-only entries file, inside the outbox directory.
pub const OUTBOX_FILE_NAME: &str = "event-outbox.jsonl";

/// The per-class offsets file, sibling of [`OUTBOX_FILE_NAME`].
pub const OFFSETS_FILE_NAME: &str = "event-outbox.offsets.json";

/// The size bound: past this many bytes, startup compaction rewrites the
/// outbox keeping only unacknowledged entries.
const MAX_OUTBOX_BYTES: u64 = 8 * 1024 * 1024;

/// A durability class: the unit of qualification, delivery, and offsets.
///
/// Qualification follows the *subscription registry*, never a hardcoded
/// list of event kinds: a pulse qualifies for a class iff that class's
/// named subscriber is registered **and** interested in the pulse. The
/// class↔subscriber↔interest mapping lives here because it is one fact —
/// which subscribers are durability-interested, and in what — stated once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventClass {
    /// The audit trail: `guard_event_log`'s record of guard calls.
    Audit,
    /// Mesh sync state: `mesh_outbox`'s capture of shareable mutations.
    Mesh,
}

impl EventClass {
    /// Both classes, in a stable order for worker startup.
    pub fn all() -> [EventClass; 2] {
        [EventClass::Audit, EventClass::Mesh]
    }

    /// The offsets-file key and log label for this class.
    pub fn as_str(&self) -> &'static str {
        match self {
            EventClass::Audit => "audit",
            EventClass::Mesh => "mesh",
        }
    }

    /// The class a registered subscriber belongs to, by its stable name.
    /// `None` is every other subscriber: never durability-qualified, never
    /// outbox-delivered.
    pub fn class_of_subscriber(name: &str) -> Option<EventClass> {
        match name {
            "guard_event_log" => Some(EventClass::Audit),
            "mesh_outbox" => Some(EventClass::Mesh),
            _ => None,
        }
    }

    /// Whether this class's subscriber acts on `pulse`. This mirrors the
    /// named subscribers' own matching, which is what "subscribes to its
    /// kind" means on a bus whose subscribers receive every pulse and
    /// no-op the ones they do not care about: `GuardEventLog` handles only
    /// `GuardedActionFired`; `MeshOutbox` handles the three storage
    /// mutations and returns early on a guard verdict.
    pub fn interested_in(&self, pulse: &ThalamicPulse) -> bool {
        match self {
            EventClass::Audit => {
                matches!(pulse, ThalamicPulse::GuardedActionFired { .. })
            }
            EventClass::Mesh => matches!(
                pulse,
                ThalamicPulse::Created { .. }
                    | ThalamicPulse::Superseded { .. }
                    | ThalamicPulse::Archived { .. }
            ),
        }
    }
}

/// One durable entry: the outbox line format, `{seq, class, pulse}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEntry {
    /// Position in the single cross-class sequence, assigned at append.
    pub seq: u64,
    /// The class this entry is delivered to (and acknowledged by).
    pub class: EventClass,
    /// The pulse itself, in its serde wire form (`event.rs`).
    pub pulse: ThalamicPulse,
}

/// Why a durable append or offset operation failed.
///
/// Returned by `ThalamicBus::emit_checked`, where a Class M publisher
/// fails its operation on a failed append — so this is a type callers
/// match on, not a log string.
#[derive(Debug)]
pub enum OutboxError {
    /// The emit was refused at the bus boundary: the caller's
    /// `RecursionToken` is held at depth ≥ 1. No append was attempted and
    /// the pulse never entered the queue.
    RecursionRefused,
    /// A filesystem operation on the outbox or offsets file failed.
    Io(std::io::Error),
    /// A pulse could not be encoded into its wire form.
    Serde(serde_json::Error),
}

impl fmt::Display for OutboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutboxError::RecursionRefused => write!(
                f,
                "emit refused: the caller holds a recursion token at depth >= 1"
            ),
            OutboxError::Io(err) => write!(f, "event outbox io error: {err}"),
            OutboxError::Serde(err) => {
                write!(f, "event outbox serialization error: {err}")
            }
        }
    }
}

impl std::error::Error for OutboxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OutboxError::RecursionRefused => None,
            OutboxError::Io(err) => Some(err),
            OutboxError::Serde(err) => Some(err),
        }
    }
}

impl From<std::io::Error> for OutboxError {
    fn from(err: std::io::Error) -> Self {
        OutboxError::Io(err)
    }
}

impl From<serde_json::Error> for OutboxError {
    fn from(err: serde_json::Error) -> Self {
        OutboxError::Serde(err)
    }
}

/// The offsets file's shape: one high-water mark per class.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct Offsets {
    audit: u64,
    mesh: u64,
}

impl Offsets {
    fn get(&self, class: EventClass) -> u64 {
        match class {
            EventClass::Audit => self.audit,
            EventClass::Mesh => self.mesh,
        }
    }

    fn set(&mut self, class: EventClass, seq: u64) {
        match class {
            EventClass::Audit => self.audit = seq,
            EventClass::Mesh => self.mesh = seq,
        }
    }
}

/// The outbox's mutable state. The entries vector is an in-memory mirror
/// of the file (same order, same contents): appends extend both together,
/// compaction rewrites both together, so a reader never races a parse.
struct Inner {
    /// The next sequence to assign. Recovered at open as one past the
    /// greatest recorded sequence — an entry's or an offset's, whichever
    /// is higher — so compaction can never restart the sequence at 1 and
    /// collide with the offsets that outlived the entries.
    next_seq: u64,
    entries: Vec<OutboxEntry>,
    offsets: Offsets,
}

/// The durable outbox over one directory. Send + Sync by construction
/// (one `Mutex` over the state); the bus shares it between `emit`'s
/// append path and the two delivery workers.
pub struct EventOutbox {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

impl EventOutbox {
    /// Open (creating if needed) the outbox in `dir`: load the offsets,
    /// load the entries, recover the sequence base, and run startup
    /// compaction. A line that does not parse — the torn tail of an append
    /// interrupted by a crash, whose publisher never saw success — is
    /// skipped with a warning; an offsets file that does not parse reads
    /// as all-zero, which redelivers: the safe direction under an
    /// at-least-once contract.
    pub fn open(dir: &Path) -> Result<Self, OutboxError> {
        std::fs::create_dir_all(dir)?;
        let file_path = dir.join(OUTBOX_FILE_NAME);
        if !file_path.exists() {
            // Create it now so no reader ever races a missing file.
            File::create(&file_path)?;
        }
        let offsets = read_offsets(dir);
        let mut entries = Vec::new();
        let text = std::fs::read_to_string(&file_path)?;
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<OutboxEntry>(line) {
                Ok(entry) => entries.push(entry),
                Err(err) => tracing::warn!(
                    "event outbox: skipping an unparseable line in {}: {err}",
                    file_path.display()
                ),
            }
        }
        entries.sort_by_key(|e| e.seq);
        let max_entry = entries.iter().map(|e| e.seq).max().unwrap_or(0);
        let next_seq = max_entry.max(offsets.audit).max(offsets.mesh) + 1;
        let outbox = Self {
            dir: dir.to_path_buf(),
            inner: Mutex::new(Inner {
                next_seq,
                entries,
                offsets,
            }),
        };
        // Startup compaction. A failure here must not take durability
        // down with it: appends still work against the uncompacted file,
        // and the next start tries again — so it is logged, not fatal.
        if let Err(err) = outbox.compact_if_needed() {
            tracing::error!("event outbox: startup compaction failed: {err}");
        }
        Ok(outbox)
    }

    fn file_path(&self) -> PathBuf {
        self.dir.join(OUTBOX_FILE_NAME)
    }

    /// Durably append `pulse` for `class`: encode the entry, `write` the
    /// line, `sync_data`, and only then record it in the mirror and return
    /// its sequence. A failed append changes nothing — no mirror entry, no
    /// sequence consumed — so the caller can fail the operation knowing
    /// the outbox does not half-hold the pulse.
    pub fn append(
        &self,
        class: EventClass,
        pulse: &ThalamicPulse,
    ) -> Result<u64, OutboxError> {
        let mut inner = self.inner.lock().expect("outbox lock poisoned");
        let seq = inner.next_seq;
        let entry = OutboxEntry {
            seq,
            class,
            pulse: pulse.clone(),
        };
        let line = serde_json::to_string(&entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.file_path())?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        drop(file);
        inner.entries.push(entry);
        inner.next_seq = seq + 1;
        Ok(seq)
    }

    /// This class's unacknowledged entries, in sequence order: everything
    /// past the class offset. This is the delivery worker's read.
    pub fn pending(&self, class: EventClass) -> Vec<OutboxEntry> {
        let inner = self.inner.lock().expect("outbox lock poisoned");
        let offset = inner.offsets.get(class);
        inner
            .entries
            .iter()
            .filter(|e| e.class == class && e.seq > offset)
            .cloned()
            .collect()
    }

    /// The last sequence this class has acknowledged.
    pub fn offset(&self, class: EventClass) -> u64 {
        self.inner
            .lock()
            .expect("outbox lock poisoned")
            .offsets
            .get(class)
    }

    /// Acknowledge `seq` for `class`: advance the offset in memory and
    /// rewrite the offsets file (tmp + rename + `sync_data`) before
    /// returning, so an acknowledged entry is never redelivered by a later
    /// process mistaking volatile state for durable. Offsets never move
    /// backwards.
    pub fn advance_offset(
        &self,
        class: EventClass,
        seq: u64,
    ) -> Result<(), OutboxError> {
        let mut inner = self.inner.lock().expect("outbox lock poisoned");
        if seq <= inner.offsets.get(class) {
            return Ok(());
        }
        inner.offsets.set(class, seq);
        write_offsets(&self.dir, inner.offsets)
    }

    /// Rewrite the entries file keeping only entries their own class has
    /// not yet acknowledged. The sequence base is untouched (it lives in
    /// the recovered high-water marks, not in the file's length).
    pub fn compact(&self) -> Result<(), OutboxError> {
        let mut inner = self.inner.lock().expect("outbox lock poisoned");
        self.compact_locked(&mut inner)
    }

    /// Compaction, when a bound says it is due: the file exceeds
    /// [`MAX_OUTBOX_BYTES`], or more than half the entries are past both
    /// class offsets (acknowledged by everyone who will ever read them).
    fn compact_if_needed(&self) -> Result<(), OutboxError> {
        let mut inner = self.inner.lock().expect("outbox lock poisoned");
        let size = std::fs::metadata(self.file_path())
            .map(|m| m.len())
            .unwrap_or(0);
        let total = inner.entries.len();
        let floor = inner.offsets.audit.min(inner.offsets.mesh);
        let dead = inner.entries.iter().filter(|e| e.seq <= floor).count();
        if size > MAX_OUTBOX_BYTES || (total > 0 && dead * 2 > total) {
            self.compact_locked(&mut inner)?;
        }
        Ok(())
    }

    fn compact_locked(&self, inner: &mut Inner) -> Result<(), OutboxError> {
        let kept: Vec<OutboxEntry> = inner
            .entries
            .iter()
            .filter(|e| e.seq > inner.offsets.get(e.class))
            .cloned()
            .collect();
        let tmp_path = self.dir.join("event-outbox.jsonl.tmp");
        {
            let mut file = File::create(&tmp_path)?;
            for entry in &kept {
                file.write_all(serde_json::to_string(entry)?.as_bytes())?;
                file.write_all(b"\n")?;
            }
            file.sync_data()?;
        }
        std::fs::rename(&tmp_path, self.file_path())?;
        inner.entries = kept;
        Ok(())
    }
}

/// The offsets, read tolerantly: a missing file is a fresh outbox; a file
/// that does not parse is reported and read as all-zero (redelivery is
/// the safe failure direction under at-least-once).
fn read_offsets(dir: &Path) -> Offsets {
    match std::fs::read_to_string(dir.join(OFFSETS_FILE_NAME)) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
            tracing::warn!("event outbox: offsets file unparseable ({err}); treating every entry as unacknowledged");
            Offsets::default()
        }),
        Err(_) => Offsets::default(),
    }
}

/// Rewrite the offsets file durably: write a sibling tmp file, `sync_data`
/// it, then rename over the real name, so a reader sees the old offsets or
/// the new ones, never a torn half.
fn write_offsets(dir: &Path, offsets: Offsets) -> Result<(), OutboxError> {
    let tmp_path = dir.join("event-outbox.offsets.json.tmp");
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(serde_json::to_string(&offsets)?.as_bytes())?;
        file.sync_data()?;
    }
    std::fs::rename(&tmp_path, dir.join(OFFSETS_FILE_NAME))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neurostrata-event-outbox-test-{}-{}-{}",
            std::process::id(),
            tag,
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn created(id: &str) -> ThalamicPulse {
        ThalamicPulse::Created {
            id: id.to_string(),
            namespace: "n".to_string(),
            kind: "fact".to_string(),
        }
    }

    #[test]
    fn append_pending_advance_round_trip() {
        let dir = scratch("round-trip");
        let ob = EventOutbox::open(&dir).expect("the outbox opens");
        let s1 = ob.append(EventClass::Mesh, &created("a")).unwrap();
        let s2 = ob.append(EventClass::Mesh, &created("b")).unwrap();
        assert_eq!((s1, s2), (1, 2), "sequences are assigned in append order");
        assert_eq!(ob.pending(EventClass::Mesh).len(), 2);
        assert!(ob.pending(EventClass::Audit).is_empty());
        ob.advance_offset(EventClass::Mesh, s1).unwrap();
        let pending = ob.pending(EventClass::Mesh);
        assert_eq!(pending.len(), 1, "the acknowledged entry leaves pending");
        assert_eq!(pending[0].seq, s2);
        drop(ob);

        // Reopen: the state is the files', not the process's.
        let ob = EventOutbox::open(&dir).expect("the outbox reopens");
        assert_eq!(ob.offset(EventClass::Mesh), s1);
        assert_eq!(ob.pending(EventClass::Mesh).len(), 1);
        let s3 = ob.append(EventClass::Mesh, &created("c")).unwrap();
        assert_eq!(s3, 3, "the sequence continues across a reopen");
    }

    #[test]
    fn class_interest_matches_the_named_subscribers() {
        let mesh_pulse = created("a");
        let guard_pulse = ThalamicPulse::GuardedActionFired {
            trace_id: "t".into(),
            action_type: "shell".into(),
            payload_hash: 1,
            verdict: "deny".into(),
            rule_ids_triggered: vec![],
            namespace: "n".into(),
        };
        assert!(EventClass::Mesh.interested_in(&mesh_pulse));
        assert!(!EventClass::Mesh.interested_in(&guard_pulse));
        assert!(EventClass::Audit.interested_in(&guard_pulse));
        assert!(!EventClass::Audit.interested_in(&mesh_pulse));
        assert_eq!(
            EventClass::class_of_subscriber("guard_event_log"),
            Some(EventClass::Audit)
        );
        assert_eq!(
            EventClass::class_of_subscriber("mesh_outbox"),
            Some(EventClass::Mesh)
        );
        assert_eq!(EventClass::class_of_subscriber("episodic_pointer_echo"), None);
    }
}
