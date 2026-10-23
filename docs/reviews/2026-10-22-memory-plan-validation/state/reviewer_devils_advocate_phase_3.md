# Phase 3 — Independent Review: Devil's Advocate

**Agreement intensity:** 20% | **Reasoning:** analogical reasoning | **Mode:** PRECISE
**Score:** 3/10 | **Recommendation:** REJECT-AND-REWRITE

## Findings

### [P1] Diagnosis is asserted, not measured
`PLAN.md:9,14,74-79`; `src/embed.rs:117-120`; `src/store/ladybug.rs:605-607,681-709`

The 523 MB figure describes a download/cache, not resident RSS. Production opens leave
the DB pool and size at engine defaults; tests explicitly bound both. The 8 TiB warning is
about mmap reservation, not proof of resident use.

**Alternative:** measure daemon RSS/VSZ/PSS under representative load; test DB and
embedder contributions separately before ranking reducers.

### [P1] Model swaps need dimension and index migration
`PLAN.md:33-37`; `src/embed.rs:254-280,291-324`; `src/store/ladybug.rs:1431-1438`

Local defaults declare 768 dims; MiniLM is 384. The remote request sends no dimension and
validates no returned width. The schema fixes embedding width at creation. A direct switch
can fail searches/writes or mix incompatible vector spaces.

### [P1] 512-token cap trades memory for recall
`PLAN.md:35-37`; `src/embed.rs:124-126`; `src/parser/ingest.rs:27-40`

4000 chars / 3.5 ≈ 1143 tokens plus headers. A 512 cap silently discards much of the
allowed symbol text and does not shrink model weights.

### [P2] BlobStore reducer has no production target
`PLAN.md:40-44`; `src/learning/blob_store.rs:10-31` — observed uses are tests; production
`record_blob_hash` is a no-op.

### [P2] Tokio feature trimming is not thread trimming
`PLAN.md:53-59`; `Cargo.toml:11`; `src/main.rs:1733-39`; `src/judgment/laya_onnx.rs:72-79`

The proposed features retain the same multi-thread runtime. A 512-thread *cap* is not 512
resident threads, and stack reservation is not RSS. The Laya semaphore does not bound
FastEmbed calls.

### [P2] Batching/RwLock can raise peak memory
`PLAN.md:61-65`; `src/embed.rs:177-185` — current mutex serializes inference. Batch
inputs raise per-call peak; an RwLock could allow concurrent inference.

### [P2] Search already applies a query LIMIT
`src/store/ladybug.rs:86-93,113-139,183-192` — lowering the 10x multiplier can return
fewer than `limit` domain-matching rows.

### [P2] Remote default displaces work and exports source text
`src/embed.rs:254-263,303-324` — requests include the full input text.

## Strongest counter-argument to the central diagnosis

The plan's 523 MB evidence is cache/download size, not RSS. Production opens the DB with
default resource settings, while tests explicitly cap its pool and size after large mmap
reservations. That does not prove the DB dominates resident memory, but it does make
"the embedder dominates RSS" unproven — and it points at an unbounded lever the plan
never mentions.

## Strengths
1. Identifies a real local ONNX model-loading path.
2. Remote embedding and smaller local models already exist.
3. Spots transient vector-search copies worth profiling.

## Weaknesses
1. No measurement supports the claimed dominant RSS consumer.
2. Ignores production DB defaults and the test-only bounds.
3. Several reducers are ineffective or risk embedding/search correctness.
