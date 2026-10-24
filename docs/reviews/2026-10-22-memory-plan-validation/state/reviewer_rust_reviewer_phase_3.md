# Phase 3 — Independent Review: Rust Reviewer

**Agreement intensity:** 40% | **Reasoning:** checklist verification | **Mode:** PRECISE
**Score:** 4/10 | **Recommendation:** REJECT-AND-REWRITE

## Findings

### [P1] `RwLock` cannot safely enable concurrent inference
`PLAN.md:65`; `src/embed.rs:129,179-185`

`TextEmbedding::embed` takes `&mut self` (the existing code binds `let mut guard`), and
ORT documents concurrent `Session::run` as unsafe. A read guard will not compile; a write
guard serializes exactly like `Mutex`. Separate sessions permit concurrency only by
duplicating model resources (hundreds of MB).

**Fix:** keep the mutex; batch requests or use a bounded single-worker queue.
**Verify:** `rg -n 'pub fn embed|not thread safe' $HOME/.cargo/registry/src/*/fastembed-7.0.1/src/text_embedding/impl.rs $HOME/.cargo/registry/src/*/ort-*/src/session/mod.rs`

### [P1] Proposed tokio features omit required features
`PLAN.md:57`; `Cargo.toml:11`

`full` enables `fs, io-util, io-std, macros, net, parking_lot, process, rt,
rt-multi-thread, signal, sync, time`. The proposed set omits `fs`, `io-util`, `net`,
`signal`, used at `src/server.rs:5,1356,1364` and `src/daemon.rs:601,1044-1068`.
This can break compilation.

**Fix:** add the missing features; retain `process` only if needed.

### [P1] Cargo features do not set runtime thread limits
`src/main.rs:1733-1740`

Tokio's 512 blocking threads is a maximum; threads spawn on demand, idle threads exit, and
the queue has no backpressure. Feature trimming reduces compiled code, not runtime RSS.

**Fix:** `Builder::max_blocking_threads`, and bound submissions before `spawn_blocking`
with a semaphore or channel. The Laya semaphore does not bound FastEmbed calls.

### [P1] Production DB opens still use engine memory defaults
`src/store/ladybug.rs:605-607,707-709,730,751`

`buffer_pool_size = 0` auto-sizes to 80% of physical memory; the `max_db_size` sentinel
(`u32::MAX`) selects an 8 TB virtual region on 64-bit Linux, which is a virtual reservation,
not an RSS cap.

**Fix:** bound both explicitly in verified and unverified configs; preserve
`.throw_on_wal_replay_failure(true)` on the normal open and keep the unverified retry opt-in.

### [P2] 512-token cap truncates 4,000-character symbols
`src/embed.rs:124-126`; `src/parser/ingest.rs:27-42` — ~1,140 tokens needed.

### [P2] BlobStore is not a current daemon RSS reducer
`src/learning/blob_store.rs:10-31` — uses are tests; `record_blob_hash` is a no-op
(`recorder.rs:55-56,60-80`). If touched: replace `.expect()` at line 28 with lock-poison
error propagation and drop the per-insert `eprintln!`.

### [P2] "Remote default" needs configured endpoint and key variable
`src/embed.rs:48-62,300-323` — defaults set both to `None`, so `build_embedder` still
loads local ONNX.

### [P2] `text.to_owned()` is required by the current `&str` API
`src/embed.rs:174-188` — `spawn_blocking` requires `'static`. Avoid the copy only by
accepting owned input; batch owned `Vec<String>` and cap batch size.

### [P2] Panic surface in touched paths
`src/judgment/laya_onnx.rs:72-79` semaphore `.unwrap()`; `src/learning/blob_store.rs:28-29`
`.expect()`. Map both to `anyhow::Result`.

### [P2] `learning-laya` is an empty feature with no gate
`Cargo.toml:63-65`; `src/main.rs:13` — no `cfg(feature = "learning-laya")` anywhere, and
`mod learning` is unconditional. Gating reduces binary size, not daemon RSS.

## Strengths
1. Correctly identifies local ONNX as a plausible large RSS contributor.
2. Reuses existing `RemoteEmbedder` support.
3. Correctly moves CPU inference onto `spawn_blocking`.

## Weaknesses
1. Treats tokio feature trimming as a runtime-memory fix.
2. Proposes concurrent access to a non-concurrent ORT session.
3. Misses DB defaults and includes a BlobStore reducer with no production use.
