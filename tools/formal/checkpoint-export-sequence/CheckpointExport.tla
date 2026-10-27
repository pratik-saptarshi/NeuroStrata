------------------------- MODULE CheckpointExport -------------------------
(***************************************************************************)
(* Faithful model of sequence S from the Task A1 coverage-gap record      *)
(* (sync SHA fe2d41b3dc1a1b5f340938bf8ffbee0fd5daf403):                   *)
(*                                                                        *)
(*   (1) checkpoint() C starts, takes the snapshot lock, its detached     *)
(*       blocking closure takes the cross-process writer lock (flock)     *)
(*       and takes the Dirty flag for a previously committed seed write,  *)
(*       pausing mid-flush;                                               *)
(*   (2) write W is dispatched, holds the write turn, and blocks          *)
(*       acquiring the flock, its MarkDirtyOnDrop not yet armed;          *)
(*   (3) C's async caller is cancelled while the closure runs detached,   *)
(*       still holding both locks;                                        *)
(*   (4) export_database() E is called and blocks on the snapshot lock;   *)
(*   (5) C's flush is released and completes.                             *)
(*                                                                        *)
(* Properties (A1 section 3): after E completes,                          *)
(*   (a) E's snapshot contains W's row in full;                           *)
(*   (b) the live store still reports dirty (E never touches the flag,    *)
(*       and W marked after C's take);                                    *)
(*   (c) a following checkpoint() clears the flag.                        *)
(*                                                                        *)
(* Every variable and action names its code anchor in                   *)
(* src/store/ladybug.rs at fe2d41b. The model covers exactly S: one       *)
(* checkpoint, one write, one export, one follow-up checkpoint. Errors,   *)
(* flush failure, and process crashes are out of scope (see README.md).   *)
(***************************************************************************)

VARIABLES
    snapHolder,   \* snapshot_lock owner: "none" | "C" | "E"
                  \* anchor: LadybugStore.snapshot_lock; C acquires at
                  \* checkpoint() :4671 (lock_owned before spawn_blocking),
                  \* E acquires at export_database() :4578
    turnHolder,   \* write_lock (write turn) owner: "none" | "W" | "E"
                  \* anchor: W's caller holds the OwnedMutexGuard passed
                  \* into write_owning_turn (:2733); E acquires at :4579
    flockHolder,  \* cross-process writer lock owner: "none" | "C" | "W" | "E"
                  \* anchor: acquire_database_writer_lock -- C's closure,
                  \* checkpoint() :4678 region; W's closure :2756 region;
                  \* E's closure :4590 region
    dirty,        \* the Dirty flag (AtomicBool) :1718-1748; mark :1730,
                  \* take :1736 (swap to FALSE, AcqRel)
    cState,       \* checkpoint C: "notStarted" | "holding" | "flushing" | "done"
    cCancelled,   \* C's async caller cancelled while closure detached
                  \* anchor: aborting the checkpoint() future does not stop
                  \* its spawn_blocking closure (checkpoint() :4676 onward)
    wState,       \* write W: "notDispatched" | "queued" | "inClosure" |
                  \* "committed" | "marked" | "done"
    wCommitted,   \* W's statement committed (row durable in the WAL)
                  \* anchor: f(&conn) returning inside write_owning_turn's
                  \* closure (:2758-2760 region)
    eState,       \* export E: "notCalled" | "waitingSnap" | "waitingTurn" |
                  \* "hasTurn" | "inClosure" | "copied" | "done"
    eFlushedW,    \* at E's internal CHECKPOINT (:4610): whether W's row
                  \* was flushed from the WAL into the data file E copies
    snapshotHasW, \* the copied snapshot contains W's row
                  \* anchor: file copy in export_database() (:4618 onward)
    snapshotDone, \* E's copy + manifest completed (snapshot finalized)
    c2Done        \* follow-up checkpoint() ran after S (property (c))

vars == <<snapHolder, turnHolder, flockHolder, dirty, cState, cCancelled,
          wState, wCommitted, eState, eFlushedW, snapshotHasW, snapshotDone,
          c2Done>>

Init ==
    /\ snapHolder = "none"
    /\ turnHolder = "none"
    /\ flockHolder = "none"
    /\ dirty = TRUE        \* seed write committed, not yet checkpointed (A1 S step 1)
    /\ cState = "notStarted"
    /\ cCancelled = FALSE
    /\ wState = "notDispatched"
    /\ wCommitted = FALSE
    /\ eState = "notCalled"
    /\ eFlushedW = FALSE
    /\ snapshotHasW = FALSE
    /\ snapshotDone = FALSE
    /\ c2Done = FALSE

(* --- Checkpoint C ------------------------------------------------------- *)

CStart ==        \* checkpoint() acquires the snapshot lock, async side (:4671)
    /\ cState = "notStarted"
    /\ snapHolder = "none"
    /\ snapHolder' = "C"
    /\ cState' = "holding"
    /\ UNCHANGED <<turnHolder, flockHolder, dirty, cCancelled, wState,
                   wCommitted, eState, eFlushedW, snapshotHasW, snapshotDone,
                   c2Done>>

CBeginFlush ==   \* closure acquires flock (:4678), takes the flag (:4681),
                 \* then pauses mid-flush (test barrier point :4684)
    /\ cState = "holding"
    /\ flockHolder = "none"
    /\ flockHolder' = "C"
    /\ dirty' = FALSE
    /\ cState' = "flushing"
    /\ UNCHANGED <<snapHolder, turnHolder, cCancelled, wState, wCommitted,
                   eState, eFlushedW, snapshotHasW, snapshotDone, c2Done>>

CCancel ==       \* step (3): caller cancelled; the detached closure and its
                 \* guards are unaffected (checkpoint() :4676 onward)
    /\ cState = "flushing"
    /\ ~cCancelled
    /\ cCancelled' = TRUE
    /\ UNCHANGED <<snapHolder, turnHolder, flockHolder, dirty, cState, wState,
                   wCommitted, eState, eFlushedW, snapshotHasW, snapshotDone,
                   c2Done>>

CFinishFlush ==  \* step (5): CHECKPOINT completes; closure returns and its
                 \* guards drop: flock then snapshot lock released
    /\ cState = "flushing"
    /\ flockHolder' = "none"
    /\ snapHolder' = "none"
    /\ cState' = "done"
    /\ UNCHANGED <<turnHolder, dirty, cCancelled, wState, wCommitted, eState,
                   eFlushedW, snapshotHasW, snapshotDone, c2Done>>

(* --- Write W ------------------------------------------------------------ *)

WDispatch ==     \* step (2): W dispatched while C is mid-flush; its caller
                 \* takes the write turn, its blocking thread then blocks
                 \* on the flock (test_write_lock_attempt point, :2752 region)
    /\ wState = "notDispatched"
    /\ cState = "flushing"
    /\ turnHolder = "none"
    /\ turnHolder' = "W"
    /\ wState' = "queued"
    /\ UNCHANGED <<snapHolder, flockHolder, dirty, cState, cCancelled,
                   wCommitted, eState, eFlushedW, snapshotHasW, snapshotDone,
                   c2Done>>

WAcquireFlock == \* closure acquires the flock (:2756 region) and arms
                 \* MarkDirtyOnDrop (:2758) -- arming is after flock
                 \* acquisition, so a queued write has NOT armed yet
    /\ wState = "queued"
    /\ flockHolder = "none"
    /\ flockHolder' = "W"
    /\ wState' = "inClosure"
    /\ UNCHANGED <<snapHolder, turnHolder, dirty, cState, cCancelled,
                   wCommitted, eState, eFlushedW, snapshotHasW, snapshotDone,
                   c2Done>>

WCommit ==       \* f(&conn) executes and commits; the row is durable in the WAL
    /\ wState = "inClosure"
    /\ wCommitted' = TRUE
    /\ wState' = "committed"
    /\ UNCHANGED <<snapHolder, turnHolder, flockHolder, dirty, cState,
                   cCancelled, eState, eFlushedW, snapshotHasW, snapshotDone,
                   c2Done>>

WMarkDrop ==     \* MarkDirtyOnDrop drops. Guard drop order in the closure:
                 \* _mark_dirty is declared after _cross_process_writer, so it
                 \* drops FIRST -- the mark lands before the flock is
                 \* released (:2756-2758 declaration order; Drop impl :1744)
    /\ wState = "committed"
    /\ dirty' = TRUE
    /\ wState' = "marked"
    /\ UNCHANGED <<snapHolder, turnHolder, flockHolder, cState, cCancelled,
                   wCommitted, eState, eFlushedW, snapshotHasW, snapshotDone,
                   c2Done>>

WRelease ==      \* flock released, then the write turn (caller's guard)
    /\ wState = "marked"
    /\ flockHolder' = "none"
    /\ turnHolder' = "none"
    /\ wState' = "done"
    /\ UNCHANGED <<snapHolder, dirty, cState, cCancelled, wCommitted, eState,
                   eFlushedW, snapshotHasW, snapshotDone, c2Done>>

(* --- Export E ----------------------------------------------------------- *)

ECall ==         \* step (4): E called while C's detached flush still holds
                 \* the snapshot lock, and after C's caller was cancelled.
                 \* The cState/snapHolder conjuncts are what make this S:
                 \* E is called BEFORE C's flush completes, so E then
                 \* blocks in EAcquireSnap until CFinishFlush releases
                 \* the snapshot lock -- E never slips in after the fact.
    /\ eState = "notCalled"
    /\ cCancelled
    /\ cState = "flushing"
    /\ snapHolder = "C"
    /\ wState /= "notDispatched"
    /\ eState' = "waitingSnap"
    /\ UNCHANGED <<snapHolder, turnHolder, flockHolder, dirty, cState,
                   cCancelled, wState, wCommitted, eFlushedW, snapshotHasW,
                   snapshotDone, c2Done>>

EAcquireSnap ==  \* export_database() acquires the snapshot lock (:4578)
    /\ eState = "waitingSnap"
    /\ snapHolder = "none"
    /\ snapHolder' = "E"
    /\ eState' = "waitingTurn"
    /\ UNCHANGED <<turnHolder, flockHolder, dirty, cState, cCancelled, wState,
                   wCommitted, eFlushedW, snapshotHasW, snapshotDone, c2Done>>

EAcquireTurn ==  \* then the write turn (:4579) -- the decisive resource:
                 \* W holds it until W is done, so E cannot overtake W
    /\ eState = "waitingTurn"
    /\ turnHolder = "none"
    /\ turnHolder' = "E"
    /\ eState' = "hasTurn"
    /\ UNCHANGED <<snapHolder, flockHolder, dirty, cState, cCancelled, wState,
                   wCommitted, eFlushedW, snapshotHasW, snapshotDone, c2Done>>

EBeginClosure == \* closure acquires the flock (:4590 region) and runs its
                 \* own internal CHECKPOINT (:4610), flushing the WAL --
                 \* including W's committed row -- into the data file.
                 \* E never touches the Dirty flag (no anchor exists).
    /\ eState = "hasTurn"
    /\ flockHolder = "none"
    /\ flockHolder' = "E"
    /\ eFlushedW' = wCommitted
    /\ eState' = "inClosure"
    /\ UNCHANGED <<snapHolder, turnHolder, dirty, cState, cCancelled, wState,
                   wCommitted, snapshotHasW, snapshotDone, c2Done>>

ECopy ==         \* files copied after the internal checkpoint (:4618 onward)
    /\ eState = "inClosure"
    /\ snapshotHasW' = eFlushedW
    /\ snapshotDone' = TRUE
    /\ eState' = "copied"
    /\ UNCHANGED <<snapHolder, turnHolder, flockHolder, dirty, cState,
                   cCancelled, wState, wCommitted, eFlushedW, c2Done>>

EFinish ==       \* closure returns; flock, write turn, snapshot lock released
    /\ eState = "copied"
    /\ flockHolder' = "none"
    /\ turnHolder' = "none"
    /\ snapHolder' = "none"
    /\ eState' = "done"
    /\ UNCHANGED <<dirty, cState, cCancelled, wState, wCommitted, eFlushedW,
                   snapshotHasW, snapshotDone, c2Done>>

(* --- Follow-up checkpoint (property (c)) -------------------------------- *)

C2Checkpoint ==  \* a following checkpoint() takes the flag (:4681), which
                 \* clears it, and flushes
    /\ cState = "done"
    /\ wState = "done"
    /\ eState = "done"
    /\ ~c2Done
    /\ dirty' = FALSE
    /\ c2Done' = TRUE
    /\ UNCHANGED <<snapHolder, turnHolder, flockHolder, cState, cCancelled,
                   wState, wCommitted, eState, eFlushedW, snapshotHasW,
                   snapshotDone>>

Terminal ==
    /\ cState = "done" /\ wState = "done" /\ eState = "done" /\ c2Done

EpisodeEnd ==
    \* The full sequence S completed, OR the episode ended before S
    \* materialised: C finished (with or without cancellation, with or
    \* without W ever being dispatched) and no export was ever called.
    \* Those prefixes are not deadlocks -- S simply did not occur -- but
    \* once E is called, the episode must run to Terminal.
    \/ Terminal
    \/ (cState = "done" /\ eState = "notCalled")

Stutter == EpisodeEnd /\ UNCHANGED vars

Next ==
    \/ CStart \/ CBeginFlush \/ CCancel \/ CFinishFlush
    \/ WDispatch \/ WAcquireFlock \/ WCommit \/ WMarkDrop \/ WRelease
    \/ ECall \/ EAcquireSnap \/ EAcquireTurn \/ EBeginClosure \/ ECopy
    \/ EFinish \/ C2Checkpoint \/ Stutter

Spec == Init /\ [][Next]_vars

(* --- Properties (A1 section 3) ------------------------------------------ *)

InvA ==          \* (a) a finalized snapshot contains W's row in full
    snapshotDone => snapshotHasW

InvB ==          \* (b) after E completes and before any follow-up
                 \* checkpoint, the live store still reports dirty
    (eState = "done" /\ ~c2Done) => dirty

InvC ==          \* (c) once the follow-up checkpoint ran, the flag is clear
    c2Done => ~dirty

Inv == InvA /\ InvB /\ InvC

=============================================================================
