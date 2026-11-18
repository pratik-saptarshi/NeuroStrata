# Kani Pilot v2 — Rerun

This directory contains the v2 rerun of the Kani pilot on the secrets scanner.
The v1 findings pack remains in `../kani-pilot/`; v2 is a new pilot with a new
pre-registration, not a continuation.

## Status

| Phase | State |
|---|---|
| 0 — preflight, rebase, baseline | complete (59/59 secrets tests) |
| 1 — capacity rerun of the v1 harness set | complete; 8/12 verified, JWT/key-scan still intractable, base64 host ICE |
| 2 — symbolic harnesses on Kani 0.68 | complete; **3 symbolic proofs** (entropy, depth, string leaf) |
| 3 — detection core on Kani `main` (#5066) | attempted; #5024 cleared, blocked by #5101 thread-local `AnonConst`; detection claim withdrawn |
| 4 — mutant matrix v2 | partial; M3′ and M4′ caught **by proof** (false negative) |
| 5 — CI lane | measured on PR #113: first cold run breached 15 min; renegotiated runs **passed all 12 harnesses in 13m26s and 10m32s** on x86_64 Linux |
| 6 — report and close-out | run log written; final report pending |

See `run-log.md` for the full execution record and `pre-registration.md`
(including Amendments 1–2) for the frozen terms.

## Basis

- Branch: `feat/kani-rerun-v2`
- Base: `6f805d15` (v1.19.0). `src/secrets.rs` is identical to the v1 census
  basis `dd1affef`; the three intervening commits do not touch it.
- Machine: Apple M3 Max, 16 cores, 64 GB RAM, macOS 27.2.1
  (`aarch64-apple-darwin`). Canonical CI target: `x86_64-unknown-linux-gnu`.
- Toolchain: Kani 0.68.0 / CBMC 6.11.0 for Phases 1–2; Kani `main` at or after
  `1bf25ce` for Phase 3.

## Local toolchain

```bash
source /Users/neo/Build/kani-rerun/env.sh
cargo kani --version
```

## Baseline (2026-11-18)

```
cargo test --bin neurostrata-mcp secrets:: -- --test-threads=1
59 passed; 0 failed; 0 ignored
```

## Run log

Phase 1 logs are under `runs/phase1/`. Every harness run records the clock
time, exit status, and the exact Kani version. A harness is only `verified` if
Kani reports `VERIFICATION SUCCESSFUL` with zero failed checks and no
unwinding-assertion truncation.

## Frozen terms

See `pre-registration.md`. Success requires at least three symbolic (`P`/`B`)
harnesses, at least one proof-carried false-negative mutant, an end-to-end
concrete scanner result on Kani `main`, and a measured CI lane. There is no
"partial success" outcome.
