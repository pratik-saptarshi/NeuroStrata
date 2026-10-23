# Implementation Plan — Runtime Memory Remediation

**Branch:** `fix/runtime-memory-remediation`
**Source:** `review_panel_report.md` (verdict REJECT-AND-REWRITE, 3.2/10)
**Sequencing rule:** evidence strength, not claimed byte savings.

## Scope decisions

The review produced 13 action items. They are not all code, and two are "do not do this".
Each is settled here; nothing is left open.

| # | Item | Decision |
|---|---|---|
| 1 | Bound production buffer pool | **Implement** (T3) |
| 2 | Measure RSS baseline | **Not code** — operator procedure. Documented in report; requires a running daemon. |
| 3 | Ingest clears before embed / swallows failures | **Implement** (T1) |
| 4 | Remote 429/5xx retry never fires | **Implement** (T2) |
| 5 | Model/dimension guard + migration | **Implement guard** (T4). Full re-embed migration is not implemented — see "Open item" below. |
| 6 | Keep remote opt-in | **No code change** — current default is already local. Do not change it. |
| 7 | Tokio feature trimming | **Rejected.** The proposed subset does not compile (`fs`/`net`/`signal` in use) and features do not set thread counts. Keep `full`. |
| 8 | BlobStore bounding | **Rejected.** No production caller. |
| 9 | Lower MAX_EMBED_TOKENS | **Rejected.** 512 truncates ~55% of a maximal symbol. Keep 2048. |
| 10 | RwLock / batching | **Rejected.** `embed` needs `&mut self`; batching raises peak memory. |
| 11 | FETCH_MULTIPLIER | **Rejected.** Query cost, not RSS; reducing it can under-return domain matches. |
| 12 | Prune finished ingest jobs | **Rejected.** `src/ingest_jobs.rs:132-135` documents retention as deliberate: one receiver per *namespace*, bounded by namespace count, and required so a finished walk's status stays queryable. The panel's "unbounded" claim is wrong. |
| 13 | Lane numbering / split rollout | **Satisfied** — this plan splits work into independent commits. |

### Open item (must not roll forward silently)

**Full re-embed migration for embedder changes.** T4 blocks a width mismatch at open, which
prevents corruption; it does not move an existing 768-dim corpus to a 384-dim model. Doing
that needs: a staging namespace or shadow DB, a re-embed pass over every stored `memory_type`
that carries a real vector, and a swap. **Next action:** file as a task titled "Re-embed
migration for embedder width/identity change" with the acceptance test "open a 768-dim DB
under a 384-dim embedder, migrate, and assert search recall on a fixed query set". Owner: to
be assigned. Until it exists, changing the model on an existing DB is a manual
delete-and-reingest.

---

## T1 — Ingest must not report success after swallowing embed failures

**File:** `src/parser/ingest.rs`, `run_consumer` match block (~`:486-508`)

Current: embed error → `eprintln!`; upsert error → `eprintln!`; loop continues;
`ingest_directory` returns `Ok(())` because `consumer_result?` sees no error.

Steps:
1. Add a failing test: embedder that always errors, store from `for_testing`, assert
   `ingest_directory` returns `Err`. Requires a fixture directory with one `.rs` file.
   **Watch it fail** (it currently returns `Ok`).
2. Replace both `eprintln!` branches with `?` propagation, each mapped to an `anyhow` error
   that names the symbol, the file, and states the namespace was already cleared.
3. Re-run the test; watch it pass.
4. Run the existing ingest tests to confirm no regression.

**Acceptance:** a failing embed makes the command fail; no path logs and continues.

---

## T2 — Make the 429/5xx retry reachable

**File:** `src/embed.rs`, `try_embed` (~`:267-271`) and `embed_with_retry` (~`:232-252`)

Current: `try_embed` returns `anyhow!("API error {status}: {body}")`. `embed_with_retry`
tests `e.downcast_ref::<reqwest::StatusCode>()`, which can never match a formatted string.
The retry branch is dead.

Steps:
1. Add a failing test: build the error `try_embed` would return, assert
   `downcast_ref::<EmbedApiStatus>().is_some()`. **Watch it fail** against the current shape.
2. Add `struct EmbedApiStatus(reqwest::StatusCode)` implementing `Display` + `Error`; return
   `anyhow::Error::new(EmbedApiStatus(status))` from the non-success branch.
3. Re-run; watch it pass.
4. Keep the retry policy unchanged (2 attempts, 1s backoff, 429 + 5xx only).

**Acceptance:** the status is recoverable from the error chain.

---

## T3 — Bound the store's memory in production

**Files:** `src/config.rs`, `src/store/ladybug.rs`, `src/main.rs`

Current: `verifying_config()` = `SystemConfig::default().throw_on_wal_replay_failure(true)`.
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

Current: `CREATE NODE TABLE Memory (... embedding FLOAT[N] ...)` is sized from the embedder's
declared width. Opening an existing 768-dim DB under a 384-dim embedder does not resize
anything; queries and writes fail later, or worse, silently mix spaces. No identity is stored.

Steps:
1. Add a sidecar `<db_path>.embedding` recording `{"dimensions": N}`.
2. In `with_system_config`, before opening: if the sidecar exists and `N` differs, return an
   error naming both widths and the file, and instruct delete-and-reingest (there is no
   migration yet — see the open item).
3. After a successful open with no sidecar, write it. A write failure is a warning, not fatal.
4. Tests: mismatched sidecar is refused; matching sidecar opens; absent sidecar creates one.

**Acceptance:** a width mismatch fails loudly at open instead of corrupting later.

---

## Verification

- `cargo build` (release check of the touched paths)
- `cargo test parser::ingest` — T1
- `cargo test embed` — T2
- `cargo test config` — T3 resolver
- `cargo test store::ladybug` — T3/T4
- Full `cargo test` before finishing the branch.

## Commits (one per task, independent and revertible)

1. `fix(ingest): fail the walk when a symbol cannot be embedded or stored`
2. `fix(embed): carry the HTTP status so the 429/5xx retry can fire`
3. `fix(store): bound the buffer pool and database size in production`
4. `fix(store): refuse to open a database whose embedding width changed`
