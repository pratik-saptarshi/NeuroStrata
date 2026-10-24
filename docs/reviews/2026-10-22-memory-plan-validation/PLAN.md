# Plan Under Review: Reduce Runtime Memory Utilization

**Repository:** /Volumes/dev/Git-SCM/NeuroStrata (branch `main`, commit `219e47d`)
**Date:** 2026-10-22
**Author:** primary agent (prior analysis session)

## Problem statement

`neurostrata-mcp` resident set size grows large while the model/embedder runs.
Goal: reduce runtime memory utilization of the running model.

## Claimed diagnosis

RSS is dominated by the `fastembed` ONNX embedder, not by Laya (which is stubbed).

Supporting claims from source:

- `src/embed.rs:128-169` — `FastEmbedder { model: Arc<Mutex<TextEmbedding>> }`,
  models `NomicEmbedTextV15` / `BGEBaseENV15`, 768 dims,
  `MAX_EMBED_TOKENS = 2048`, cache `~/.cache/neuro/models/fastembed`.
  A source comment references a "523 MB model cache".
- `src/learning/runtime.rs`, `src/learning/model_bundle.rs`,
  `src/learning/trainer.rs:75-80`, `src/judgment/laya_onnx.rs:67-79` — Phase-0
  stubs / no-op; no model weights held.
- `Cargo.toml:11` — `tokio = { features = ["full"] }`.
- `Cargo.toml:17-20` — `fastembed` with `ort-download-binaries-rustls-tls`,
  `hf-hub-rustls-tls`.

## Proposed reducers (the plan to validate)

### 1. Eliminate or shrink the embedder — "100s of MB"

- `src/embed.rs:300` — `RemoteEmbedder` already exists; use it
  (`base_url` / `api_key_env`) so no local ONNX is loaded at all.
- If local is required: switch to a smaller model (e.g. `AllMiniLML6V2`, 384-dim,
  ~80 MB vs 500+ MB) and lower `MAX_EMBED_TOKENS: 2048 -> 512` to match
  `MAX_SYMBOL_CHARS = 4000` in `src/parser/ingest.rs:31`.
- Ensure a single `Arc` instance; do not call `build_embedder()` per request.

### 2. Fix unbounded in-memory duplicate that grows with ingest

- `src/learning/blob_store.rs:11,28` — `Mutex<HashMap<PayloadHash, Vec<u8>>>`
  stores `content.as_bytes().to_vec()` forever, duplicating DB content.
  Replace with a bounded LRU (e.g. 128 entries) or store hashes only.

### 3. Stop 10x over-fetch in vector search

- `src/store/ladybug.rs:52-53,86-93` — `FETCH_MULTIPLIER = 10`; query builds
  `vec_str` via `format!` (768 floats -> ~10 KB string per query); holds
  `all_primaries` + `filtered_primaries` Vecs. Push `LIMIT` down, stream rows,
  avoid intermediate `Vec<String>` clones at `:157,176,207`.

### 4. Trim `tokio full` and ORT threads

- `Cargo.toml:11` — `full` pulls extra features and a default multi-thread
  runtime with a blocking pool (claimed 512 threads x 2 MB stack = worst-case RSS).
  Use `features = ["rt-multi-thread","macros","sync","time","process"]`.
- ORT: set `intra_op_num_threads = 1`; keep the bounded `inference_semaphore`
  in `src/judgment/laya_onnx.rs:24`.

### 5. Batch and de-serialize the embed path

- `src/embed.rs:178-186` — `text.to_owned()` + `vec![&*text]` per call, and a
  `Mutex` serializes all embeds, so queues build during ingest. Batch symbols
  (32-64) per `embed()` call; use `RwLock` or a single-writer channel.

### 6. Minor

- `src/secrets.rs:146` `MAX_SCAN_BYTES = 256k`; `src/buffer.rs:15` 500 KB
  rollover — described as fine.
- `src/ingest_jobs.rs:135` — `Mutex<HashMap<String, watch::Receiver>>`; prune
  finished jobs.

## Proposed verification before changing

```bash
cargo bloat --release --crates | head -30
ps -o rss,vsz -p $(pgrep neurostrata-mcp)
NEUROSTRATA_MODEL=BGEBaseENV15 cargo run --release -- ingest --help
```

## Proposed sequencing

Implement lanes 1 + 2 + 3 (remote-embedder default, bounded BlobStore,
trimmed tokio features) in one change.
