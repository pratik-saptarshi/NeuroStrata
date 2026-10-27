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

/// The length of the clean prefix of an outbox file: the offset just past
/// the last newline-terminated, parseable entry. A trailing physical line
/// is clean only in that form; an unterminated line was never durably
/// receipted (append writes the line and newline before `sync_data`), and
/// an unparseable trailing line cannot be trusted as an entry boundary.
fn clean_outbox_len(bytes: &[u8]) -> usize {
    let mut end = bytes.len();
    while end > 0 {
        let terminated = bytes[end - 1] == b'\n';
        let line_end = end - usize::from(terminated);
        let line_start = bytes[..line_end]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let line = &bytes[line_start..line_end];
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            if terminated {
                return end;
            }
            end = line_start;
            continue;
        }
        if terminated && serde_json::from_slice::<OutboxEntry>(line).is_ok() {
            return end;
        }
        end = line_start;
    }
    0
}

/// The durable outbox over one directory. Send + Sync by construction
/// (one `Mutex` over the state); the bus shares it between `emit`'s
/// append path and the two delivery workers.
pub struct EventOutbox {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

/// The file operations the append write step needs, factored behind a
/// trait so a test can drive the step with a file whose write fails after
/// a partial write — the mid-line failure (a disk filling, a device
/// dropping) the rollback in [`write_line_durably`] exists for.
/// Production always passes the real [`File`]; the trait is a seam for
/// the write step only, never a behavior switch.
trait AppendTarget: Write {
    /// The file's current length: the point a failed append rolls back to.
    fn file_len(&self) -> std::io::Result<u64>;
    /// Flush the file's data to storage (`File::sync_data`).
    fn sync(&self) -> std::io::Result<()>;
    /// Truncate the file to `len` (`File::set_len`): the rollback itself.
    fn truncate_to(&self, len: u64) -> std::io::Result<()>;
}

impl AppendTarget for File {
    fn file_len(&self) -> std::io::Result<u64> {
        self.metadata().map(|metadata| metadata.len())
    }

    fn sync(&self) -> std::io::Result<()> {
        self.sync_data()
    }

    fn truncate_to(&self, len: u64) -> std::io::Result<()> {
        self.set_len(len)
    }
}

/// Write one entry line and sync it, atomically from the caller's point
/// of view: the file's length is recorded before the write, and on any
/// write or sync error the file is truncated back to that length (and
/// synced again, once the truncation itself has succeeded) before the
/// error returns. Without the rollback, the torn bytes of a mid-line
/// failure stay in the file and the next successful append glues onto
/// them; the glued physical line is then truncated as torn at the next
/// open, taking the later, successfully-receipted entry with it. A failed
/// append therefore leaves the file byte-identical to its pre-append
/// state — the same "changes nothing" the mirror already guarantees.
/// Callers hold the outbox lock across the whole call, so no other
/// append or compaction can interleave with the write + rollback.
fn write_line_durably(target: &mut impl AppendTarget, line: &[u8]) -> Result<(), OutboxError> {
    let prior_len = target.file_len()?;
    let outcome = target.write_all(line).and_then(|()| target.sync());
    if let Err(err) = outcome {
        if target.truncate_to(prior_len).is_ok() {
            let _ = target.sync();
        }
        return Err(err.into());
    }
    Ok(())
}

impl EventOutbox {
    /// Open (creating if needed) the outbox in `dir`: load the offsets,
    /// repair a torn tail, load the entries, recover the sequence base,
    /// and run startup compaction. Repair truncates an unterminated or
    /// unparseable final line back to the last clean boundary before any
    /// append can glue a new entry onto it. Other unparseable lines are
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
        let bytes = std::fs::read(&file_path)?;
        let clean_len = clean_outbox_len(&bytes);
        if clean_len < bytes.len() {
            let file = OpenOptions::new().write(true).open(&file_path)?;
            file.set_len(clean_len as u64)?;
            file.sync_data()?;
            tracing::warn!(
                "event outbox: truncated {} torn trailing byte(s) from {} at open",
                bytes.len() - clean_len,
                file_path.display()
            );
        }
        let text = String::from_utf8_lossy(&bytes[..clean_len]);
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
    /// sequence consumed, and no bytes: the write step ([`write_line_durably`])
    /// rolls the file back to its pre-append length on any write or sync
    /// failure, so the caller can fail the operation knowing the outbox
    /// does not half-hold the pulse, in the mirror or on disk.
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
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.file_path())?;
        write_line_durably(&mut file, &line)?;
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
        let mut new_offsets = inner.offsets;
        new_offsets.set(class, seq);
        write_offsets(&self.dir, new_offsets)?;
        inner.offsets = new_offsets;
        Ok(())
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
                let mut line = serde_json::to_vec(entry)?;
                line.push(b'\n');
                file.write_all(&line)?;
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
    fn torn_tail_is_repaired_at_open_before_next_append() {
        let dir = scratch("torn-tail");
        let file_path = dir.join(OUTBOX_FILE_NAME);
        let ob = EventOutbox::open(&dir).expect("the outbox opens");
        ob.append(EventClass::Mesh, &created("kept")).unwrap();
        ob.append(EventClass::Mesh, &created("torn")).unwrap();
        drop(ob);

        // Simulate a crash after part of the second entry's bytes landed:
        // the file no longer ends at a clean line boundary.
        let len = std::fs::metadata(&file_path).unwrap().len();
        OpenOptions::new()
            .write(true)
            .open(&file_path)
            .unwrap()
            .set_len(len - 10)
            .unwrap();

        let ob = EventOutbox::open(&dir).expect("the torn outbox reopens");
        let pending = ob.pending(EventClass::Mesh);
        assert_eq!(pending.len(), 1, "the torn entry is removed cleanly");
        assert!(matches!(
            &pending[0].pulse,
            ThalamicPulse::Created { id, .. } if id == "kept"
        ));

        let seq = ob
            .append(EventClass::Mesh, &created("after-repair"))
            .unwrap();
        assert_eq!(seq, 2, "the unreceipted torn sequence is reused");
        let text = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "the new append is its own line");
        for line in lines {
            serde_json::from_str::<OutboxEntry>(line)
                .expect("every remaining line is a parseable entry");
        }
        drop(ob);

        let ob = EventOutbox::open(&dir).expect("the repaired outbox reopens");
        assert_eq!(ob.pending(EventClass::Mesh).len(), 2);
    }

    #[test]
    fn invalid_utf8_line_is_skipped_at_open() {
        use std::io::Write as _;

        let dir = scratch("invalid-utf8");
        let file_path = dir.join(OUTBOX_FILE_NAME);
        let ob = EventOutbox::open(&dir).expect("the outbox opens");
        ob.append(EventClass::Mesh, &created("before")).unwrap();
        {
            let mut file = OpenOptions::new()
                .append(true)
                .open(&file_path)
                .unwrap();
            file.write_all(&[0xff, 0xfe, b'\n']).unwrap();
        }
        ob.append(EventClass::Mesh, &created("after")).unwrap();
        drop(ob);

        let ob = EventOutbox::open(&dir).expect("invalid UTF-8 does not fail open");
        let ids: Vec<String> = ob
            .pending(EventClass::Mesh)
            .into_iter()
            .map(|entry| match entry.pulse {
                ThalamicPulse::Created { id, .. } => id,
                other => panic!("expected a Created pulse, got {other:?}"),
            })
            .collect();
        assert_eq!(ids, vec!["before".to_string(), "after".to_string()]);
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

    /// A real file whose writes stop after a fixed byte budget and then
    /// fail, as a disk filling mid-write behaves: a prefix of the
    /// caller's bytes genuinely lands in the real file before the error
    /// surfaces. `len_at_failure` records the file's length at the
    /// moment of failure, so the test can prove the partial bytes landed
    /// — and were then rolled back.
    struct FailAfterFile {
        file: File,
        budget: usize,
        len_at_failure: Option<u64>,
    }

    impl Write for FailAfterFile {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.budget == 0 {
                self.len_at_failure = Some(self.file.metadata()?.len());
                return Err(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    "simulated mid-write failure (disk full)",
                ));
            }
            let accepted = buf.len().min(self.budget);
            let written = self.file.write(&buf[..accepted])?;
            self.budget -= written;
            Ok(written)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }

    impl AppendTarget for FailAfterFile {
        fn file_len(&self) -> std::io::Result<u64> {
            self.file.metadata().map(|metadata| metadata.len())
        }

        fn sync(&self) -> std::io::Result<()> {
            self.file.sync_data()
        }

        fn truncate_to(&self, len: u64) -> std::io::Result<()> {
            self.file.set_len(len)
        }
    }

    #[test]
    fn failed_append_rolls_back_a_partial_write() {
        let dir = scratch("append-rollback");
        let file_path = dir.join(OUTBOX_FILE_NAME);
        let ob = EventOutbox::open(&dir).expect("the outbox opens");
        ob.append(EventClass::Mesh, &created("kept")).unwrap();
        drop(ob);
        let prior = std::fs::read(&file_path).unwrap();

        // Drive the write step `append` itself uses, against a file that
        // fails after 10 bytes of the new line have genuinely landed.
        let entry = OutboxEntry {
            seq: 2,
            class: EventClass::Mesh,
            pulse: created("victim"),
        };
        let mut line = serde_json::to_vec(&entry).unwrap();
        line.push(b'\n');
        let mut failing = FailAfterFile {
            file: OpenOptions::new()
                .append(true)
                .open(&file_path)
                .unwrap(),
            budget: 10,
            len_at_failure: None,
        };
        let err = write_line_durably(&mut failing, &line).unwrap_err();
        assert!(
            matches!(err, OutboxError::Io(_)),
            "the write failure surfaces as an io error, got {err:?}"
        );
        assert_eq!(
            failing.len_at_failure,
            Some(prior.len() as u64 + 10),
            "the partial bytes genuinely landed before the failure"
        );
        drop(failing);
        assert_eq!(
            std::fs::read(&file_path).unwrap(),
            prior,
            "the rollback leaves the file byte-identical to its pre-append state"
        );

        // A subsequent successful append lands as its own parseable
        // entry — it does not glue onto the failed append's debris.
        let ob = EventOutbox::open(&dir).expect("the outbox reopens");
        let seq = ob
            .append(EventClass::Mesh, &created("after-rollback"))
            .unwrap();
        assert_eq!(seq, 2, "the failed append consumed no sequence");
        let text = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "the new append is its own line");
        for line in lines {
            serde_json::from_str::<OutboxEntry>(line)
                .expect("every remaining line is a parseable entry");
        }
        drop(ob);

        let ob = EventOutbox::open(&dir).expect("the outbox reopens again");
        assert_eq!(ob.pending(EventClass::Mesh).len(), 2);
    }
}
