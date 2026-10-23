# Review Panel Report

**Work reviewed:** Memory-reduction plan (`docs/reviews/2026-10-22-memory-plan-validation/PLAN.md`)
**Repository:** NeuroStrata @ `main` / `219e47d` | **Date:** 2026-10-22
**Panel:** 6 reviewers + Completeness Auditor + Supreme Judge
**Verdict:** **REJECT-AND-REWRITE** | **Score:** 3.2/10 | **Confidence:** High
**Review mode:** Precise (code claims) / Exhaustive (plan prose)
**Auto-detected signals:** Rust, ONNX/ML inference, embedded graph DB, tokio async
**Data flow trace:** Skipped — plan-only artifact; claims verified against source in Phases 10/11
**Codebase state:** `main`, up to date with `origin/main`

> ⚠️ **Method note:** the panel ran with reduced isolation. Subagent role prompts in this
> runtime cannot act as adversarial reviewers (the `oracle` and `fixer` agents refuse the
> role; `explorer` is read-only), so reviewers returned findings as text and the
> orchestrator persisted them. Debate rounds 1-3 were therefore not run as independent
> rounds. All findings were instead adjudicated by ground-truth verification, which is
> where the panel's strongest results came from. Treat consensus claims with more caution
> than usual; treat the verified defects below with less.

---

## Executive Summary

The plan's central intuition is sound and its execution is not. Local ONNX embedding really
is a several-hundred-megabyte fixed cost, and the remote path really would remove it. But
the plan optimizes **three levers that do not exist or run backwards** (an inert `BlobStore`,
a token cap that destroys recall, batching that raises peak memory), **two that would not
compile** (`RwLock` against a `&mut self` API, a tokio feature list missing `fs`/`net`/
`signal`), and **omits the one unbounded consumer that scales with the host's RAM** — the
database buffer pool, which defaults to a ceiling of 80% of physical memory.

It also omits the migration any model change requires (the embedding column is fixed-width
`FLOAT[N]`), and it proposes making remote embedding the default without noticing that the
ingest path clears the namespace *before* embedding and only logs per-symbol failures — so
a network blip would empty a code index and exit 0.

Score **3.2/10**. Rewrite ordered by evidence strength, not by claimed byte savings.

---

## Score Summary

| Reviewer | Persona | Intensity | Score | Recommendation |
|---|---|---|---|---|
| Feasibility Analyst | delivery realism | 60% | 4/10 | REJECT-AND-REWRITE |
| Code Quality Auditor | line-by-line accuracy | 40% | 3/10 | REJECT-AND-REWRITE |
| Risk Assessor | failure modes | 30% | 2/10 | REJECT-AND-REWRITE |
| Devil's Advocate | framing attack | 20% | 3/10 | REJECT-AND-REWRITE |
| Performance Specialist | memory mechanics | 50% | 3/10 | REJECT-AND-REWRITE |
| Rust Reviewer | idiom + tokio/ort | 40% | 4/10 | REJECT-AND-REWRITE |

Mean 3.2/10. Spread 2 — below the correlated-bias threshold, but note all six inherited the
plan's framing before four of them rejected it.

---

## Consensus Points

1. **The diagnosis is unmeasured.** No RSS attribution exists anywhere. The "523 MB" figure
   in `src/embed.rs` describes a *model cache*, not resident memory. `cargo bloat` measures
   binary size. `ingest --help` exits during clap parsing (`src/main.rs:1747-49`) before the
   embedder is ever constructed (`:1874-85`).
2. **`BlobStore` is inert.** Unbounded map at `src/learning/blob_store.rs:11,28`, but no
   production call site — only a re-export and `#[cfg(test)]` uses.
3. **Tokio feature trimming is not thread trimming.** Thread counts come from the runtime
   `Builder` (`src/main.rs:1733-1740`), not from cargo features. And the proposed list omits
   `fs`/`io-util`/`net`/`signal`, all used in `src/server.rs` and `src/daemon.rs`.
4. **Any embedder change needs a migration.** The schema is `embedding FLOAT[N]`, fixed at
   creation (`src/store/ladybug.rs:1431-1437`). No embedder identity is stored; no re-embed
   tooling exists.
5. **Lane numbering is self-contradictory.** `PLAN.md:84-85` says "1+2+3" but names lane 4.

---

## Disagreement Points (with judge rulings)

### D1 — Is the DB buffer pool the dominant consumer?

- **Performance Specialist:** "could rival model memory" (ranked consumer #1).
- **Devil's Advocate:** "unproven, but points at an unbounded lever the plan never mentions."

**Ruling: Devil's Advocate overreaches less; Performance Specialist overclaims.**
`buffer_pool_size = 0` resolves to a *ceiling* of 80% of physical RAM
(lbug `main/database.cpp:52-70`, `constants.h:56`), not an allocation. The pool fills only as
pages are touched, so on a small database resident usage may be modest. What is *certain*:
the ceiling is unbounded by configuration, it scales with installed RAM, and the test path
deliberately bounds it to 64 MiB (`ladybug.rs:691-704`) while production does not. Bound it
and measure — do not assert dominance.

### D2 — Severity of batching/RwLock

- **Performance Specialist, Rust Reviewer:** P1.
- **Feasibility Analyst:** P2.

**Ruling: P2.** Direction credible, magnitude unmeasured. Kept as a hazard to avoid, not a
defect to fix.

---

## Completeness Audit Findings

Two omissions that change the shape of the fix:

1. **No target and no rollback.** The plan defines no target RSS and no abort path, so it
   cannot succeed or fail. For a change that alters vector semantics, rollback *is*
   re-embedding — the migration the plan omits.
2. **The operator cannot bound the pool today.** `NEUROSTRATA_TEST_BUFFER_POOL_SIZE` exists
   but is read only inside `for_testing()`. The cheapest immediate mitigation needs a new
   production read path, which the plan does not mention.

The judge adds: the plan never asks *which failure mode* it targets. "RSS grows during long
ingests and never returns" points at ORT arena high-water retention; "RSS is high while idle"
points at weights and the pool ceiling. Without that discrimination, a corrected plan could
still optimize the wrong axis.

---

## Coverage Gaps

No reviewer examined: ORT arena high-water retention behaviour; daemon lifecycle policies
(periodic restart as a bounded-ceiling lever); whether `learn`-module code contributes RSS at
all (it contributes binary size, and its feature flag is decorative — `Cargo.toml:63-65`
declares `learning-laya` but no `cfg(feature = ...)` exists anywhere).

---

## Action Items

Ordered by evidence strength, not claimed savings.

### P0 — do these first

1. **`[VERIFIED]` Bound the production buffer pool.**
   `LadybugStore::new` → `verifying_config()` passes `SystemConfig::default()`
   (`ladybug.rs:605-607,707-709`), whose `buffer_pool_size: 0` the engine resolves to
   **80% of physical RAM** (`lbug main/database.cpp:52-70`). Add a config field, plumb it,
   and set an explicit budget. Keep `throw_on_wal_replay_failure(true)` on the normal open.
   Also add a production read path for pool sizing (today only `NEUROSTRATA_TEST_*` exists).
   *This is the single highest-ceiling, lowest-risk change in the whole set.*

2. **`[VERIFIED]` Establish a measured baseline before any reducer.**
   Measure the **daemon** PID only (not the proxy). Use `/usr/bin/time -l` for peak and
   `vmmap <pid>` (macOS) or `/proc/<pid>/smaps_rollup` (Linux). Steps: after startup, after
   first inference, after a fixed ingest, and after idle. Discriminate idle-high from
   grows-never-returns.

3. **`[EXISTING_DEFECT]` Fix the ingest data-loss path before touching the embedder.**
   `ingest_directory` calls `clear_ingested()` before the walk (`ingest.rs:544-553`) and the
   embed-failure branch only logs (`:490-507`), so the run exits `Ok(())` with the namespace
   emptied. Stage the ingest and swap on full success; propagate embed/upsert failures.

### P1 — required for a safe rewrite

4. **`[VERIFIED]` Fix the remote retry, which never fires.**
   `try_embed` returns a plain `anyhow` string for HTTP errors (`embed.rs:267-271`), but
   `embed_with_retry` downcasts to `reqwest::StatusCode` (`:242`). The downcast never
   matches, so 429/5xx are not retried.

5. **`[VERIFIED]` Add a model/dimension guard and a migration before any embedder change.**
   Persist embedder identity + width; refuse startup on mismatch; provide an explicit
   re-embed path. Note `README.md:303` ships a 1536-dim remote example against 768-dim local
   defaults — the mismatch is one config edit away.

6. **`[VERIFIED]` Keep remote embedding opt-in. Do not make it the default.**
   Defaults are local (`embed.rs:48-63`); remote requires both `base_url` and `api_key_env`
   (`:303`). Making it default exports every ingested symbol's source text to a third party,
   adds a network dependency to startup, and — combined with item 3 — turns a blip into data
   loss.

7. **`[VERIFIED]` Either drop the tokio feature change or do it correctly.**
   `full` enables `fs, io-util, io-std, macros, net, parking_lot, process, rt,
   rt-multi-thread, signal, sync, time`. The proposed subset omits `fs`, `io-util`, `net`,
   `signal`, used at `server.rs:5,1356-1364` and `daemon.rs:601,1044-1068`. If thread memory
   is actually implicated, the lever is `Builder::max_blocking_threads` at
   `main.rs:1733-1740`, not cargo features.

8. **`[VERIFIED]` Drop `BlobStore` from this plan entirely.**
   No production caller. If it is ever wired in, replace `.expect()` at
   `blob_store.rs:28` with error propagation and drop the per-insert `eprintln!`.

9. **`[VERIFIED]` Keep `MAX_EMBED_TOKENS = 2048`. Do not lower it to 512.**
   4000 chars / ~3.5 chars-per-token ≈ 1143 tokens; 512 covers ≈1792 characters and would
   silently truncate most of a maximal symbol. Lowering the token cap does not reduce weight
   residency — it only reduces the activation peak when inputs exceed the cap. If shrinking
   is the goal, reduce stored text and token cap *together*, and measure recall.

### P2 — hazards and cleanup

10. **Keep the `Mutex`; do not adopt `RwLock`.** `TextEmbedding::embed` needs `&mut self`
    (`embed.rs:180`); a read guard will not compile and a write guard serializes identically.
    Batching is a throughput change that raises peak memory (≈6 MiB per `[1,2048,768]`
    tensor; batch 32 ≈ 192 MiB). Only batch behind a bounded queue, and measure peak.
11. **Treat `FETCH_MULTIPLIER = 10` as a query-cost issue, not an RSS issue.** The
    stringified 768-float query vector (`ladybug.rs:46,86-89`) is transient. Reducing the
    multiplier can return fewer than `limit` domain-matching rows — preserve filter semantics.
12. **Prune finished ingest jobs.** `jobs` map (`ingest_jobs.rs:135,178`) has no removal path.
13. **Fix the lane numbering and split the rollout.** `PLAN.md:84-85` names lane 4 as part of
    "1+2+3". Make each reducer an independently measurable, revertible commit.

---

## Scope & Limitations

Reviewed: the plan document and every file/line it cites, plus the pinned `lbug` and
`fastembed` sources where the plan depended on third-party defaults.

Not evaluated: runtime behaviour under your actual workload, your actual database size, or
your machine's RAM — none of which were available. The buffer-pool finding predicts a
*ceiling*, not a measured usage; only item 2 can tell you whether that ceiling is being
reached.

**Structural limitation:** this panel ran without independent debate rounds (see the method
note at the top). Its value therefore rests on Phase 10/11 verification rather than on
ensemble diversity — and that verification did hold up: 9/9 panel-introduced claims were
confirmed against source, with 0 hallucinations, and 9/16 plan claims were found inaccurate
or misattributed.

Epistemic labels: `[VERIFIED]` `[EXISTING_DEFECT]` `[PLAN_RISK]` `[CONSENSUS]`
`[UNVERIFIED]` `[DISPUTED]`

---

## Detailed Reviews

Full per-reviewer text, the claim-verification table, the severity table, and the judge's
ruling are in `state/`:

- `state/reviewer_feasibility_analyst_phase_3.md`
- `state/reviewer_code_quality_auditor_phase_3.md`
- `state/reviewer_risk_assessor_phase_3.md`
- `state/reviewer_devils_advocate_phase_3.md`
- `state/reviewer_performance_specialist_phase_3.md`
- `state/reviewer_rust_reviewer_phase_3.md`
- `state/phase_8_audit.md`
- `state/phase_10_claim_verification.md`
- `state/phase_11_severity_verification.md`
- `state/phase_14_judge_ruling.md`
