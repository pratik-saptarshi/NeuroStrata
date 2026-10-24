# Phase 3 — Independent Review: Performance Specialist

**Agreement intensity:** 50% | **Reasoning:** systematic enumeration + measurement discipline | **Mode:** PRECISE
**Score:** 3/10 | **Recommendation:** REJECT-AND-REWRITE

## Ranked memory-consumer estimates

| # | Consumer | Mechanism | Magnitude | Residency | Evidence |
|---|---|---|---|---|---|
| 1 | Ladybug buffer pool | `SystemConfig::default()` leaves `buffer_pool_size = 0`; engine computes 80% of physical RAM | up to 0.8 x RAM, growing as pages are touched, never returned | Resident | `ladybug.rs:605-709`; lbug `database.cpp:52-70`, `constants.h:56` |
| 2 | ONNX weights | ORT session weight tensors; Nomic ~137M params ≈ 548 MB, BGE-base ~109M ≈ 436 MB, MiniLM-L6 ~22M ≈ 88 MB | Hundreds of MB (tens with quantization) | Mostly resident after warm-up | `embed.rs:138-162` |
| 3 | ORT working set | Session arena + activations; `[1,2048,768]` f32 ≈ 6 MiB per tensor; batch 32 ≈ 192 MiB | MiB (batch 1) to hundreds of MiB (batch 32-64) | Transient peak; arena may retain high-water | `embed.rs:126,160,184` |
| 4 | Ladybug mmap | `max_db_size` default = `u32::MAX` sentinel -> `DEFAULT_VM_REGION_MAX_SIZE` = 8 TB; `MAP_NORESERVE` | 8 TB virtual, ~0 RSS until touched | Virtual | lbug `database.cpp:82-85`, `constants.h:63` |
| 5 | Tokio stacks | Blocking threads lazily spawned up to 512; OS-default stacks | Tens of MiB virtual worst case | Mostly virtual | `main.rs:1733-1739` |
| 6 | Tokenizer | Vocabulary + tokenizer structs | Likely MBs to low tens of MB [UNVERIFIED] | Resident | `embed.rs:157-161` |
| 7 | Allocator fragmentation | Rust/C++ arenas, freed pages retained | Secondary [UNVERIFIED] | Resident | no profiling in plan |
| 8 | BlobStore cache | Unbounded `HashMap` copy of every payload — but no production call site | Zero today | Resident if wired | `blob_store.rs:10-30` |

Note: rows 1 and 4 overlap (touched DB pages count against the pool); do not double-count.

## Findings

### [P1] No valid runtime baseline; diagnosis asserted, not measured
`PLAN.md:14,74-80` — `cargo bloat` is binary code size; `pgrep` may include the proxy;
`ingest --help` exits at `Cli::parse()` (`src/main.rs:1747-49`) before `build_embedder()`
(`:1874-85`).

**Fix:** measure the daemon PID after model load and after controlled inference/ingest.
Use `/usr/bin/time -l` for peak and `vmmap <PID>` (macOS) or `/proc/<PID>/smaps_rollup`
(Linux) for the resident/virtual breakdown. Keep `cargo bloat` for code size only.

### [P1] Plan misses the DB memory configuration, a plausible dominant consumer
`src/store/ladybug.rs:605-607,681-709` — production leaves all engine defaults; tests cap
the pool at 64 MiB and DB size at 512 MiB. The 8 TiB figure is virtual, but the pool
ceiling is real and scales with installed RAM.

**Fix:** set an explicit, configurable production `buffer_pool_size`; keep `max_db_size`
separate from RSS reasoning. Do not copy the test limits blindly.

### [P1] Batching and removing serialization can increase peak memory
`src/embed.rs:179-185` — one 2048x768 f32 tensor is ~6 MiB; batch 32 is ~192 MiB.
Multiple sessions would duplicate hundreds of MB of weights.

**Fix:** keep serialized inference as baseline; benchmark batch size and concurrency
against peak RSS, throughput, and latency; adopt only if the peak fits the target.

### [P1] Switching to 384 dimensions needs a database/vector migration
`src/store/ladybug.rs:1431-1442` — schema fixes `FLOAT[N]` at creation; reopening an
existing 768-dim table migrates neither schema nor vectors; same-dimension models still
produce incompatible vector spaces.

### [P2] Lowering max length is not a weight optimization and risks truncation
`src/embed.rs:124-26`; `src/parser/ingest.rs:31-42` — 512 tokens ≈ 1,792 chars.

### [P2] BlobStore is not a current production RSS reducer
Test-only; `src/learning/mod.rs:15` only re-exports it.

### [P2] Tokio feature trimming is not a demonstrated RSS lever and the list is incomplete
`src/daemon.rs:601,1044-68`; `src/server.rs:1356-64` need `fs`/`net`/`signal`. Feature
removal mostly changes compiled code, not runtime pool data.

### [P2] Vector-search cleanup is per-query, not a top RSS lever
`src/store/ladybug.rs:49-53,91-93,156-80` — scales with query limit and payload size.

## Corrected measurement procedure

1. Record commit, OS, RAM, model/config, DB size, warm/cold cache.
2. Start the release **daemon**, not a proxy or `--help`; identify its exact PID.
3. Capture RSS/VSZ after startup, after first inference, after ingest/search, after idle.
4. macOS: `/usr/bin/time -l` + `vmmap <PID>`; Linux: `/proc/<PID>/smaps_rollup` + cgroup metrics.
5. Vary one axis at a time: local vs remote, batch 1/8/32/64, ORT threads, pool size.
6. Native heap profiling (Instruments / heaptrack) if anonymous growth is unexplained; ORT
   allocations must be visible.
7. `cargo bloat` only for binary-size questions.

## Strengths
- Correctly treats local ONNX weights as a plausible large fixed cost.
- Recognizes Laya holds no inference weights today.
- Identifies input cloning and serialized embedding as measurable paths.

## Weaknesses
- No baseline or fixed-workload peak measurement.
- Misses production Lbug's 80%-RAM pool cap and 8 TB virtual reservation.
- Proposes irrelevant BlobStore work and risky model/batch changes without safeguards.
