# Phase 3 — Independent Review: Feasibility Analyst

**Agreement intensity:** 60% | **Reasoning:** backward reasoning | **Mode:** PRECISE
**Score:** 4/10 | **Recommendation:** REJECT-AND-REWRITE

## Findings

### [P1] Remote embedder is not a drop-in default
`PLAN.md:33-37,84-85`; `src/embed.rs:48-63,303-324`; `src/store/ladybug.rs:1435-1442`

Defaults select local models (`base_url: None`); a missing API-key env var errors out;
and the fixed-width DB retains vectors from the old model space. 384-dim or 1536-dim
changes — and same-width model swaps — need compatibility checks and re-embedding.

**Fix:** keep remote opt-in; stamp and check model identity + dimensions; define migration.
**Verify:** `rg -n 'base_url|api_key_env|dimensions|CREATE NODE TABLE Memory' src/embed.rs src/store/ladybug.rs`

### [P1] Proposed tokio feature set omits required features
`PLAN.md:55-59`; `src/server.rs:5,1356`; `src/daemon.rs:601,1044`

Proposed features omit direct `io`, `fs`, and `signal` uses; trimming may fail to compile.
Feature selection also does not cap runtime threads.

**Fix:** include required features; tune runtime limits only if profiling finds thread memory material.
**Verify:** `rg -n 'tokio::(net|fs|io|signal)' src`

### [P1] 512-token cap does not match 4,000 characters
`PLAN.md:35-37`; `src/embed.rs:124-126,153-160`; `src/parser/ingest.rs:27-42`

Source states ~3.5 chars/token and silent truncation. 512 tokens ≈ 1,792 chars, far
below the 4,000-char stored body; embedder would describe only part of the stored symbol.

**Fix:** retain adequate context; measure tokenized inputs before changing.
**Verify:** `rg -n 'MAX_EMBED_TOKENS|MAX_SYMBOL_CHARS|truncate_for_embedding' src/embed.rs src/parser/ingest.rs`

### [P1] Diagnosis is unverified
`PLAN.md:12-15,74-80`; `src/embed.rs:73-80,157-162`; `src/main.rs:1749,1880`

The 523 MB comment concerns a model *cache*, not measured resident memory.
`cargo bloat` measures binary size; `ps` is a snapshot; `ingest --help` exits at Clap
parsing before `build_embedder()`.

**Fix:** compare daemon RSS before/after, warmed under a fixed workload; identify the daemon PID separately from the proxy.
**Verify:** `rg -n 'Cli::parse|build_embedder|start_mcp_proxy|Commands::Ingest' src/main.rs`

### [P2] BlobStore has no production caller
`PLAN.md:40-44`; `src/learning/blob_store.rs:10-31,34-57`; `src/learning/mod.rs:15`

Only uses are the definition, a re-export, and tests. Bounding it gives no current daemon RSS reduction.

### [P2] Search allocations are request-time peaks, not persistent model memory
`PLAN.md:46-51`; `src/store/ladybug.rs:52-53,92-95,175-207`

Both search paths over-fetch 10x. "Stream rows" requires driver support that the plan does not establish.

### [P2] Sequencing label is internally inconsistent
`PLAN.md:82-85` — "lanes 1+2+3" actually names lane 4 (tokio) and omits lane 3 (search).

## Strengths
1. Correctly identifies local FastEmbedder as the plausible large resident cost.
2. RemoteEmbedder already exists; startup constructs one shared instance.
3. Finds concrete over-fetch and duplicate-vector allocation sites.

## Weaknesses
1. No valid before/after RSS measurement or workload.
2. Embedding provider/model migration requirements omitted.
3. Bundles unrelated reducers, including one with no production use.
