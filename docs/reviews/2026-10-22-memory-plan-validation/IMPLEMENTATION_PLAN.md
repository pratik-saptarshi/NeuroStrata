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
The exact remote base for the current local work is `1abf0c3cd1313f2f272c378650c6e23da04b3bf0`; the working tree adds snapshot/restore, stage-provenance upgrade, cancellation, export-lock, restore-open-lock, and symlink-alias corrections. The full binary test suite passed 780/780 with a temporary `ort-load-dynamic` override and LadybugDB built from source. Export now refuses an active legacy stage that lacks durable provenance, and holds its lease while registering abandoned legacy stages at the snapshot cut. A concurrent pre-catalog writer that ignores the canonical database writer lock is outside the stable-cut guarantee; stop any such old process before exporting. A copied, open database plus active WAL is an unverified crash-state fixture, not a supported snapshot: strict restore rejected it on checksum verification and the regression asserts the source bytes, lease sidecars, target siblings, and restore interlock remain unchanged. Production `export_database` checkpoints before copying; its resulting snapshot has a separate restore-preservation test. No successful strict-mode restore from a known-valid nonempty WAL has been demonstrated. The default locked local build was blocked because the `ort-sys` prebuilt download from
`cdn.pyke.io` returned HTTP 403 through the environment proxy. That override is only a local
verification path; it is distinct from hosted default-feature CI. Workflow-dispatch run
`40036052762` for `bd5cdf75332badf8def9e19f5745316c2a82948e` passed all four jobs, including
the Ubuntu 765-test job. A newer run `40056891840` on the then-published head
`1abf0c3cd1313f2f272c378650c6e23da04b3bf0` passed Ubuntu tests, macOS and Ubuntu builds;
the Windows build was still in progress at the latest check. Neither run contains the
additional uncommitted snapshot, restore, locking, and model-validation changes documented
here; those require CI on the exact pushed candidate SHA.
An earlier run (`39911946378`) failed Ubuntu test compilation on the stacked async test
attributes in `src/learning/recorder.rs`; macOS, Ubuntu build, and Windows build jobs passed.
Runtime memory was not measured. A small synthetic fixture and a runner with common offline flags now exist at `tests/fixtures/runtime_validation/` and `scripts/runtime_validation.py`; the script itself does not restrict network access, so outbound access must be disabled externally before any real-inference run. The corpus is 1,977 bytes across four files and is not production-representative. Budgets are an 8 GiB per-process virtual-address limit, a 4 GiB observed RSS stop threshold, 1 GiB DB/WAL, 120 seconds per command, and 20 minutes total. The runner refuses symlinked model-cache entries, budgets two isolated copies, and records hashes for every supplied cache file. The pinned official Nomic model artifact is not present in the local cache, and the installed system ONNX Runtime 1.21 is incompatible with this build's ort API-24 requirement. The normal `ort-sys` prebuilt download returned proxy HTTP 403; no retry or alternate download route was used. Therefore real local inference and matched base/candidate measurements remain blocked; the prepared harness has only been exercised in fixture-generation mode. It records one CLI lifecycle per binary and labels idle RSS, steady-state inference, reader responsiveness, and warm-run variance as unmeasured. No empirical memory/throughput improvement is claimed. The last resource inventory showed a 16 GiB cgroup, 4 CPU quota, about 11 GiB free workspace space and 6.7 GiB free in `/tmp`.
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

---

## Verification status

| Gate | Status |
|---|---|
| Source changes | Implemented; changed Rust files formatted; `git diff --check` passes |
| Type-check | `cargo check --tests` passed with temporary `ort-load-dynamic` override and source-built LadybugDB |
| Binary test suite | 780/780 passed in the final run with temporary `ort-load-dynamic` and source-built LadybugDB, including the active-legacy-stage refusal guard. Other focused snapshot/restore, legacy lease migration, symlinked-parent alias, canceled export/checkpoint/init, and restore-open serialization regressions passed; strict restore rejects the unverified active-WAL fixture without changing source or destination. Actual local inference was not exercised |
| Default locked build | Blocked: `ort-sys` prebuilt download from `cdn.pyke.io` returned proxy HTTP 403; the installed system ONNX Runtime 1.21 is incompatible with the ort API-24 requirement. No download bypass was attempted |
| Exact-head PR CI | `bd5cdf75332badf8def9e19f5745316c2a82948e` passed workflow-dispatch run `40036052762` (all four jobs; 765 Ubuntu tests). Then-published head `1abf0c3cd1313f2f272c378650c6e23da04b3bf0` run `40056891840`: Ubuntu tests, macOS and Ubuntu builds passed; Windows build remained in progress at latest check. Current follow-up is uncommitted and needs hosted CI on its exact pushed SHA |
| Runtime harness | Synthetic fixture generation passed; 1,977 bytes across four files. Runner not executed because the complete compatible ONNX runtime/model cache is unavailable. Budgets: 8 GiB process virtual address space, observed 4 GiB RSS stop threshold, 1 GiB DB/WAL, 120 seconds per command, 20 minutes total. Network access must be disabled externally for any later inference run |
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
