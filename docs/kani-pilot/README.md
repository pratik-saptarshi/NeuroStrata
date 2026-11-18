# Kani Pilot — Findings Pack

A gated, pre-registered pilot of the Kani Rust verifier on one NeuroStrata
component: the secrets scanner (`src/secrets.rs`), run against upstream
`main` @ `dd1affef` (v1.18.1). The pilot is **closed**: the owner accepted
Stage 1 as a **partial success** on 2026-11-18.

## Outcome in one paragraph

13 Kani harnesses were produced; **9 verified** with zero failed checks and
**4 could not be verified on the pilot machine** (solver timeouts and CBMC
memory exhaustion — a capacity result, not a property result; the owner has
accepted that those four need more work on a bigger-capacity machine). The
pre-registered seeded-defect check passed in full: **all five injected
mutants were caught**, two of them by Kani proofs directly. The headline
technical finding: **Kani 0.68 cannot compile the `regex` crate** (an
internal compiler error), so the scanner's extraction layer and everything
above it is outside proof scope for this toolchain — recorded as register
entry T4-S1 in the bridge design, not worked around. No kill criterion
fired. Any wider use of Kani is a separate decision.

## Contents

- `stage0-report.html` — defect census (100 fix commits), subject
  selection, and the Stage 0 gate assessment.
- `stage1-report.html` — the Stage 1 result: produced/verified split,
  mutant results, the full trusted-annotation register, and the verdict
  against the frozen success criteria.
- `pre-registration.md` — the frozen pilot terms: ceilings, success and
  kill criteria, the five named mutants.
- `g2-bridge-design.md` — how the harnesses bind to the shipped code
  (refactor-in-place), plus the trusted-annotation register (T1–T6).
- `stage1-harness-plan.md` — the two-tier harness architecture and the
  proof-of-life ladder.
- `stage1-log.md` — the execution record: every ladder rung, harness run,
  and mutant check.
- `stage1-closeout.md` — the owner's ruling and what remains.
- `census.md`, `subject-census.md` — the Stage 0 census data behind the
  subject selection.

The code side of the pilot is on this branch: the harness commit
(`src/secrets.rs`, Bolero dev-dependency) and the advisory CI lane
(`.github/workflows/kani.yml`).
