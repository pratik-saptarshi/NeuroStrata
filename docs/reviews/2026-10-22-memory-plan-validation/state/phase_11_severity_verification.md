# Phase 11 — Severity Verification

Every P0/P1 re-read against source. Classified as `[EXISTING_DEFECT]` (true of the code
today) or `[PLAN_RISK]` (materialises only if the plan is implemented as written).
Per protocol, P0 requires an existing defect.

| # | Finding | Panel severity | Type | Verified | Adjusted |
|---|---|---|---|---|---|
| 1 | Ingest clears rows before embedding and swallows per-symbol failures | P1 | EXISTING_DEFECT | Yes — `ingest.rs:544-553`, `:490-507`, `:518` | **P0** (raised: plan proposes enabling remote, making the failure path routine) |
| 2 | Remote 429/5xx retry never fires | P1 | EXISTING_DEFECT | Yes — `embed.rs:242` vs `:267-271` | **P1** (confirmed) |
| 3 | Production buffer pool unset => 80% of RAM ceiling | P1 | EXISTING_DEFECT (configuration condition) | Yes — `ladybug.rs:605-607`, lbug `database.cpp:52-70` | **P0** (raised: this is the dominant lever and the plan omits it) |
| 4 | Width change breaks fixed-width schema; no migration | P1 | PLAN_RISK | Yes — `ladybug.rs:1431-1437`; no re-embed path | **P1** (confirmed) |
| 5 | Same-width model swap silently mixes vector spaces | P1 | PLAN_RISK | Yes — no identity column | **P1** (confirmed) |
| 6 | Diagnosis unmeasured; `cargo bloat`/`ps` insufficient | P1 | PLAN_RISK | Yes — `main.rs:1747-49` exits before `:1874-85` | **P1** (confirmed) |
| 7 | Proposed tokio feature list would not compile | P1 | PLAN_RISK | Yes — `server.rs:5,1356-1364`, `daemon.rs:601,1044-1068` | **P1** (confirmed) |
| 8 | `RwLock` is infeasible for the ORT session | P1 | PLAN_RISK | Yes — `embed.rs:180` needs `&mut` | **P1** (confirmed) |
| 9 | Batching raises peak memory | P1 | PLAN_RISK | Partly — arithmetic sound (6 MiB/tensor at 2048x768), effect size depends on ORT arena reuse | **P2** (demoted: plausible but unmeasured) |
| 10 | 512-token cap truncates ~55% of maximal symbols | P2 | PLAN_RISK | Yes — 4000/3.5 ≈ 1143 | **P1** (raised: silent recall regression on the primary ingest path) |
| 11 | BlobStore bounded → memory saving | P2 | not a defect | No production call site | **Not a finding** (inert; drop from plan) |
| 12 | `FETCH_MULTIPLIER` over-fetch is a memory problem | P2 | not a defect | Real over-fetch, transient only | **P3** (demoted: query cost, not RSS) |
| 13 | Per-request embedder construction | P2 | not a defect | Already not the case | **Not a finding** (drop) |
| 14 | Lane numbering inconsistent | P2 | not a defect | Yes | **P3** (documentation) |

## Severity changes and why

- **Raised to P0 (#1):** the ingest path is already destructive-on-failure, and the plan's
  headline recommendation (make remote the default) converts an edge case into a routine
  one. A network blip would empty a namespace's code index and exit 0.
- **Raised to P0 (#3):** verified against the engine's own source. `buffer_pool_size = 0`
  resolves to 80% of physical RAM, and `max_db_size = u32::MAX` resolves to an 8 TB virtual
  region. This is the one unbounded, RAM-scaling consumer in the process, and the plan does
  not mention it. Note carefully: the 8 TB is virtual; the 80% pool ceiling is the part
  that can become resident.
- **Raised P1 (#10):** a silent recall regression on the main ingest path is more
  consequential than the plan implies; it was filed as "minor".
- **Demoted (#9):** the arithmetic is right but nobody measured ORT's actual activation
  high-water; the direction is credible, the magnitude is speculative.
- **Dropped (#11, #13):** #11 targets code with no production caller; #13 describes
  something the code does not do.

## External-domain claims

None. Every finding is verifiable from this repository or from the pinned dependency
sources under `~/.cargo/registry`. No web verification required.
