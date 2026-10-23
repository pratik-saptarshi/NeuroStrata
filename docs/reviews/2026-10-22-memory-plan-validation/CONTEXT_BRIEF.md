# Context Brief — Memory Plan Review

**Repo:** /Volumes/dev/Git-SCM/NeuroStrata
**Branch:** main @ 219e47d
**Work under review:** `docs/reviews/2026-10-22-memory-plan-validation/PLAN.md`
**Content type:** Mixed (plan prose + code claims) → **Review mode: PRECISE for code claims, EXHAUSTIVE for plan prose**

## Goal of the plan

Reduce runtime memory utilization (RSS) of the running model in `neurostrata-mcp`.

---

## Verified repository facts (established by grep/read BEFORE the panel; re-verify independently if you doubt them)

### Architecture
- This is a Rust MCP server. Single binary `neurostrata-mcp` (`Cargo.toml:2-8`).
- Two process roles:
  - **Daemon** holds the DB + embedder (`src/main.rs:1880` `build_embedder()`, `:1881` `LadybugStore::new(...)`).
  - **MCP stdio proxy** holds NO engine and NO embedder — it forwards JSON-RPC over HTTP to the daemon (`src/main.rs:1780-1782`, `src/server.rs:855+`). Comment at `src/main.rs:729`: "The proxy holds no engine handle -- that is the whole point."
- `build_embedder()` has exactly 3 production call sites, all process-startup paths (`src/main.rs:1880`, `:2436`, `:2658`) — never per-request.

### Embedder
- `src/embed.rs:128-169` `FastEmbedder { model: Arc<Mutex<TextEmbedding>> }`.
- Config: `~/.config/neurostrata/embedders.json`; defaults are `NomicEmbedTextV15` (768 dim) and `BGEBaseENV15` (768 dim), both with `base_url: None` → local ONNX (`src/embed.rs:48-70`).
- `NEUROSTRATA_MODEL` env selects among acceptable models (`src/embed.rs:105-113`).
- `MAX_EMBED_TOKENS = 2048` (`src/embed.rs:126`).
- Source comment at `src/embed.rs:124-126`: "Roughly covers MAX_SYMBOL_CHARS of source at ~3.5 characters per token, with headroom for the header lines."
- `MAX_SYMBOL_CHARS = 4000` (`src/parser/ingest.rs:31`); `truncate_for_embedding()` at `:33-42`.
- `embed()` clones text (`text.to_owned()`), calls `spawn_blocking`, takes a `Mutex` around the model, embeds a 1-element batch (`src/embed.rs:174-190`).
- `RemoteEmbedder` exists (`src/embed.rs:201-298`), selected only when BOTH `base_url` and `api_key_env` are set (`src/embed.rs:303`). Its `api_model` defaults to `text-embedding-3-small` (`src/embed.rs:307-310`).
- fastembed 7.0.1 (`Cargo.lock:806-807`). Its `TextInitOptions` supports `.with_max_length()`, `.with_intra_threads()`, `.with_session_config(key, value)`, `.with_cache_dir()`, `.with_show_download_progress()`.
- `EmbeddingModel` variants include `AllMiniLML6V2` (384 dim), `AllMiniLML6V2Q` (384, dynamic quant), `BGESmallENV15Q` (384, static quant), `NomicEmbedTextV15Q` (768, dynamic quant).

### Vector store
- `LadybugStore::new(path, dimensions)` → `with_system_config(path, dimensions, verifying_config())` (`src/store/ladybug.rs:707-709`).
- `verifying_config()` = `SystemConfig::default().throw_on_wal_replay_failure(true)` (`src/store/ladybug.rs:605-607`) — **buffer pool and max_db_size are left at engine defaults**.
- `for_testing()` explicitly sets `max_db_size` default 512 MiB and `buffer_pool_size` default 64 MiB (`src/store/ladybug.rs:691-704`). Its doc comment (`:681-690`) says tests would otherwise "mmap-reserve 8 TiB per open", referencing error `Mmap for size 8796093022208 failed`.
- Schema: `CREATE NODE TABLE Memory (... embedding FLOAT[{dimensions}], PRIMARY KEY (id))` (`src/store/ladybug.rs:1431-1437`) — fixed-width array sized by the embedder's declared width.
- Search over-fetch: `FETCH_MULTIPLIER = 10`, `fetch_limit = limit * 10` (`src/store/ladybug.rs:52-53`); query vector is stringified with `format!` into the Cypher text (`:46`, `:86-89`) — a 768-float vector becomes a long literal.

### Learning / Laya
- `src/learning/runtime.rs:24-42` — `ensure_ready()` returns `RuntimeState::Ready`; `runtime()` returns paths only.
- `src/learning/model_bundle.rs:22-31` — `load()` returns a synthetic `v0.0.0-bridge` bundle, no manifest parse.
- `src/learning/trainer.rs:75-80` — `train_candidate()` is a no-op.
- `src/judgment/laya_onnx.rs:67-79` — `judge_duplicates()` acquires a semaphore permit then returns `vec![]`.
- `src/learning/dataset.rs:103-122` — `build_dedup_dataset()` returns a zeroed report (TODO).
- `src/learning/blob_store.rs:10-31` — `BlobStore { cache: Mutex<HashMap<PayloadHash, Vec<u8>>> }`, unbounded, stores a full copy of `payload.content`.
- **`BlobStore` production use sites: NONE.** grep shows only `src/learning/mod.rs:15` (re-export), `src/learning/blob_store.rs` (definition + test), `src/learning/observation.rs:76,90` (test), `src/learning/recorder.rs:63,78` (test).

### Runtime / concurrency
- Tokio runtime built by hand: `Builder::new_multi_thread().enable_all().on_thread_start(...)` (`src/main.rs:1733-1740`). No `.max_blocking_threads()`, no `.worker_threads()`, no `.thread_stack_size()`.
- `Cargo.toml:11` — `tokio = { version = "1.53", features = ["full"] }`.
- `Cargo.toml:17-20` — `fastembed = { version = "7", default-features = false, features = ["ort-download-binaries-rustls-tls", "hf-hub-rustls-tls"] }`.
- `Cargo.toml:63-65` — `[features] default = []; learning-laya = []`. Nothing is gated by `cfg(feature = ...)` yet (grep found no `cfg(feature` uses).
- `src/ingest_jobs.rs:135` — `jobs: Mutex<HashMap<String, watch::Receiver<IngestProgress>>>`; entries are inserted at `:178`; grep found no removal/retention of finished jobs.
- `src/guard/semantic.rs:29,62` call `embedder.embed(...)`; `src/task/mod.rs:319,800,2316` also call `embed`.

---

## Constraints the reviewer should weigh

- `AGENTS.md` §8: adhere to existing conventions; never assume a library is available.
- The plan proposes implementing lanes 1+2+3 together as one change.
- The plan proposes `cargo bloat --release --crates` and `ps -o rss` as pre-change verification.

## What this review must decide

Is the plan's diagnosis correct, are its ranked reducers sound, are the cited line numbers accurate, what did it miss, and what would break if implemented as written?
