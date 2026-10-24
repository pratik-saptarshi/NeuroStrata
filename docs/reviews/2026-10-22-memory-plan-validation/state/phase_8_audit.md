# Phase 8 — Completeness Audit

Single-agent scan for what the whole panel missed. Does not re-score the plan; only finds
overlooked items.

## Items the panel did not examine

### A. The plan has no success criterion or target
`PLAN.md` never states a target — no "reduce steady-state daemon RSS from X to Y", no
budget, no acceptance threshold. Without a target, no proposed change can be judged
sufficient or insufficient, and the verification section cannot pass or fail. Every
reviewer flagged the missing baseline; none flagged the missing target.

### B. No rollback or abort path
`PLAN.md:82-85` bundles three lanes. There is no statement of how to revert if retrieval
quality regresses, no feature flag, and no dual-run comparison. For a change that alters
vector semantics, rollback requires re-embedding, which is itself the missing migration.

### C. Operator surface for the DB pool does not exist
The highest-value lever identified in Phase 11 (`buffer_pool_size`) has no configuration
plumbing. `Config` has no pool field; `verifying_config()` hardcodes defaults. A fix
requires adding both the field and its propagation. The plan's "config risk" discussion
concerns only the embedder, never the store.

### D. `NEUROSTRATA_TEST_*` variables cannot help production
`src/store/ladybug.rs:692-699` reads `NEUROSTRATA_TEST_MAX_DB_SIZE` and
`NEUROSTRATA_TEST_BUFFER_POOL_SIZE`, but only inside `for_testing()`. An operator cannot
bound the production pool by environment today. This is the cheapest immediate mitigation
and needs a new production read path.

### E. ORT arena high-water retention is not discussed
Nothing in the plan addresses the fact that ORT sessions retain peak allocations. If the
observed growth is "RSS never comes back down after a large ingest", the fix is session
configuration or a process recycle policy, not fewer features or smaller models. This
distinction determines whether the plan is even aimed at the right failure mode.

### F. Long-lived daemon lifecycle
The daemon is long-lived and single-instance (`src/main.rs:1780-1816` spawn/wait logic).
The plan never considers whether a bounded memory ceiling with periodic restart is
acceptable operationally — a legitimate, low-risk lever that requires no code semantics
change.

### G. The `learning` module is unconditional and un-gated
`Cargo.toml:63-65` declares `learning-laya = []` but `src/main.rs:13` imports
`mod learning` unconditionally, so the feature flag is decorative. The plan did not
notice, and neither did it consider that the real cost here is binary size, not RSS.

## Audit verdict

The panel covered the plan's individual claims thoroughly. The systemic gap is that the
plan is **aimed at a hypothesis rather than a measurement**, and it lacks the two things
that would make it executable: a target and a rollback path. Items A–D are the material
omissions; E–F determine whether the plan is solving the right problem at all.
