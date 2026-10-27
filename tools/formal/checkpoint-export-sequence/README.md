# CheckpointExport — formal model of the cancelled-checkpoint / queued-write / export sequence (sync SHA: fe2d41b3dc1a1b5f340938bf8ffbee0fd5daf403)

Task A2 of the pstack-NeuroStrata adoption plan. Model of exactly sequence S
from the Task A1 coverage-gap record (`coverage-gap.md` §3); no broader system.

## The property, in plain words

A checkpoint is running and has already taken the "there are unflushed
writes" flag for an earlier write. While its flush is paused, a new write
queues behind it. The checkpoint's caller is cancelled — but the flush itself
runs detached and still holds its locks. An export (the snapshot/backup cut)
is then requested and waits. When everything finishes:

- **(a)** the export's snapshot contains the queued write's row in full;
- **(b)** the live store still reports itself *dirty* — the export does not
  clear the flag, and the queued write marked it again after the checkpoint
  took it;
- **(c)** a following checkpoint clears the flag.

If (a) failed, a committed write would be silently absent from a snapshot
every actor reported as successful. If (b) failed, the store would believe
itself flushed when it is not. Either is durable-memory loss with no signal.

## Files

- `CheckpointExport.tla` — the faithful model (TLA+).
- `CheckpointExport.cfg` — TLC config: `Spec`, invariants `InvA`/`InvB`/`InvC`
  (the three properties above).
- `run.sh` — the one rerun command: `./run.sh`. Fails closed with
  `ERROR(tla-pin-missing)` / `ERROR(tla-pin-mismatch)` if the pinned jar is
  absent or its checksum differs.
- `broken-variant/` — spike scaffolding for plan Step 3 preparation: the same
  model with the dirty re-mark removed (models the real-code break on branch
  `spike/a2-break-dirty-remark`). Expected to violate `InvB`.

## Tool pin

- TLA+ tools **v1.8.0** ("The Clarke release"),
  https://github.com/tlaplus/tlaplus/releases/tag/v1.8.0 — asset
  `tla2tools.jar`, installed at `~/opt/tla-v1.8.0/tla2tools.jar`
  (never vendored into this repo).
- SHA-256 of the installed jar, computed at provisioning (2026-10-27):
  `78f7e2c6bf9a8b65eda665a23e94431255587e0b43a834a4e2af1723562d9b4a`
- Cross-check: the jar's SHA-1 is
  `9116776be10bca704fbdaaf178f4eb9365892e23`, exactly the SHA-1 published on
  the official v1.8.0 release page (the census recorded that the release
  publishes SHA-1 only; the SHA-256 above is ours, computed from the
  downloaded jar whose SHA-1 matches the official value).
- Runtime: Temurin JDK 25.0.4.1 (`~/opt/jdk-25.0.4.1+1/bin/java`);
  `java -cp tla2tools.jar tlc2.TLC` reports
  `TLC2 Version 2026.10.26.231022 (rev: fad899c)`.

## Bounds and what they exclude

The model is deliberately exactly S, nothing more:

- One checkpoint C, one write W, one export E, one follow-up checkpoint.
  The seed write is abstracted to the initial state `dirty = TRUE`.
- Dispatch order is fixed by S: W is dispatched only while C is mid-flush;
  E is called only after W is dispatched and C's caller is cancelled, and
  while C is still mid-flush holding the snapshot lock — `ECall` requires
  `cState = "flushing" /\ snapHolder = "C"`, so E then blocks on the
  snapshot lock until C's flush completes. All other interleavings of the
  modelled steps are explored by TLC.
- Locks are modelled as perfect mutual exclusion (single holder variables).
  The OS flock's behaviour, the `try_lock` probe used by tests, and lock
  poisoning are not modelled.
- Cancellation is modelled as caller-only: aborting the async future never
  stops the detached `spawn_blocking` closure or its guards — which is what
  the code does (guards are moved into the closure).
- The engine's `CHECKPOINT` is abstracted to "committed WAL rows are
  flushed to the data file." Engine internals (lbug) are not modelled.
- Excluded: flush/write **errors** (including checkpoint's re-mark on flush
  failure, `ladybug.rs:4693` region), process crashes, init's own dirty
  mark (`:3578`), multiple queued writes, and any second export.

## Code anchors (all `src/store/ladybug.rs` at fe2d41b)

| Model element | Anchor |
|---|---|
| `Dirty` flag, `mark` / `take` semantics | :1718–1748 |
| `MarkDirtyOnDrop`, its `Drop` impl | :1740–1748 |
| C: snapshot lock before `spawn_blocking` | `checkpoint()` :4671 |
| C: flock, then `dirty.take()`, then barrier point | :4678, :4681, :4684 |
| C: re-mark on flush error (excluded from S) | :4693 region |
| W: write turn held by caller into `write_owning_turn` | :2733 |
| W: flock acquired before mark armed; guard drop order (mark drops first — declared last) | :2756–2758 |
| E: snapshot lock, then write turn | `export_database()` :4578–4579 |
| E: flock inside closure | :4590 region |
| E: internal `CHECKPOINT`, then file copy | :4610, :4618 onward |
| E: never touches the `Dirty` flag | (absence — no anchor exists) |

## Maintenance

If any anchored code changes, this model describes a system that no longer
exists: rerun `./run.sh` only after re-checking each anchor against the new
code, and update the sync SHA in this file's title line. A PASS at an old
SHA is a statement about that SHA only.
