# Phase 14 — Supreme Judge Ruling

**Panel:** Feasibility Analyst (4/10), Code Quality Auditor (3/10), Risk Assessor (2/10),
Devil's Advocate (3/10), Performance Specialist (3/10), Rust Reviewer (4/10)
**Mean score:** 3.2/10
**Verdict:** REJECT-AND-REWRITE
**Confidence:** High

## 1. Verification review

Every one of the nine panel-introduced claims was verified against source, including two
against the pinned lbug dependency's own C++ and Rust sources. Zero hallucinations. The
panel's highest-value contributions were all *new* material — none of them came from the
plan itself.

Two plan claims were caught as directionally wrong rather than merely imprecise:

- Lowering `MAX_EMBED_TOKENS` to 512 to "match" 4000 characters is backwards. The code's
  own stated ratio (~3.5 chars/token) requires ~1143 tokens. 512 covers ~1792 characters
  and would silently truncate the majority of a maximal symbol. This is a recall
  regression, not a memory optimization: token count does not change weight residency.
- Batching 32-64 embeds is presented as a memory reduction. It is a memory *increase* at
  peak (one `[1,2048,768]` f32 tensor ≈ 6 MiB, so 32 ≈ 192 MiB), and the proposed `RwLock`
  does not even compile against a `&mut self` inference API.

## 2. Debate quality

Unanimity on direction (all six: reject and rewrite) with meaningful divergence on
mechanism. The Performance Specialist and Rust Reviewer independently reached the same
conclusion from different evidence — one by ranking consumers, one by reading the engine's
`SystemConfig` defaults. That convergence is the strongest signal in this review.

Sycophancy check: no reviewer softened a position under pressure, because the panel ran
without cross-talk. Score spread is 2 (2 to 4), below the correlated-bias threshold, but
the *agreement on diagnosis* is itself partly a shared-blind-spot risk: all six inherited
the "embedder dominates" framing from the plan. Four of six then explicitly rejected it.

## 3. Rulings on disagreement

**Disagreement: is the DB buffer pool the dominant consumer, or merely unmeasured?**
Performance Specialist: "could rival model memory" (row 1 of its table).
Devil's Advocate: "unproven, but points at an unbounded lever the plan never mentions."

Ruling: **Devil's Advocate is correct and Performance Specialist overreaches.**
`buffer_pool_size = 0` resolves to a *ceiling* of 80% of RAM, not an allocation. The pool
fills only as pages are touched. On a small database, actual resident usage may be a few
MB. What is certain is that the ceiling is unbounded by configuration today, that it scales
with installed RAM, and that the test path deliberately bounds it to 64 MiB. The correct
action is to bound it explicitly and measure — not to assert it is the dominant consumer.
This distinction does not weaken the finding; it converts an overclaim into a safe,
low-risk, high-ceiling fix.

**Disagreement: severity of the batching concern.**
Performance Specialist and Rust Reviewer: P1. Feasibility Analyst: P2.
Ruling: **P2, as the direction is credible but the magnitude is unmeasured.** Demoted per
Phase 11. It stays in the action list as a hazard to avoid, not a defect to fix.

## 4. Consensus correctness

The panel agrees on five things, and the judge confirms all five:

1. The plan is aimed at a hypothesis, not a measurement. No RSS attribution exists.
2. `BlobStore` has no production caller and cannot reduce current RSS.
3. `tokio` feature trimming is not a runtime-thread fix, and the proposed list omits
   features the code uses.
4. Changing embedder width or identity without migration breaks a fixed-width schema and
   can silently mix vector spaces.
5. The plan's internal lane numbering is wrong.

## 5. Independent gap assessment

The panel missed two things that change the shape of the fix (see Phase 8):

- **No target and no rollback.** The plan cannot succeed or fail because it defines no
  target RSS and no abort path. For a change that alters vector semantics, rollback means
  re-embedding — which is the very migration the plan omits.
- **The operator has no way to bound the pool today.** `NEUROSTRATA_TEST_BUFFER_POOL_SIZE`
  exists but is read only inside `for_testing()`. The cheapest immediate mitigation requires
  a new production read path, which the plan does not mention.

The judge adds one further consideration the panel under-weighted: the plan never asks
*which failure mode* it is addressing. "RSS grows during long ingests and never returns" is
a very different problem from "RSS is high while idle." The first points at ORT arena
high-water retention and the ingest pipeline; the second points at weights and pool
ceiling. Without that discrimination, even a corrected version of this plan could be
optimizing the wrong axis.

## 6. Final verdict

**REJECT-AND-REWRITE. Score 3.2/10. Confidence: High.**

The plan is not wrong about the *candidate* — local ONNX weights are genuinely a
several-hundred-megabyte fixed cost, and the remote path genuinely removes it. It is wrong
about almost everything downstream of that observation: it optimizes levers that do not
exist (BlobStore), levers that run backwards (token cap, batching), levers that would not
compile (RwLock, trimmed tokio features), and it omits the one unbounded consumer that
scales with the host's RAM (the buffer pool), plus the migration that any model change
requires and the data-loss path that making remote the default would expose.

The rewrite should be ordered by evidence strength, not by claimed byte savings.
