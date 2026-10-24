# Review Panel Process History

Chronological, verbatim-where-available log of the panel that reviewed
`PLAN.md` (memory-reduction plan) on 2026-10-22.

---

## Persona Profiles Registry

### Feasibility Analyst
- **Lens:** technical feasibility and delivery realism
- **Agreement intensity:** 60%
- **Reasoning strategy:** backward reasoning — start from the desired outcome, trace back
- **Phases:** 3, 10, 11, 14

### Code Quality Auditor
- **Lens:** line-by-line accuracy of every factual claim
- **Agreement intensity:** 40%
- **Reasoning strategy:** systematic enumeration
- **Phases:** 3, 10, 11, 14

### Risk Assessor
- **Lens:** failure modes, dependencies, rollout risk
- **Agreement intensity:** 30%
- **Reasoning strategy:** adversarial simulation
- **Phases:** 3, 10, 11, 14

### Devil's Advocate
- **Lens:** attack the framing itself
- **Agreement intensity:** 20%
- **Reasoning strategy:** analogical reasoning against known failure patterns
- **Phases:** 3, 14

### Performance Specialist
- **Lens:** allocator-level memory mechanics and measurement discipline
- **Agreement intensity:** 50%
- **Reasoning strategy:** systematic enumeration with magnitude estimates
- **Phases:** 3, 10, 11, 14

### Rust Reviewer
- **Lens:** idiom, ownership, tokio/ort/DB specifics
- **Agreement intensity:** 40%
- **Reasoning strategy:** checklist verification
- **Phases:** 3, 10, 11, 14

### Completeness Auditor
- **Lens:** what the whole panel missed. Does not re-score.
- **Phases:** 8

### Supreme Judge
- **Lens:** domain-neutral arbitration
- **Phases:** 14

---

## Phase 1: Setup

**Content classification:** Mixed (plan prose + code citations).
**Review mode:** Precise for code claims, Exhaustive for prose.

**Persona selection.** Base mixed-content set (Feasibility Analyst, Code Quality Auditor,
Risk Assessor, Devil's Advocate) plus two signal-detected specialists:

- *ONNX / ML inference* signal → Performance Specialist
- *Rust* signal → Rust Reviewer

Six total, at the auto-persona cap.

**Context gathering.** Sibling scan of `src/`, `Cargo.toml`, `Cargo.lock`, `README.md`,
`AGENTS.md`. Reference tracing across `embed.rs`, `parser/ingest.rs`, `store/ladybug.rs`,
`learning/*`, `judgment/laya_onnx.rs`, `main.rs`, `server.rs`, `daemon.rs`. Safety-mechanism
discovery: `clear_ingested`, `throw_on_wal_replay_failure`, `verifying_config`,
`unverified_config`, `preserve_unclean_wal`, `strict_wal_verification`, `zero_vector`,
ingest root-readability precheck.

**Temporal scope check:** not applicable — no temporal exclusion claims in the work.

**Codebase state check:** `main` @ `219e47d`, up to date with `origin/main`, not in a
worktree. No `[STALE_BRANCH]` warning.

**Dependency-source grounding.** The plan relies on third-party defaults, so the pinned
sources were read directly:
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lbug-0.20.4` and
`.../fastembed-7.0.1`.

**Context brief written to:** `CONTEXT_BRIEF.md`.

### Operator note: panel-execution constraint (recorded for honesty)

Three dispatch strategies were attempted before the panel could run:

1. **`oracle` agents** (intended reviewer role) — refused. "Oracle is read-only. The
   orchestrating session must persist the review." No review content produced.
2. **`fixer` agents** — refused. "This is a read-only adversarial review, outside my Fixer
   role. I did not inspect files or write the review."
3. **`explorer` agents** (reads freely, returns text) — succeeded. Reviewers returned
   findings as final message text; the orchestrator persisted them into `state/`.

Additionally, the subagent default model was misconfigured (`opencode/space-bunny-free`,
unavailable) and the `model` parameter was ignored by the dispatcher; the working route was
`opencode-go/gpt-6-luna` at `high` variant.

**Consequence:** Phases 4 (private reflection), 5 (debate rounds), and 7 (blind finals) were
**not run as independent rounds**. The panel had six independent Phase 3 reviews but no
cross-talk. Value was recovered in Phase 10/11 by ground-truth verification. This is a
reduced-isolation run and is flagged as such in the report header.

---

## Phase 3: Independent Reviews

Six reviewers, isolated from each other. Scores: 4, 3, 2, 3, 3, 4.

### Feasibility Analyst — 4/10 — REJECT-AND-REWRITE

Five P1/P2 findings: remote is not a drop-in default; tokio feature list omits required
features; 512 tokens do not match 4000 characters; diagnosis is unverified; `BlobStore` has
no production caller; search allocations are request-time peaks.
Full text: `state/reviewer_feasibility_analyst_phase_3.md`.

### Code Quality Auditor — 3/10 — REJECT-AND-REWRITE

Produced a nine-row verification table. Verdicts: 3 MISATTRIBUTED, 4 INACCURATE,
2 PARTIAL/VERIFIED. Caught that `vec_str` is at `ladybug.rs:46` (not as cited), that
`sr.clone()` copies a `SearchResult` not a `Vec<String>`, and that `primary_ids` is the real
string vector. Arithmetic check: 4000/3.5 ≈ 1143 tokens vs 512 claimed.
Full text: `state/reviewer_code_quality_auditor_phase_3.md`.

### Risk Assessor — 2/10 — REJECT-AND-REWRITE

The most consequential reviewer. Established the destructive ingest path
(`clear_ingested` before the walk at `ingest.rs:544-553`; embed failures only logged at
`:490-507`; `Ok(())` returned), the broken 429/5xx retry (`embed.rs:242` vs `:267-271`), the
fixed-width schema with no "already exists" width check, the absent embedder identity, and
the README's 1536-dim remote example against 768-dim local defaults.
Full text: `state/reviewer_risk_assessor_phase_3.md`.

### Devil's Advocate — 3/10 — REJECT-AND-REWRITE

Strongest counter-argument (verbatim): *"The plan's 523 MB evidence is cache/download size,
not RSS. Production opens the DB with default resource settings, while tests explicitly cap
its pool and size after large mmap reservations. That does not prove the DB dominates
resident memory, but it does make 'the embedder dominates RSS' unproven — and it points at
an unbounded lever the plan never mentions."*
Full text: `state/reviewer_devils_advocate_phase_3.md`.

### Performance Specialist — 3/10 — REJECT-AND-REWRITE

Produced the ranked consumer table and the corrected measurement procedure. Estimated
weights: Nomic ~548 MB, BGE-base ~436 MB, MiniLM-L6 ~88 MB (fp32). Activation math:
`[1,2048,768]` f32 ≈ 6 MiB per tensor, batch 32 ≈ 192 MiB. Ranked the DB pool first,
"up to 0.8 x RAM."
Full text: `state/reviewer_performance_specialist_phase_3.md`.

### Rust Reviewer — 4/10 — REJECT-AND-REWRITE

Refuted `RwLock` on the grounds that `TextEmbedding::embed` requires `&mut self` and ORT
sessions are not safe for concurrent `run`. Enumerated tokio `full`'s actual feature set and
showed the proposed subset omits `fs`, `io-util`, `net`, `signal`. Confirmed the
`buffer_pool_size = 0` / `max_db_size` sentinel behaviour from lbug source.
Full text: `state/reviewer_rust_reviewer_phase_3.md`.

---

## Phases 4, 5, 7: Reflection, Debate, Blind Finals

**Not run.** See the operator note in Phase 1. No cross-talk occurred, so no sycophancy
signal and no convergence analysis is available. Scores in Phase 3 are treated as the
standing positions.

---

## Phase 8: Completeness Audit

Seven overlooked items. The two material ones: the plan has **no target and no rollback
path**, and the operator has **no production surface to bound the buffer pool**
(`NEUROSTRATA_TEST_*` is read only inside `for_testing()`). Also flagged: ORT arena
high-water retention is undiscussed, and the `learning-laya` feature is decorative
(declared at `Cargo.toml:63-65`, never gated).
Full text: `state/phase_8_audit.md`.

---

## Phase 10: Claim Verification

All citations checked against the working tree and against pinned dependency sources.

**Plan claims:** 16 checked — 6 VERIFIED, 1 VERIFIED-but-low-impact, 9 INACCURATE or
MISATTRIBUTED.

**Panel-introduced claims:** 9 checked — **9/9 VERIFIED**, 0 hallucinations. Two were
verified against the engine's own source:
`lbug-src/src/main/database.cpp:52-70` and `src/include/common/constants.h:56,63`, plus the
Rust binding's `SystemConfig::default()` in `lbug-0.20.4/src/database.rs`.

The decisive verification, quoted from `database.cpp`:

```cpp
if (bufferPoolSize_ == -1u || bufferPoolSize_ == 0) {
    auto systemMemSize = ... _SC_PHYS_PAGES * _SC_PAGESIZE;
    bufferPoolSize_ = 0.8 * min(systemMemSize, UINTPTR_MAX);
    bufferPoolSize_ = min(bufferPoolSize_, DEFAULT_VM_REGION_MAX_SIZE * 0.8);
}
...
if (maxDBSize == -1u) { maxDBSize = DEFAULT_VM_REGION_MAX_SIZE; }  // 1<<43 = 8TB
```

Full text: `state/phase_10_claim_verification.md`.

---

## Phase 11: Severity Verification

Three severities raised, two demoted, two findings dropped as non-defects.

- Raised to **P0**: the ingest data-loss path (existing defect; the plan would make it
  routine by enabling remote by default).
- Raised to **P0**: the unbounded buffer-pool ceiling (existing condition; the plan omits it).
- Raised to **P1**: the 512-token recall regression (filed as minor by the plan).
- Demoted to **P2**: batching/RwLock (direction credible, magnitude unmeasured).
- Dropped: `BlobStore` (no production caller) and per-request embedder construction (does not
  occur).

Full text: `state/phase_11_severity_verification.md`.

---

## Phase 14: Supreme Judge Deliberation

**Verdict: REJECT-AND-REWRITE. Score 3.2/10. Confidence: High.**

Rulings:

- *D1 (buffer pool dominance):* Devil's Advocate overreaches less than Performance
  Specialist. The 80% figure is a ceiling, not an allocation; it scales with RAM and is
  unbounded by configuration, which is sufficient to act on without asserting dominance.
- *D2 (batching severity):* P2 — plausible direction, speculative magnitude.

The judge's closing observation: *"The plan is not wrong about the candidate... It is wrong
about almost everything downstream of that observation: it optimizes levers that do not
exist, levers that run backwards, levers that would not compile, and it omits the one
unbounded consumer that scales with the host's RAM."*

Full text: `state/phase_14_judge_ruling.md`.

---

## Phase 14.5: Post-Judge Verification

No judge-introduced P0/P1 findings to verify — every P0/P1 in the ruling traces to a
panel-raised finding already covered by Phase 11. Gate passes; no
`[JUDGE-HALLUCINATED]` entries.

---

## Phase 15: Output Generation

- 15.1 → `review_panel_report.md`
- 15.2 → `review_panel_process.md` (this file)
- 15.3 → `review_panel_report.html`
