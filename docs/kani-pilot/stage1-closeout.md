# Kani Pilot — Close-Out

**Owner ruling (the owner, 2026-11-18):** Stage 1 is **accepted as a partial success** and the pilot is **closed on this split**.

## What the ruling settles

- The pilot outcome is recorded as a partial success under the frozen pre-registration: success criteria 1 (refactor-in-place), 3 (atomics gate), 4 (seeded-defect check, 5/5 mutants caught), and 6 (reporting with produced/verified splits and the full register) were met; criterion 2 (all harnesses pass) was not — 9 of 13 harnesses verified, the other 4 unverified by solver timeout / CBMC memory exhaustion on the pilot machine, each carried by an executable dual-use test; criterion 5 (CI ceiling) was designed to the ceiling with the runtime margin recorded as a risk.
- No kill criterion fired at any point in the pilot.
- The headline finding stands as the pilot's principal technical result: Kani 0.68 cannot compile the `regex` crate (compiler ICE, register T4-S1), so the scanner's extraction layer and everything above it is outside proof scope for this toolchain. The verified set covers the pure decision predicates below that seam.

## What remains, per the pre-registration's own terms

- The deliverables stand: the Stage 0 census (`data/`), the harness set and dual-use tests, the trusted-annotation register (`state/g2-bridge-design.md`, T1–T6 with sub-entries), the execution record (`state/stage1-log.md`, `state/runs/`), and both reports (`stage0-report.html`, `stage1-report.html`).
- The work lives on the local branch `feat/kani-secrets-scanner-pilot` (commits `bfee639`, `8b36ec9`) in `work/neurostrata`. **Nothing was pushed and no PR was opened.** Publishing the branch, or any part of it, remains a separate decision for the owner.
- The four unverified harnesses remain in the file as produced, documented in situ, excluded from the CI lane's enumeration. **The owner additionally accepts (2026-11-18) that more work is needed on these harnesses on a bigger-capacity machine** — their non-verification here is a machine-capacity result (CBMC memory exhaustion and solver timeouts on an ~8 GB shared VM), not a property result. That re-run is accepted as the known remaining work; it is not scheduled, no machine is designated, and running it remains a separate authorization when a suitable machine exists. It is not a continuation of this pilot, which is closed.
- Any wider use of Kani — other subjects, a required CI lane, daemon-wide adoption — remains a separate decision, exactly as the panel verdict and the pre-registration specified. This close-out authorizes none of it.
