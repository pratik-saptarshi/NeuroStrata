# Phase 3 — Independent Review: Risk Assessor

**Agreement intensity:** 30% | **Reasoning:** adversarial simulation | **Mode:** PRECISE
**Score:** 2/10 | **Recommendation:** REJECT-AND-REWRITE

## Findings

### [P1] Embedding-width changes can erase ingested rows
`PLAN.md:35-37`; `src/store/ladybug.rs:1435-1442`; `src/parser/ingest.rs:544-552,490-507`

A 384-dim embedder opens an existing 768-dim schema: `init` ignores "already exists" and
marks the schema ready. `ingest_directory` calls `clear_ingested()` BEFORE the walk.
Per-symbol embed failures are logged, not propagated, so ingest reports success without
restoring the cleared symbols. `RemoteEmbedder` never checks the returned vector's width
against the declared width.

**Net effect:** switching models can leave a populated database with its ingested rows
deleted and not replaced, while the command exits 0.

**Mitigation:** validate DB width vs model width before any destructive step; stage the
ingest and swap on full success; validate response vector length.

### [P1] Same-width model switches silently mix vector spaces
`src/embed.rs:164-198,291-297`; `src/store/ladybug.rs:1435-1437,1712-1717`

The DB stores no embedder identity, yet search compares new query vectors against old
stored vectors. Same dimensions avoid a schema error but degrade retrieval silently.
No re-embedding path exists in source.

### [P1] Remote outage can produce a falsely successful, incomplete ingest
`src/embed.rs:217-220,232-250,267-281,303-323`; `src/parser/ingest.rs:544-552,490-507`

- Construction does not check connectivity.
- 429/5xx are converted to `anyhow` strings, so the `StatusCode` downcast in
  `embed_with_retry` never matches and the retry never runs.
- Ingest clears prior rows first and swallows per-symbol embed errors.
- A missing API-key env var instead fails daemon startup entirely.

### [P2] Config switching is underspecified and can silently select local
`src/embed.rs:36-45,65-68,103-112,303-323` — operators must edit
`~/.config/neurostrata/embedders.json` with `model_name`, `dimensions`, `base_url`,
`api_key_env`. Remote requires both URL and key-env. An unmatched `NEUROSTRATA_MODEL`
silently selects the first entry. Invalid JSON falls back to defaults and OVERWRITES the
operator's file.

### [P2] BlobStore reducer has no current production effect
`src/learning/blob_store.rs:10-29`; `src/learning/recorder.rs:55-80` — recorder method is a no-op.

### [P2] Proposed tokio feature trim is incomplete and does not cap threads
`Cargo.toml:11`; `src/main.rs:1733-1739`; `src/server.rs:1356-1364`; `src/daemon.rs:1044-1078`
— omits `fs` and `signal`; retaining `rt-multi-thread` does not set a blocking-thread limit.

### [P2] 512-token limit contradicts the 4,000-character bound
`src/embed.rs:124-126`; `src/parser/ingest.rs:27-42`.

### [P2] Rollout scope is ambiguous and hard to bisect
`PLAN.md:82-85`.

### [P2] RSS diagnosis remains unverified
`PLAN.md:14-27,74-80`.

## Strengths
- Identifies concrete local-embedder and vector-search cost candidates.
- Remote embedder and config hooks already exist.
- Notes serialized embedding and per-call text allocation.

## Weaknesses
- No safe width/model migration plan.
- Remote failure can erase prior ingested rows and still report success.
- Bundled scope lacks RSS evidence; one reducer is unused.
