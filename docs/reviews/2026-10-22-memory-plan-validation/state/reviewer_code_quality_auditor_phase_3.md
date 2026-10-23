# Phase 3 — Independent Review: Code Quality Auditor

**Agreement intensity:** 40% | **Reasoning:** systematic enumeration | **Mode:** PRECISE
**Score:** 3/10 | **Recommendation:** REJECT-AND-REWRITE

## Claim verification table

| # | Claim | Cited location | Actual content | Verdict |
|---|---|---|---|---|
| 1 | FastEmbed dominates RSS; Laya does not | `PLAN.md:14-27` | FastEmbed loads locally by default; Laya judgment is a no-op. No RSS attribution exists. 523 MB refers to a cache. | MISATTRIBUTED |
| 2 | BlobStore is a live runtime consumer | `PLAN.md:42-44` | Cache is unbounded, but uses are definition, re-export, tests. No production construction. | INACCURATE |
| 3 | 512 tokens cover 4,000 chars | `PLAN.md:35-37` | Code states ~3.5 chars/token: 4,000/3.5 ≈ 1,143 tokens; 512 covers ≈1,792 chars. | INACCURATE |
| 4 | Vector literal ~10 KB materially affects RSS | `PLAN.md:48-51` | `f32::to_string()` builds a temporary Cypher literal. ~10 KB plausible but transient. | MISATTRIBUTED |
| 5 | Embedder may be built per request | `PLAN.md:38` | Production calls at `main.rs:1880,2436,2658` are startup paths. Proxy forwards; builds nothing. | INACCURATE |
| 6 | Trimming tokio `full` reduces threads | `PLAN.md:53-59` | Runtime uses `new_multi_thread().enable_all()`; no worker/blocking limits set. Features don't set them. | INACCURATE |
| 7 | `FETCH_MULTIPLIER = 10` is a memory problem | `PLAN.md:46-51` | Increases transient candidates and query work; not a long-lived cache. | MISATTRIBUTED |
| 8 | Batches of 32-64 reduce memory | `PLAN.md:61-65` | Trait embeds one string; ingest awaits sequentially. Larger batches may raise peak input/output/activation. | INACCURATE |
| 9 | Citations accurate | Multiple | `vec_str` is at `ladybug.rs:46`; model defaults at `embed.rs:48-63,79-81`; `:157,176,207` are not all `Vec<String>` clones. | MISATTRIBUTED |

## Findings

### [P1] Diagnosis lacks RSS evidence
`PLAN.md:9-14,74-80` — `cargo bloat` measures binary size; `ps` gives aggregate process
RSS; neither attributes memory to FastEmbed vs Ladybug. Production DB opens with
`SystemConfig::default()` (`src/store/ladybug.rs:605-607,680-709`) while tests explicitly
cap DB and buffer-pool sizes. Measure daemon RSS across model-load and DB-open stages.

### [P1] Proposed tokio feature set omits required features
`PLAN.md:55-59` — code uses tokio `io`, `fs`, `net`, `signal`
(`src/server.rs:5,1356-1364`; `src/daemon.rs:601,1044-1077`), absent from the proposed
list. `full` also does not configure runtime thread limits.

### [P2] Token reduction silently loses source coverage
`PLAN.md:35-37` — 512 is less than half the stated ~1,143-token requirement. Preserve
2,048 or compute a tokenizer-derived limit covering the truncated body plus headers.

### [P2] BlobStore reducer cannot lower current production RSS
`PLAN.md:42-44` — other references are under `#[cfg(test)]`
(`src/learning/observation.rs:73-90`; `src/learning/recorder.rs:60-78`).

### [P2] Search allocation characterization is imprecise
`PLAN.md:48-51` — `sr.clone()` copies a `SearchResult` (`src/store/ladybug.rs:175-180`),
not a `Vec<String>`; `primary_ids` is the actual cloned string vector (`:207`).
Preserve domain fallback semantics.

### [P2] Batching is not established as a memory reducer
`src/parser/ingest.rs:490-508`; `src/traits.rs:141-144`; `src/embed.rs:174-190`.

### [P2] Per-request embedder construction is not present
Calls at `src/main.rs:1880,2436,2658`; proxy starts at `src/server.rs:855`.

### [P2] Sequencing contradicts reducer numbering
`PLAN.md:84-85` — "Lanes 1+2+3" lists reducers 1, 2 and 4, omitting 3.

## Verification commands
- `ps -axo pid,rss,vsz,command | rg 'neurostrata-mcp'`
- `rg -n 'tokio =|tokio::(net|io|fs|signal)' Cargo.toml src`
