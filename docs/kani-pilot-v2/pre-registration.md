# Kani Pilot v2 — Pre-Registration (frozen)

**Filed:** 2026-11-18, before Phase 1 of the rerun.
**Branch:** `feat/kani-rerun-v2`, based on `6f805d15` (v1.19.0).
**Subject:** the secrets scanner, `src/secrets.rs`. The subject file is byte-identical to the v1 census basis `dd1affef`; the three intervening upstream commits do not touch it. This is a new pilot with a new basis, not a continuation of v1.
**Machine:** Apple M3 Max, 16 cores, 64 GB RAM, macOS 27.2.1 (`aarch64-apple-darwin`) for development; canonical CI is `x86_64-unknown-linux-gnu`.
**Toolchain:** Kani 0.68.0 / CBMC 6.11.0 for Phase 1–2; Kani `main` at or after `1bf25ce` (the #5066 niche-discriminant fix) for Phase 3. Every run records the exact version.

## Why v2 exists

v1 is closed as **incomplete**: 13 harnesses were produced, 9 reported `VERIFICATION SUCCESSFUL`, but the shipped set contains no `kani::any` and therefore no symbolic proof. The four "produced but not verified" harnesses are also concrete batteries. v1's verified set covers exemption/helper predicates, not the detection path that prevents a missed secret, and three of its five mutant catches are native tests rather than harnesses.

v2's question is narrower and falsifiable:

> Can Kani prove a property of this scanner that the existing example tests cannot — a property over an input space, on the code path that carries the scanner's escaped defects?

## Outcomes (defined before the run)

- **success** — all success criteria below are met.
- **inconclusive** — no kill criterion fires, but one or more success criteria are not met. The report states which, with the produced/verified/carrier table.
- **fail** — any kill criterion fires.

There is no "partial success". A split result is reported as `inconclusive`, not re-labelled.

## Carrier taxonomy

Every harness and every mutant catch is labelled:

| Label | Meaning |
|---|---|
| `P` | Symbolic proof over an input space (Kani) |
| `B` | Bounded symbolic proof (Kani, declared bound) |
| `C` | Concrete Kani battery (finite constants; not a proof over inputs) |
| `T` | Native `cargo test` |
| `F` | Bolero/fuzz sample (sampling, not proof) |

A headline count must never mix carriers without the split.

## Declared harness set (success-bearing)

| # | Harness | Carrier | Property |
|---|---|---|---|
| 1 | `tier1_placeholder_symbolic` | `B` | No panic for any ≤8-byte string over the declared alphabet; known placeholder positives still hold |
| 2 | `tier1_jwt_segments_symbolic` | `B` | `find_jwt_segments` returns `Some` iff exactly two dots; the three parts concatenate to the input; no part contains a dot; no panic |
| 3 | `tier1_base64_roundtrip_symbolic` | `B` | For every fixed 6-byte array, `base64_decode(encode_ref(x)) == Some(x)`; no panic |
| 4 | `tier1_base64_invalid_symbolic` | `B` | Any bounded string with a character outside the alphabet (after padding trim) decodes to `None`; no panic |
| 5 | `tier1_entropy_symbolic` | `P` | With `shannon_entropy_bits_per_char` stubbed to a symbolic `f64`, `meets_high_entropy` returns exactly `e >= 4.2` for a token meeting the length/class preconditions (registered T3) |
| 6 | `tier2_depth_symbolic` | `B` | For all `d`, `d > MAX_DEPTH` rejects with `max_depth_exceeded`; `d <= MAX_DEPTH` on `Null` does not reject for depth |
| 7 | `tier2_key_scan_symbolic` | `B` | A symbolic clean key does not reject; a concrete marker key does reject (M3 property) |
| 8 | `tier3_scan_str_concrete` | `C` | The assembled scanner rejects a GitHub PAT, an AWS key, a JWT, and a metadata value; accepts a clean document (Phase 3, Kani `main`) |
| 9 | `tier3_provider_form_concrete` | `C` | `validate_provider_form` accepts tokens at and above `min_len` and rejects one byte below; AKIA suffix shape (Phase 3, Kani `main`) |
| 10 | `tier3_scan_str_symbolic_ghp` | `B` (attempt) | Bounded input ≤16 bytes over the declared alphabet: if it contains `ghp_` plus a valid suffix, `scan_str` rejects. This is an empirical attempt, not a success gate |

The 13 v1 harnesses are re-run as **calibration only** (`C`); they do not count toward success.

## Input envelope (T2)

- Strings ≤ 128 bytes; for symbolic harnesses, the per-harness bound is 6–16 bytes as stated in the harness.
- JSON depth ≤ 4; collections ≤ 8.
- Production caps (`MAX_SCAN_BYTES` 256,000; depth 8) are outside the proven envelope and remain covered by example tests. No proof claim at production bounds.

## Trusted-annotation register (v2)

- **T1** — the two `OnceLock` immutable statics (scanner singleton; base64 table). Harnesses initialize concretely or avoid the singleton. No unregistered assume about contents.
- **T2** — the input envelope above.
- **T3** — the entropy computation. `tier1_entropy_symbolic` stubs `shannon_entropy_bits_per_char` to a symbolic `f64`; the threshold decision is proved modulo the entropy value. The computation itself is not proved (the `HashMap` seed is nondeterministic under Kani).
- **T4** — the regex extraction seam. Kani 0.68 ICEs on `regex-automata` 0.4.18 (#5024). Phase 3 uses Kani `main` after #5066. If the fix does not compile the scanner, extraction remains outside proof scope and the report says so; no unregistered stub.
- **T5** — the `scan_inner` oracle for Tier 2: rejects iff the scanned text contains the planted `KANI_MARKER`, by a manual byte scan. Covers `scan_value_with` traversal only, not detection quality.
- **T6** — **retired.** v1's fabricated zeroed `SecretScanner` receiver is removed by the free-function refactor; Tier 2 calls `scan_value_with` with the oracle closure and no receiver. If the refactor is rejected, T6 returns with an explicit soundness argument before use.

An entry is added before the trusted item is relied on. An unregistered stub/assume is kill criterion 4.

## Mutants (pre-registered, exact patches logged)

Each mutant is applied alone, the catching harness is run, the mutation is reverted, and `git status` is checked. The exact patch is recorded in the run log.

| ID | Mutation | Direction | Required carrier |
|---|---|---|---|
| M1′ | `validate_provider_form` returns `None` for a valid `ghp_` token at `min_len` (a known secret is no longer recognised) | false negative | `C` or `P` (function-level); end-to-end `C` preferred |
| M2′ | `find_jwt_segments` returns `Some` for 2- or 4-segment strings | boundary / false positive | `B` |
| M3′ | `scan_value_with` skips metadata **values** (not keys) | false negative | `B` or `C` |
| M4′ | `meets_high_entropy` threshold `>= 4.2` → `> 4.3` | false negative | `P` |
| M5′ | `is_placeholder` exempts any value containing `changeme` anywhere, including `xchangeme` | false positive / over-exemption | `B` or `C` |

At least three catches must be `P`/`B`, including at least one false-negative mutant. A surviving mutant after one repair cycle kills the proof claim.

## Success criteria (all required)

1. Refactor-in-place: the harnesses call the shipped functions; no verified copy. The Tier 2 free-function refactor is the only production edit, and the full existing suite stays green (documented baseline failures excepted and attributed).
2. At least **three** of harnesses 1–7 verify with `VERIFICATION SUCCESSFUL`, zero failed checks, and no unwinding-assertion truncation. A timeout is not a pass.
3. At least **one** proof-carried (`P`/`B`) false-negative mutant (M1′/M3′/M4′).
4. Phase 3: Kani `main` compiles the regex seam and the concrete end-to-end scanner harnesses (8–9) verify, **or** the report states plainly that #5066 did not make the detection core tractable and the detection claim is withdrawn.
5. The CI lane is executed on `x86_64-unknown-linux-gnu` and measured within 15 minutes, and it fails when a declared harness does not verify.
6. Reporting: produced/verified/excluded split, carrier labels, architecture, exact toolchain, wall-clock, peak memory, and the full register. No headline number without its split.

## Kill criteria (any one)

1. No harness in 1–7 verifies symbolically after the time-box, and no explained toolchain cause is available.
2. Kani `main` still cannot compile the regex seam, and no registered model seam is acceptable to the owner.
3. The CI ceiling is breached and not renegotiated before the claim.
4. The subject file changes materially mid-run (new census basis), invalidating the baseline.
5. Success would require an unregistered `assume`/stub or the unstable contracts path.

## Ceilings and time-box

- Verified core ≤ 1,200 production LOC (the subject core is ~950).
- ≤ 40 harnesses total; declared set 10.
- 9 working days of effort (Phase 0–6).
- CI advisory lane ≤ 15 minutes; renegotiation must be written before the claim.

## Architecture and scope

- Proofs are per-target. Local results are `aarch64-apple-darwin`; the canonical claim is `x86_64-unknown-linux-gnu` on CI. The report never presents an arm64 result as a Linux result.
- Wider Kani adoption, promotion to a required check, and other subjects are out of scope.

— Frozen 2026-11-18, before Phase 1.

---

## Amendment 1 — 2026-11-18 (after Phase 1, before the symbolic run is judged)

The frozen set was written before the capacity run. Phase 1 produced three
findings that change the *carrier* assignment, not the question or the
success bar:

1. **JWT/placeholder symbolic forms are not tractable on this host.** The JWT
   spec and panic-freedom harnesses still exceed 600 s at N=4 over `{a,b,.}`;
   the placeholder symbolic form exceeds 600 s at N=8. The pilot's rung-1 and
   rung-2a findings reproduce on a 16-core/64 GB M3 Max, so this is a CBMC
   formulation cost, not RAM. They remain in the file as produced artifacts
   and are reported as not verified; they do not count toward success.
2. **Base64 symbolic forms are blocked by the host std, not by the property.**
   Kani 0.68 ICEs on `catch_unwind` with `OnceLock` reachable; Kani `main`
   compiles it but fails on unsupported `ptr_mask`. Both symbolic base64
   harnesses remain in the file, are not counted locally, and are deferred to
   the x86_64 Linux lane.
3. **Harness 7 is split into 7a and 7b.** `tier2_string_leaf_symbolic` (7a,
   `B`) proves every `Value::String` leaf reaches the scanner; the concrete
   `tier2_key_scan` (7b, `C`) covers object keys. The symbolic key form is
   retained but not counted because the concrete map traversal already
   exceeds 15 minutes at 12 GB.

The success bar remains: at least three `P`/`B` harnesses, at least one
proof-carried false-negative mutant, Kani `main` tried for the detection core
and the result reported, and the CI lane measured. The Phase 1 calibration
counts nothing toward success.

## Amendment 2 — 2026-11-18 (Kani `main` result)

Kani `main` at `faffcee` (post-#5066) clears the #5024 regex ICE but hits the
open #5101 `AnonConst`/thread-local class at
`regex_automata::util::pool::inner::THREAD_ID` (and, on the minimal probe,
`std::hash::RandomState::new`). The documented `RandomState::new` zero-seed
stub removes the first hit; the regex `THREAD_ID` hit remains. The detection
core is therefore **not reached** on this host/toolchain. Per the frozen
terms and the owner's recommendation, the rerun does not build a model seam
as a rescue; the detection claim is withdrawn and the negative result is
reported.

— Amended 2026-11-18, before the symbolic success ruling.

## Amendment 3 — 2026-11-18 (CI ceiling breach)

The first run of `kani-v2.yml` on PR #113 was cancelled by its 15-minute job
timeout. The log shows the job reached `Run declared harnesses` and then
`Checking harness ... tier1_placeholder_battery`; no harness verdict was
produced. The cause is cold Kani codegen for the 463-package crate on a
2-core `ubuntu-latest` runner, not a harness failure. The 15-minute number
in the frozen terms is therefore **not met** on the first cold run.

The full advisory lane is renegotiated to a **45-minute** job timeout, and
the breach is reported as such. A cached fast PR lane (or a larger runner)
is future work and is not claimed. The local 777 s v1 sum was never a
representative CI measurement; this run is.

— Amended 2026-11-18.
