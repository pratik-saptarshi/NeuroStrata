# Implementation Plan — Runtime Memory Remediation

**Branch:** `fix/runtime-memory-remediation`
**Source:** `review_panel_report.md` (verdict REJECT-AND-REWRITE, 3.2/10)
**Sequencing rule:** evidence strength, not claimed byte savings.

## Status record — 2026-10-24

Source changes implemented: recorder append/reopen coverage, staged ingest with atomic
publication and failure propagation, local HTTP retry tests, store-limit validation, and
embedding width/identity plus backup/restore marker handling. The follow-up also unified the
configured and runtime remote embedder identity, removed URL credentials from persisted
identity/error text, promoted staged records inside the database transaction without copying
vectors or payloads through application memory, hid/recovered abandoned staging generations,
and waits for definitive transaction resolution if publication exceeds its deadline. Stage
recovery now requires the exact generated UUID namespace plus its durable lease sidecar;
legacy `-ingest-stage-<UUID>` names and reserved-prefix lookalikes without a sidecar remain
visible and untouched. Malformed reserved identifiers do not block recovery of unrelated
stages, and new writes cannot claim the reserved namespace.
Source under test: commit `442a509374e2f45b3c46132f8e23b3206304aa96` (`test(runtime): tighten snapshot and harness regressions`), based on the prior remote head `9798742669e157b8c1cc0d0fa50c8c2a7d011f4e`. It adds stricter first-read hiding for proven pre-catalog stage rows, reopen/list/export and active-lease regressions, exact crash-state payload/vector/edge checks, awaited cancellation, same-namespace qualified CLI edge checks, and bounded benchmark-runner process cleanup. Focused results on the source commit: Python runner tests 4/4; legacy-stage reopen/list/export and strict active-WAL tests 1/1 each; crash-boundary publication (five killed subprocess phases, exact 130-row identity/content/vector/edge state) 1/1; cancellation resolution 1/1; same-namespace qualified rule relinking 1/1. The full binary suite on this exact source commit passed 782/782 (97.16 seconds) with a temporary `ort-load-dynamic` override and source-built LadybugDB.

The copied open database plus active WAL is an unverified crash-state capture, not a supported snapshot. The latest Ubuntu run `40058139853` on SHA `9798742669e157b8c1cc0d0fa50c8c2a7d011f4e` completed 779 tests and failed the test that incorrectly required this capture to fail checksum verification; its logs show no verification-failure diagnostic, and CI returned the recovered namespaces. The correction now distinguishes an intentionally byte-corrupted WAL (strict rejection required) from an unverified live-copy WAL: strict restore may reject it, in which case the test checks the exact verification error, untouched source bytes, and clean destination/interlock; if the engine accepts it, the test requires exact restored fact/rule/vector/edge/stage state and unchanged source. This is not evidence that a known-valid, engine-consistent nonempty WAL fixture has been restored. Production `export_database` checkpoints at its stable snapshot cut and has separate restore-preservation coverage. Active legacy leases are not migrated to durable provenance while the lease is held; pre-catalog stage rows with exact reserved UUID namespace, lease sidecar, and auto-ingestor-qualified IDs are hidden on the first read after reopen without being treated as deletion authority. Malformed names and user-owned lookalikes remain visible.

The default locked local build remains blocked because the `ort-sys` prebuilt download from `cdn.pyke.io` returned proxy HTTP 403; no retry or alternate download route was used. A temporary `ort-load-dynamic` override plus source-built LadybugDB is only a local alternative verification path, distinct from hosted default-feature CI. Earlier hosted run `40036052762` on `bd5cdf75332badf8def9e19f5745316c2a82948e` passed four jobs and 765 Ubuntu tests; run `40056891840` on `1abf0c3cd1313f2f272c378650c6e23da04b3bf0` was green on Ubuntu tests, macOS and Ubuntu builds. Run `40058139853` on SHA `9798742669e157b8c1cc0d0fa50c8c2a7d011f4e` reported 779 passed and one WAL-fixture failure. No run for the corrected source tree exists yet.

Runtime memory was not measured. The deterministic synthetic corpus is 1,977 bytes across four files and is not production-representative. The runner sets an 8 GiB per-child address-space and CPU-time limit, kills timed-out/output-overflowing process groups, caps captured output at 4 MiB per stream, and checks RSS (4 GiB) and DB/WAL size (1 GiB) after every CLI command as acceptance thresholds. RSS and DB/WAL thresholds are not live caps; real runs require an independently bounded memory and temporary-disk environment. Its 20-minute wall-time window is checked after synchronous setup copies and before/after CLI phases; an individual copy call cannot be interrupted. It uses the fixture project root as `cwd`, ingests `./src`, and expects the full `synthetic-validation::src/core.rs` ID and same-namespace rule edge. Runner tests passed 4/4 and fixture generation completed; no real CLI inference was run. The pinned official Nomic artifact is absent, installed system ONNX Runtime 1.21 is incompatible with the build's ort API-24 requirement, and the allowed prebuilt download returned proxy 403. Matched baseline/candidate memory and performance measurements, model inference, idle RSS, throughput/latency comparison, and database-growth measurements remain unexecuted; no empirical improvement is claimed.
The cooperating writer/snapshot sidecar lock resolves relative paths and symlinks to one canonical path. Hard-link aliases are not unified by that sidecar; the storage engine separately refuses concurrent opens of a database file.

## Scope decisions

The review produced 13 action items. They are not all code, and two are "do not do this".
Each is settled here; nothing is left open.

| # | Item | Decision |
|---|---|---|
| 1 | Bound production buffer pool | **Implement** (T3) |
| 2 | Measure RSS baseline | **Not code** — operator procedure. Documented in report; requires a running daemon. |
| 3 | Ingest clears before embed / swallows failures | **Implement** (T1) |
| 4 | Remote 429/5xx retry never fires | **Implement** (T2) |
| 5 | Model/dimension guard + migration | **Implement catalog width and model identity guards** (T4). In-place re-embedding is unavailable; use the safe separate-database lifecycle below. |
| 6 | Keep remote opt-in | **No code change** — current default is already local. Do not change it. |
| 7 | Tokio feature trimming | **Rejected.** The proposed subset does not compile (`fs`/`net`/`signal` in use) and features do not set thread counts. Keep `full`. |
| 8 | BlobStore bounding | **Rejected.** No production caller. |
| 9 | Lower MAX_EMBED_TOKENS | **Rejected.** 512 truncates ~55% of a maximal symbol. Keep 2048. |
| 10 | RwLock / batching | **Rejected.** `embed` needs `&mut self`; batching raises peak memory. |
| 11 | FETCH_MULTIPLIER | **Rejected.** Query cost, not RSS; reducing it can under-return domain matches. |
| 12 | Prune finished ingest jobs | **Rejected.** `src/ingest_jobs.rs:132-135` documents retention as deliberate: one receiver per *namespace*, bounded by namespace count, and required so a finished walk's status stays queryable. The panel's "unbounded" claim is wrong. |
| 13 | Lane numbering / split rollout | **Satisfied** — this plan splits work into independent commits. |

### Embedding change disposition

There is no in-place re-embedding migration. The store checks persisted catalog width and,
for databases with a v1 marker, model identity. On a mismatch, keep the original database and
marker intact. Reopen with the original embedder to retain access, or configure a separate
database path for the new embedder and ingest into that new database. Do not delete or
overwrite the original database. A legacy database with width but no model identity adopts the
currently configured model once with a warning; that historical identity cannot be proven.
New writes record identity, and later identity changes are rejected.

The supported operating behavior is to retain the original database with its original
embedder, or use a separate database path and ingest the corpus again with the new embedder.
In-place re-embedding is not supported or promised by this change; this is a resolved product
boundary, not a deferred implementation task. No migration task ID or owner acceptance is
claimed. The NeuroStrata task/memory CLI and MCP tracking tools were unavailable in this
environment, so no external task could be created or assigned.

---

## T1 — Ingest must not report success after swallowing embed failures

**File:** `src/parser/ingest.rs`, `run_consumer` match block (~`:486-508`)

Original defect: symbol embed/upsert errors propagated, structural-node upsert errors were
logged and ignored, and the live namespace was cleared before the walk finished.

Steps:
1. Keep regression tests for embedder and structural upsert failures, asserting the old graph
   remains available.
2. Stage the new graph under a private namespace; propagate structural and symbol failures.
   Publish rows and rebuild declared edges inside one LadybugDB transaction.
3. Re-run the test; watch it pass.
4. Run the existing ingest tests to confirm no regression.

**Acceptance:** producer, embedder, and structural/symbol store failures return errors; a
failed staged rebuild leaves the previously published ingested rows available. Publication
deletes old ingested rows, writes the new rows, and recreates declared edges in one database
transaction; readers see the prior committed generation until commit. A failed query rolls
back before the error returns, and closing an interrupted transaction relies on LadybugDB's
transaction recovery. Staged rows are promoted in bounded pages inside the transaction, with
vectors and content kept in the database; successful commit also disposes of the staging
generation. Abandoned generations with durable lease provenance are hidden and recovered on
a later ingest. Legacy stage-shaped names without provenance are preserved as user data
because the older naming scheme cannot prove ownership. If the write
deadline expires, the caller waits for the transaction's definitive outcome before returning.
Integration tests cover query failure rollback, a connection ending before commit, reader
visibility while a publication transaction is open, timeout past commit resolution, and
successful promotion with stage disposal.

---

## T2 — Make the 429/5xx retry reachable

**File:** `src/embed.rs`, `try_embed` (~`:267-271`) and `embed_with_retry` (~`:232-252`)

Original defect: `try_embed` returned a formatted status string that could not match the
retry classifier, so the retry branch was dead.

Steps:
1. Preserve the typed API status in the error chain and test actual request counts against a
   deterministic local HTTP server.
2. Add `struct EmbedApiStatus(reqwest::StatusCode)` implementing `Display` + `Error`; return
   `anyhow::Error::new(EmbedApiStatus(status))` from the non-success branch.
3. Re-run; watch it pass.
4. Keep the retry policy unchanged (2 attempts, 1s backoff, 429 + 5xx only).

**Acceptance:** the status is recoverable from the error chain.

---

## T3 — Bound the store's memory in production

**Files:** `src/config.rs`, `src/store/ladybug.rs`, `src/main.rs`

Original defect: `verifying_config()` used `SystemConfig::default().throw_on_wal_replay_failure(true)`.
`SystemConfig::default()` sets `buffer_pool_size: 0`, which lbug resolves to **80% of
physical RAM**, and `max_db_size: u32::MAX`, which resolves to an 8 TB virtual region. Tests
bound both; production does not.

Steps:
1. In `src/store/ladybug.rs`, add:

   ```rust
   pub struct StoreLimits { pub buffer_pool_bytes: u64, pub max_db_bytes: u64 }
   impl Default for StoreLimits { /* 256 MiB pool, 8 GiB max db */ }
   ```
2. Change `verifying_config()` / `unverified_config()` to take `StoreLimits` and set both
   fields via the existing builders. Preserve `throw_on_wal_replay_failure(true)` and the
   checksum/verification behaviour exactly.
3. Thread `StoreLimits` through `with_system_config` into the recovery fallbacks, and add
   `new_with_limits(path, dimensions, limits)`. Keep `new(path, dimensions)` delegating to
   `StoreLimits::default()` so the test suite and existing call sites are untouched.
4. In `src/config.rs`, add `StoreConfig { buffer_pool_bytes: Option<u64>, max_db_bytes:
   Option<u64> }` with `#[serde(default)]`, and `Config::store_limits()` returning defaults
   for `None`. Allow `NEUROSTRATA_BUFFER_POOL_BYTES` / `NEUROSTRATA_MAX_DB_BYTES` to override,
   so an operator can bound the pool without editing JSON.
5. In `src/main.rs`, switch the three production constructions (`:1881`, `:2437`, `:2659`) to
   `new_with_limits`.
6. Tests: `store_limits()` default resolution; env override wins; config value wins over
   default; env wins over config. (The lbug `SystemConfig` exposes builders but no getters, so
   the assertion lives on the pure resolver, not on the built struct.)

**Acceptance:** no production open path reaches `SystemConfig::default()` unmodified.

---

## T4 — Refuse to open a database whose embedding width differs

**File:** `src/store/ladybug.rs`

Original defect: `CREATE NODE TABLE Memory (... embedding FLOAT[N] ...)` is sized from the
embedder's declared width. A missing sidecar adopted configuration without checking catalog
width, and same-width models shared an unguarded vector space.

Steps:
1. Persist a v1 JSON sidecar with width and embedder identity.
2. When absent, inspect persisted `Memory.embedding` catalog width before adopting config.
3. Ignore an orphan sidecar when the DB path is absent; replace it after opening a fresh DB.
4. Copy the sidecar with backup and restore the paired sidecar with the database.
5. Guard known same-width model identity changes; warn when a legacy DB's historical model is
   unknown and adopt the configured identity once.

**Acceptance:** a width mismatch fails loudly at open instead of corrupting later.

### Follow-up safeguards from PR review comment 6380491674

The review found two source-traced gaps after the 3db8372 CI run. This correction
slice is being developed on top of that reviewed head; it is not yet committed or
hosted-CI verified.

**Database path aliases.** Resolve existing DB files through symlinks and relative
spellings to one canonical engine, writer-lock, marker, WAL, and snapshot path.
Check the canonical marker and the marker beside the supplied spelling before
opening/replaying the DB. Conflicting identities, widths, malformed entries, and
dangling marker symlinks fail closed without replacing marker evidence. A matching
historical alias marker remains in place while the canonical marker becomes the
authoritative copy. Preserve supplied spellings only for legacy ingest-lease sidecar
discovery. Bare relative DB creation resolves against the current directory.
Arbitrary hard-link aliases are not supported: the filesystem does not provide a
portable way to discover all names for one inode, so use the original DB path or a
separate copied database. No hard-link compatibility claim is made.

**Reserved ingest destinations.** A shared prefix validator rejects every
user-selected `__neurostrata_ingest_stage__...` destination, including malformed
names, in `IngestJobs::run` and `ingest_directory` before job registration, file
walking, staging, or embedding. `publish_ingested` repeats the guard before lease
lookup or transaction start. Direct-store and public-boundary regressions cover
active stage preservation, full row/vector/edge equality, lease ownership, and
subsequent successful publication.

**Current candidate evidence.** Changed Rust files pass `rustfmt --check` and
`git diff --check`; Python runtime-harness tests pass 4/4. The complete locked
serial Rust suite passes 796/796 on the current local working tree with the
temporary `fastembed/ort-load-dynamic` feature and lbug built from cached source
using `LBUG_BUILD_FROM_SOURCE=1` and the installed CMake executable at
`/tmp/neuro-python-cmake/cmake/data/bin/cmake`. Fourteen focused marker-alias,
lease-recovery, reserved-destination, and marker-write-failure regressions also
pass. Marker write/replace failure now propagates from construction, and the
failure regression verifies prior evidence and temporary-file cleanup. This is local
alternative-build evidence, not default-feature verification or hosted CI. An
initial lbug prebuilt fetch returned proxy HTTP 403 (one alternate artifact URL
returned 404); no retry was made. The CMake distribution was already present in
`/tmp`, but its wrapper referenced an unavailable Python interpreter, so the
source build used the executable directly. No Cargo manifest or lockfile change
was made. The tested source was pushed as commit
`6e652abb1dd17c7cc35ec44ee13bb8749bbf6aab` to the fork branch; PR #1 remains
open and draft. Exact-head workflow and combined-status queries returned no
run/status, and no workflow-dispatch action is available here, so default hosted
CI is pending rather than passed. Task and memory MCP tools and `neurostrata-mcp`
are unavailable here, so no task was claimed or invented. Earlier CI on
`3db8372` does not validate these edits.

---

## Verification status

| Gate | Status |
|---|---|
| Source changes | Implemented; changed Rust files formatted; `git diff --check` passes |
| Type-check | Full locked Cargo test build compiled the current test binary with temporary `ort-load-dynamic` override and source-built LadybugDB |
| Binary test suite | 796/796 passed in 93.66 seconds on the current local working tree; 14 focused safeguard regressions passed. This is alternative local verification, not default-feature verification. Actual local inference was not exercised |
| Default locked build | Blocked: `ort-sys` prebuilt download from `cdn.pyke.io` returned proxy HTTP 403; the installed system ONNX Runtime 1.21 is incompatible with the ort API-24 requirement. No download bypass was attempted |
| Exact-head PR CI | Previously published head `3db83724e6e2261d329b261f3e9e455079f7a6d6` passed run `40060049864` (all four jobs; 782 Ubuntu tests). Current pushed head `6e652abb1dd17c7cc35ec44ee13bb8749bbf6aab` has no hosted run/status yet; the available GitHub tools expose no workflow-dispatch action |
| Runtime harness | Synthetic fixture generation passed; 1,977 bytes across four files. Runner process-group timeout, output-cap, fast-exit output-cap, and model-cache symlink tests passed 4/4. No real CLI inference run. It uses an 8 GiB per-process address-space limit and 120s per-command timeouts. The 20m wall-time window is an acceptance deadline checked after synchronous setup copies and before/after CLI phases; a single copy call cannot be interrupted. RSS 4 GiB and DB/WAL 1 GiB are post-command acceptance thresholds, not live enforcement; independent memory and temporary-disk quotas are required around actual runs |
| Runtime memory comparison | Not executed: no matched base/candidate runs, no real inference, no raw measurement logs, and no empirical improvement claim |
| Migration follow-up | No in-place migrator is supported by this change; separate database plus fresh ingest is the declared lifecycle. No task ID or owner acceptance is claimed; task/memory tracking CLI/MCP was unavailable |
| NeuroStrata task/memory gate | Unavailable: task/memory MCP tools and the `neurostrata-mcp` CLI are not installed |

Default-feature verification commands for this local environment remain blocked by the ONNX
prebuilt download:

- `cargo build --locked --release`
- `cargo test --locked parser::ingest`
- `cargo test --locked embed`
- `cargo test --locked config`
- `cargo test --locked store::ladybug`
- `cargo test --locked -- --test-threads=1`

## Change organization

The approved corrections share failure-handling and persistence boundaries. Commit them as
one reviewable PR correction after the required tests and exact-head CI pass.
