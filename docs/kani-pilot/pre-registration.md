# Kani Pilot — Pre-Registered Metrics and Kill Criteria

**Filed:** 2026-11-17 (Stage 0, before any Stage 1 work — pre-registration per the verification-frameworks panel judge's verdict §5 and audit gap G9).
**Pilot:** Kani bounded model checking on NeuroStrata, one subject, refactor-in-place.
**Subject (selected from the Stage 0 census, see `../data/census.md` and `../data/subject-census.md`):** the secrets scanner, `src/secrets.rs`, upstream `Cognilogical/NeuroStrata` main @ `dd1affef8b685bda92616736ed8ef479cc039224` (v1.18.1).
**Status of this document:** frozen at filing. Any change after Stage 1 begins must be appended as a dated amendment with a reason; silent edits void the pre-registration.

## Stage 0 ceilings (set now, binding on Stage 1)

| Ceiling | Value |
|---|---|
| Verified core size | ≤ 1,200 production LOC (subject core measured ~950 decision LOC + ~120 const tables at census) |
| Harnesses | ≤ 40 `#[kani::proof]` harnesses, all in Bolero dual-use style |
| Harness input envelope | strings ≤ 128 bytes; JSON depth ≤ 4; collection sizes ≤ 8 (production caps — `MAX_SCAN_BYTES` 256,000, depth 8 — are enforced by code outside the proven envelope and are covered by example tests, not by the bounded proofs; this split is stated, not hidden) |
| CI advisory lane | ≤ 15 minutes wall-clock per run (judge's cap) |
| Refactor diff | extraction/harness edits only; zero production behaviour change, evidenced by the full existing test suite (2,176 tests at census basis) staying green |
| Time-boxes | Stage 0: 2 working days. Stage 1: 10 working days of effort. |

## Success criteria — ALL required for a "pilot succeeded" finding

1. **Shipped code is what is verified.** The harnesses exercise the production functions in `src/secrets.rs` that the daemon's write paths call (refactor-in-place; single source of truth). No verified copy, shim duplicate, or parallel reimplementation earns credit.
2. **All harnesses pass, with unwinding assertions passing** on every harness (no harness is green by loop truncation).
3. **Atomics/threads gate:** the verified core's atomics/threads census is zero, or every such item is outside the verified core and named in the trusted-annotation register. (At filing: zero atomics/threads in `src/secrets.rs`; two `OnceLock` immutable statics are register items T1.)
4. **Non-vacuity (seeded-defect check):** all five pre-registered mutants below are injected one at a time into the core and each must make at least one harness fail:
   - M1 — invert one provider-pattern predicate (a known secret format scans clean).
   - M2 — off-by-one on a JWT segment boundary/length check.
   - M3 — delete the metadata-key scan branch (top-level keys only are scanned).
   - M4 — weaken the high-entropy threshold comparison (`>=` → `>`).
   - M5 — remove the placeholder exemption (or symmetrically, exempt all values shaped like a registered provider pattern's placeholder).
   A surviving mutant voids the green result. One harness-repair cycle is permitted; if the check still cannot be made to pass in full, the pilot is killed (judge's criterion).
5. **CI ceiling held:** the advisory lane (design in the Stage 0 report) runs within 15 minutes.
6. **Reporting:** the Stage 1 result is reported with produced/verified splits (harnesses produced vs verified vs failed) and the complete trusted-annotation register. No headline number without its split.

## Kill criteria — ANY one stops the pilot, with no fallback scope

1. Refactor-in-place proves infeasible within the ceilings above, or the only feasible route is verify-a-copy.
2. The subject cannot be isolated from async, FFI, `Drop`/lease semantics, interior mutability, or atomics without rewriting it into a different component. (At filing the subject is already a pure synchronous core; this criterion guards against discoveries during harness work, e.g. a core dependency on a construct Kani cannot model that dominates the decision logic.)
3. Either time-box is exceeded, or the CI ceiling is exceeded.
4. Success would require the unstable Kani contracts path, an unregistered `assume`/stub, or a trusted annotation the register cannot bound.
5. The seeded-defect check cannot be made to pass after one harness-repair cycle.
6. The census basis materially changes under the pilot (e.g. the subject is rewritten upstream mid-pilot) such that the verified code is no longer the shipped code.

## What the outcomes mean (pre-registered)

- **Success** authorizes nothing by itself: any wider Kani use is a new decision requiring its own ruling (judge's verdict §5).
- **Kill** leaves the deliverables behind: the census, and the harnesses re-expressed as property/fuzz tests (Bolero dual-use), which join the alternatives-first stack. Nothing is stranded in Kani-only form.
- **The alternatives-first stack is not assurance-equivalent to proof.** (Judge's mandated wording, verdict §7.9: testing samples executions of the shipped system; TLA+ proves properties of a design model whose fidelity is trusted; a verifier proves properties for all inputs within its stated bounds, supported subset, and trusted base. The stack is sequenced first because it covers NeuroStrata's actual risk surface — not because it delivers the same guarantee.)

— Filed by Musebot-Astro, Stage 0, 2026-11-17.
