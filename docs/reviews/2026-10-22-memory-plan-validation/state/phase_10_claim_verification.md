# Phase 10 — Claim Verification

Every load-bearing citation checked against source. Verified by the orchestrator with
`read`/`rg` against the working tree at commit `219e47d`, plus primary sources under
`~/.cargo/registry/src/index.crates.io-*/`.

## Plan claims

| # | Claim (PLAN.md) | Cited | Verified content | Verdict |
|---|---|---|---|---|
| 1 | `FastEmbedder` holds `Arc<Mutex<TextEmbedding>>` | `embed.rs:128-169` | Exact match; `model: Arc<Mutex<TextEmbedding>>` at :129 | VERIFIED |
| 2 | Defaults are Nomic/BGE at 768 dims | `embed.rs:48-63` | `NomicEmbedTextV15` and `BGEBaseENV15`, both `dimensions: 768` | VERIFIED |
| 3 | `MAX_EMBED_TOKENS = 2048` | `embed.rs:126` | Exact | VERIFIED |
| 4 | `MAX_SYMBOL_CHARS = 4000` | `ingest.rs:31` | Exact | VERIFIED |
| 5 | 512 tokens "matches" 4000 chars | `PLAN.md:35-37` | Embedder comment says ~3.5 chars/token => 4000/3.5 ≈ 1143. 512 ≈ 1792 chars. **512 truncates ~55% of a maximal symbol.** | INACCURATE |
| 6 | `BlobStore` unbounded `Mutex<HashMap<.., Vec<u8>>>` | `blob_store.rs:11,28` | Struct is correct. **But no production call site** — only mod re-export + `#[cfg(test)]` uses (`observation.rs:76,90`; `recorder.rs:63,78`). | MISATTRIBUTED (as a runtime reducer) |
| 7 | `FETCH_MULTIPLIER = 10` | `ladybug.rs:52-53` | Exact; `fetch_limit = limit * FETCH_MULTIPLIER` | VERIFIED |
| 8 | Query vector stringified via `format!`, ~10 KB | `ladybug.rs:46` | `vec_str` built at `:46`, embedded in query at `:86-89`. 768 floats ≈ 8-10 KB of text. Transient per query. | VERIFIED but low impact |
| 9 | `build_embedder()` may be called per request | `PLAN.md:38` | 3 call sites, all process startup: `main.rs:1880` (daemon), `:2436`, `:2658` (CLI). Proxy builds none (`server.rs:855+`). | INACCURATE |
| 10 | `tokio features = ["full"]` | `Cargo.toml:11` | Exact | VERIFIED |
| 11 | `full` causes a 512-thread blocking pool | `PLAN.md:53-59` | Features enable APIs; thread counts come from `Builder`. `main.rs:1733-1740` sets only `enable_all` + `on_thread_start`. 512 is tokio's lazy *cap*, and stack reservation is virtual. | INACCURATE |
| 12 | Proposed trimmed feature list compiles | `PLAN.md:57` | Source uses `tokio::net`, `fs`, `signal`, `io` at `server.rs:5,1356-1364` and `daemon.rs:601,1044-1068`. Proposed list omits `fs`, `io-util`, `net`, `signal`. | INACCURATE (would break build) |
| 13 | Batching 32-64 reduces memory | `PLAN.md:61-65` | Trait is scalar (`traits.rs:141-144`); ingest awaits one embed per symbol (`ingest.rs:490-508`). Batching raises activation peak: one `[1,2048,768]` f32 tensor ≈ 6 MiB; batch 32 ≈ 192 MiB. | INACCURATE (likely increases) |
| 14 | `RwLock` improves the embed path | `PLAN.md:65` | `TextEmbedding::embed` needs `&mut self` (`let mut guard` at `embed.rs:180`); ORT sessions are not safe for concurrent `run`. Read guard won't compile. | INACCURATE |
| 15 | `ingest_jobs` retains finished jobs | `PLAN.md:86` | `jobs: Mutex<HashMap<String, watch::Receiver<..>>>` inserted at `:178`; no removal found. | VERIFIED |
| 16 | Lane sequencing "1+2+3" | `PLAN.md:84-85` | Names remote, BlobStore, tokio — i.e. reducers 1, 2, 4 — and omits reducer 3. | INACCURATE |

## Claims introduced by the panel (independently checked)

| # | Claim | Source | Verified content | Verdict |
|---|---|---|---|---|
| 17 | Production DB opens leave the buffer pool at engine defaults | Performance Specialist, Rust Reviewer | `LadybugStore::new` -> `verifying_config()` = `SystemConfig::default().throw_on_wal_replay_failure(true)` (`ladybug.rs:605-607,707-709`). `SystemConfig::default()` sets `buffer_pool_size: 0`, `max_db_size: u32::MAX` (lbug `src/database.rs`). | VERIFIED against engine source |
| 18 | `buffer_pool_size = 0` means 80% of physical RAM | Performance Specialist, Rust Reviewer | lbug `main/database.cpp:52-70`: when 0 or -1u, `bufferPoolSize = 0.8 * min(total_phys, UINTPTR_MAX)`, capped at `0.8 * DEFAULT_VM_REGION_MAX_SIZE`. `constants.h:56` `DEFAULT_PHY_MEM_SIZE_RATIO_FOR_BM = 0.8`. | VERIFIED against engine source |
| 19 | `max_db_size` sentinel maps to 8 TB | Rust Reviewer | `database.cpp:82-85`: `maxDBSize == -1u` -> `DEFAULT_VM_REGION_MAX_SIZE`; `constants.h:63` = `1 << 43` (8 TB) on 64-bit. This is a virtual reservation, not RSS. | VERIFIED (virtual, not resident) |
| 20 | Tests bound the pool to 64 MiB / 512 MiB | Everyone | `for_testing()` reads `NEUROSTRATA_TEST_BUFFER_POOL_SIZE` (default 64 MiB) and `NEUROSTRATA_TEST_MAX_DB_SIZE` (default 512 MiB) at `ladybug.rs:691-704`. Only used in `for_testing`. | VERIFIED |
| 21 | Ingest clears before embedding and swallows failures | Risk Assessor | `ingest.rs:544-553` `clear_ingested()` before the walk; embed error branch at `:490-507` only `eprintln!`; directory walk returns `Ok(())` at `:518`. | VERIFIED |
| 22 | Remote 429/5xx retry never fires | Risk Assessor | `try_embed` returns `anyhow!("API error {}: {}", status, body)` (`embed.rs:267-271`) — no `StatusCode` in the chain; `embed_with_retry` tests `e.downcast_ref::<reqwest::StatusCode>()` (`:242`). Downcast never matches. | VERIFIED |
| 23 | README ships a 1536-dim remote example vs 768 local | Risk Assessor | `README.md:303` `"dimensions": 1536`; `README.md:299` `768`; README:323 states local defaults are 768. | VERIFIED |
| 24 | Schema is fixed-width `FLOAT[N]`, ignores "already exists" | Risk Assessor | `ladybug.rs:1431-1437` `embedding FLOAT[{}]` formatted from `self.dimensions`. | VERIFIED |
| 25 | No embedder identity stored / no re-embed tooling | Risk Assessor | `embedder_id()` exists on the trait but no schema column stores it; no re-embed command in `main.rs` subcommands. | VERIFIED (absence) |

## Summary

- 16 plan claims: 6 VERIFIED, 1 VERIFIED-but-low-impact, 9 INACCURATE or MISATTRIBUTED.
- 9 panel-introduced claims: all 9 VERIFIED against source, including two verified against
  the lbug engine's own C++ and Rust sources.
- The panel did not hallucinate a single new claim. No `[HALLUCINATED]` verdicts.
