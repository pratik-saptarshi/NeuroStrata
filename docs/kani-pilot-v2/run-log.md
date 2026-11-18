# Kani Pilot v2 — Run Log

**Run date:** 2026-11-18
**Branch:** `feat/kani-rerun-v2` (base `6f805d15`, v1.19.0)
**Host:** Apple M3 Max, 16 cores, 64 GB RAM, macOS 27.2.1
**Toolchain:** Kani 0.68.0 / CBMC 6.11.0 (aarch64-apple-darwin); Kani `main` at `faffcee` (nightly-2026-11-12) for the detection probe.
**Baseline:** `cargo test --bin neurostrata-mcp secrets:: -- --test-threads=1` — **59 passed, 0 failed** (3.08 s) before and after the refactor.

## Phase 1 — capacity rerun of the v1 set

The v1 harnesses were run on the 64 GB host with a 30-minute cap and JSON
export. The point was calibration: how much of v1's "not verified" set was
machine capacity, and how much was formulation cost.

| Harness | Carrier | Result | Wall | Peak RSS |
|---|---|---|---|---|
| `tier1_placeholder_battery` | C | SUCCESSFUL | 21 s | — |
| `tier1_exempt_code_expression` | C | SUCCESSFUL | 37 s | — |
| `tier1_exempt_source_reference` | C | SUCCESSFUL | 43 s | — |
| `tier1_exempt_type_names` | C | SUCCESSFUL | 43 s | — |
| `tier1_entropy_at_threshold` | C | SUCCESSFUL | 21 s | — |
| `tier1_entropy_below_threshold` | C | SUCCESSFUL | 22 s | — |
| `tier1_entropy_above_threshold` | C | SUCCESSFUL | 21 s | — |
| `tier2_depth_fail_closed` | C | SUCCESSFUL | 22 s | — |
| `tier2_key_scan` | C | TIMEOUT (operator cap) | 917 s | 12.4 GB |
| `tier1_jwt_segments_some` | C | TIMEOUT (operator cap) | ~946 s | 25.8 GB |
| `tier1_jwt_segments_none` | C | not run (paired with above) | — | — |
| `tier1_exempt_member_chain` | C | not run | — | — |
| `tier1_base64_battery` | C | **compiler ICE** | — | — |

**Capacity conclusion.** The 8 GB VM's OOMs are gone, but three formulation
costs remain: the JWT/`str::split` battery grows past 25 GB and does not
finish; the key-scan map traversal passes 12 GB and does not finish; base64
hits a host-std compiler bug. More RAM alone is not the answer for the JWT
and key-scan forms.

**Base64 ICE (0.68, aarch64).** `kani-compiler/src/intrinsics.rs:243`:
`assertion failed: matches!(output.kind(), TyKind::RigidTy(RigidTy::Int(IntTy::I32)))`
— the `catch_unwind` intrinsic, reached through `OnceLock`. Reproduced in
isolation in `probe-oncelock` with a minimal `OnceLock` table. The same probe
compiles on Kani `main` but fails on unsupported `ptr_mask` in std, so base64
is deferred to the x86_64 Linux lane.

## Phase 2 — symbolic harnesses on Kani 0.68

The v1 file had no symbolic harness. The rerun added the following and ran
each with a per-harness cap.

| Harness | Carrier | Result | Verification time |
|---|---|---|---|
| `tier2_depth_symbolic` | B | **SUCCESSFUL** | 0.52 s |
| `tier1_entropy_symbolic` | P | **SUCCESSFUL** | 1.42 s |
| `tier2_string_leaf_symbolic` | B | **SUCCESSFUL** | 0.97 s |
| `tier2_depth_fail_closed` | C | SUCCESSFUL | 0.43 s |
| `tier1_placeholder_symbolic` | B | TIMEOUT | 600 s |
| `tier1_jwt_segments_symbolic` (N=8) | B | TIMEOUT | 900 s |
| `tier1_jwt_segments_symbolic` (N=6) | B | TIMEOUT | 600 s |
| `tier1_jwt_segments_symbolic` (N=4) | B | TIMEOUT | 300 s |
| `tier1_jwt_segments_panic` (N=4) | B | TIMEOUT | 300 s |
| `tier1_base64_roundtrip_symbolic` | B | not run (OnceLock host ICE) | — |
| `tier1_base64_invalid_symbolic` | B | not run (OnceLock host ICE) | — |
| `tier2_key_scan_symbolic` | B | not run (concrete form already >15 min) | — |

**Success-bearing set:** `tier1_entropy_symbolic` (P),
`tier2_depth_symbolic` (B), `tier2_string_leaf_symbolic` (B). Three symbolic
properties, satisfying the amended criterion.

**What they mean.**

- `tier1_entropy_symbolic`: for a token meeting the length/class
  preconditions, `meets_high_entropy` returns exactly `e >= 4.2` for **every**
  `f64` the entropy function could return. The entropy value itself remains
  stubbed (register T3); this is the threshold decision, proved.
- `tier2_depth_symbolic`: for every `d` in `[0, MAX_DEPTH+1]`, the traversal
  rejects exactly when `d > MAX_DEPTH`, with the `max_depth_exceeded` reason.
- `tier2_string_leaf_symbolic`: for every 4-byte ASCII string leaf, the
  traversal calls the scanner (the oracle sees it) and does not drop it.
  This is the symbolic half of the M3 family; the concrete key battery covers
  object keys.

## Mutant checks (proof-carried)

Each mutation was applied alone, the named harness run, then the mutation
reverted and `git status` checked.

| ID | Mutation | Harness | Result |
|---|---|---|---|
| M3′ | `scan_value_with`: `Value::String(s) => scan_text(s, location)` → `Value::String(_) => None` (skip metadata string values) | `tier2_string_leaf_symbolic` | **VERIFICATION FAILED**, exactly 1 failing check (`matches!(r, Some(...))`) |
| M4′ | `meets_high_entropy`: `h >= 4.2` → `h > 4.3` | `tier1_entropy_symbolic` | **VERIFICATION FAILED**, exactly 1 failing check (`got == (e >= 4.2)`) |

Both are false-negative mutants and both are proof-carried (`B`/`P`), which
satisfies the amended mutant criterion. M1′/M2′/M5′ are not proof-carried in
this run: M1′ is above the regex seam, M2′'s symbolic JWT form times out, and
M5′'s symbolic placeholder form times out; the concrete batteries and native
tests remain their carriers.

## Phase 3 — Kani `main` and the detection core

- Built Kani `main` at `faffcee` (includes #5066) successfully on this host.
- Minimal regex probe: **#5024 is cleared** (the `regex_automata::meta::strategy`
  ICE is gone), but a new ICE appears:
  `rustc_metadata/src/rmeta/decoder.rs: ... AnonConst did not have any
  attributes stored`, codegen item
  `regex_automata::util::pool::inner::THREAD_ID`.
- The documented `RandomState::new` zero-seed stub (from Kani's
  `tests/perf/hashset`) removes the `RandomState` hit; the regex `THREAD_ID`
  hit remains.
- `probe-oncelock` on `main`: compiles, but verification fails on
  `ptr_mask is not currently supported by Kani`.

**Result:** the detection core (`SecretScanner::build`, `scan_str`,
`validate_provider_form`) is **not reached** on this host/toolchain. Per the
frozen terms and the owner's recommendation, no model seam was built as a
rescue; the detection claim is withdrawn and recorded as a negative result.

## CI lane

Added `.github/workflows/kani-v2.yml` and `scripts/kani-json-check.py`.
The lane installs Kani 0.68.0, runs the declared 12 harnesses, and fails if
any expected harness is missing or not `Success`.

**Measured on PR #113 (run 44391519563):** the job was **cancelled at the
15-minute timeout** while still on the first harness (`tier1_placeholder_battery`).
The install step completed; the cold Kani codegen for the 463-package crate
on a 2-core `ubuntu-latest` runner consumed the budget before a verdict. No
harness failed; no verdict was reached. The 15-minute ceiling is **breached**
and the full lane is renegotiated to 45 minutes (Amendment 3). The local
JSON check was exercised against the four Phase 2 results and passed; a
successful CI run is still pending.

## Local JSON check

```
tier2_depth_symbolic        Success     518 ms
tier1_entropy_symbolic      Success    1418 ms
tier2_string_leaf_symbolic  Success     972 ms
tier2_depth_fail_closed     Success     433 ms
Kani result check: all expected harnesses succeeded
```

## Reproduction

```bash
source /Users/neo/Build/kani-rerun/env.sh
cd /Users/neo/Git-SCM/ns-wt/kani-rerun

# baseline
cargo test --bin neurostrata-mcp secrets:: -- --test-threads=1

# the symbolic success set
for h in tier1_entropy_symbolic tier2_depth_symbolic tier2_string_leaf_symbolic; do
  cargo kani -Z stubbing -Z unstable-options --harness "$h" \
    --output-format terse --export-json "kani-$h.json"
done
```

## Verdict at this point

**Inconclusive, with a positive symbolic core and a negative detection result.**
The rerun proves three properties v1 did not (one over all `f64` entropy
values, one over all depths, one over all bounded string leaves) and catches
two false-negative mutants by proof. It does not reach the scanner's detection
path, because Kani `main` is blocked by the open #5101 thread-local
`AnonConst` class after #5066 clears the regex ICE. The CI lane is written but
unmeasured on GitHub.
