# Kani Pilot — Stage 1 Execution Log

**Started:** 2026-11-17 (Stage 1 authorized by the owner as pre-registered).
**Governing documents:** `pre-registration.md` (frozen), `g2-bridge-design.md`, `stage1-harness-plan.md` — all in this directory.
**Worktree:** `<pilot-workspace>/work/neurostrata`, branch `feat/kani-secrets-scanner-pilot` @ `dd1affef` (v1.18.1), clean at start.
**Toolchain:** Kani 0.68.0 (cargo plugin), CBMC 6.11.0, cargo 1.100.0. First `cargo kani --version` triggered a one-time rustup auto-install of toolchain `1.100.0-x86_64-unknown-linux-gnu`.
**Machine:** ~7.9 GB RAM total (~1 GB available at start; a NeuroStrata daemon and the runtime hold the rest), ~1.5 effective CPUs. One Kani run at a time; every run under `timeout`; timings recorded per run.
**Crate shape:** binary-only crate `neurostrata-mcp` (`mod secrets;` in `src/main.rs`), 463 lockfile packages. Full-crate Kani codegen cost is the ladder's first unknown.

Entries are appended as work lands. Verdicts and error text are recorded verbatim.

---

**Run 5 (v4 generator, rung 1 proper)** — `rung1_is_placeholder_never_panics` (symbolic, N=16): **TIMEOUT at 600 s, no verdict.** CBMC stayed in a healthy profile (~2 GB, no thrash) but did not finish: the function's composed internals (trim's whitespace searcher, four TwoWay `contains` searches, two `chars().all()` scans, nine pattern equalities over symbolic characters) make a hard SAT instance at this bound on 1.5 CPUs. This is a per-function price, not a toolchain failure. Consequences, in order: the concrete placeholder battery runs next (the M5 property is concrete by nature); the symbolic placeholder harness is retried at N=8; if that also times out, the placeholder properties stand as concrete Kani harnesses + bolero sampling, recorded in the produced/verified split — not silently dropped.

**Rung 2a, first form** — `rung2_find_jwt_segments` (symbolic, N=12, alphabet {a,b,.}): **TIMEOUT at 600 s.** Two harness-style defects compound the production cost: the assertion used `format!` and `assert_eq!` on `String`s, which drags `core::fmt` into the reachable set on the failure path (a known Kani cost trap), and N=12 leaves a large dot-placement space over `split`'s memchr search. Harness discipline adopted for the whole set from here: assertions are manual byte loops and `assert!` on booleans only — no `format!`, no `assert_eq!` on heap values — and string bounds shrink to the smallest size that exercises the property (N=8 for segment logic). The concrete placeholder battery, by contrast, **VERIFIED in 121 s** (run above), so the M5 property already has a passing Kani form.

**Rung 2a, exhaustive form** — all 364 strings over {a, b, .} of length ≤ 5 in one harness: **TIMEOUT at 600 s**, still in symex at the cut (the log's tail is CBMC unwinding the property's own assertion loops inside the 364-case enumeration). Per-case cost is dominated by inlining `str::split`'s searcher machinery hundreds of times into one formula. Disposition for the full set: exhaustive enumeration only up to length 3 (40 cases, every dot-count and emptiness shape for short strings) plus a designed concrete family for lengths 4–8 (3-segment acceptance with longer parts, 4-segment rejection, empty leading/middle/trailing segments, the all-empty `..` case) — the family is mutant-adequate for M2 by construction, and the split is stated in the produced/verified report rather than smoothed over. The general property continues to be sampled by the bolero driver under `cargo test`.

**Rung 2b, round-trip, first run** — **VERIFICATION FAILED at 102 s, on exactly 1 of 3,991 checks**: `secrets::base64_decode::{closure#0}.unwind.0` — an unwinding-assertion failure in the OnceLock table-build closure (64 alphabet iterations over the harness's `#[kani::unwind(32)]`). **Correction to the first reading of this result:** with the bound at 32, every path through the 64-iteration closure was cut at the unwinding assertion, so the round-trip equality downstream was in all likelihood vacuous/undetermined rather than proven — an under-bounded harness must never be read as "property passed, bound failed". The bound was raised to 96 and the harness re-run — which **timed out at 600 s**: with the closure allowed to complete, the formula (OnceLock initialization machinery + symbolic-length `Vec` handling in the decode path) exceeds this machine's budget at that bound. The unwinding-assertion discipline did its job in both directions: it caught the under-bound loudly, and it prevents the truncated run from being quoted as a verification. Next experiments: a concrete base64 battery (isolates the OnceLock path from symbolic-length effects), then symbolic with a fixed length.

**Rung 2b disposition (after four symbolic attempts):**
- Concrete battery (`rung2_base64_concrete`, 9 decode calls incl. padding-optional and invalid-char cases): **VERIFICATION SUCCESSFUL, 132 s.** The OnceLock (T1) path is not the problem.
- Symbolic round-trip, variable length ≤ 9 (unwind 96): TIMEOUT 600 s. Symbolic round-trip, fixed length 6: TIMEOUT 600 s (symex completes; the SAT instance — symbolic bytes through the encoder, the 128-entry table lookup by symbolic index, and the bit-packing loop — does not solve on 1.5 CPUs). Symbolic invalid-char (unwind 96): TIMEOUT 600 s.
- Conclusion: `base64_decode` is verified **concretely** under Kani; its round-trip and rejection properties are carried by the bolero drivers under `cargo test` (sampling, honestly labelled) — recorded in the produced/verified split. A length-3 symbolic attempt remains possible gravy, not a gate.

## Ladder rung 5 — entropy, concrete (`rung5_entropy_concrete`)

**TIMEOUT at 600 s.** Even fully concrete inputs do not verify: `shannon_entropy_bits_per_char` builds a `HashMap<char, usize>` whose `RandomState` seed comes from OS randomness, which Kani models nondeterministically — every insert/lookup then branches over symbolic hash values, and the f64 `log2` accumulation prices on top. Conclusion (the T3 case the register anticipated): the entropy *value* is not directly provable on this machine; the M4-relevant property is the threshold *decision*, to be proved against a registered stub of `shannon_entropy_bits_per_char` returning a symbolic value bracketing 4.2 (registered under T3 before use; the resulting property is reported verified-modulo-T3). `#[kani::stub]` feasibility in 0.68 is probed as part of the harness build-out.

## Ladder rung 3 — the regex question (`rung3_regex_trivial`)

**Kani compiler ICE at 89 s** (codegen phase; no verification attempted). Verbatim core of the error:

```
thread 'rustc' panicked at kani-compiler/src/codegen_cprover_gotoc/codegen/rvalue.rs:1009:78:
called `Result::unwrap()` on an `Err` value: TryFromIntError(PosOverflow)
[Kani] current codegen item: codegen_function: regex_automata::meta::strategy::new
[Kani] current codegen location: .../regex-automata-0.4.18/src/meta/strategy.rs, start_line: 81
```

Any harness whose reachable set includes `Regex::new` (or matching over a compiled `Regex`) cannot even be compiled by Kani 0.68.0 against regex-automata 0.4.18. Consequences, all recorded rather than worked around silently:

- Rungs 3b/3c/4 are answered by this result: `SecretScanner::build()`, the AKIA branch of `validate_provider_form` (its body contains `Regex::new`, so the whole function is un-codegennable), `scan_str`, and `scan_inner` are all outside Kani's reach on this toolchain. Rung 4 gets one confirming run for first-hand evidence; 3b/3c would repeat the identical codegen path.
- This is the harness plan's AKIA amendment trigger in stronger form (whole-crate ICE rather than one un-executable call) — filed in the register as T4-S1 and reported to the owner as amendment-relevant. Per the plan, the pilot does not improvise a fallback; Tier 1 shrinks to the regex-free functions, and Tier 2 proceeds only if the T5 stub keeps regex codegen out of `scan_value_inner`'s reachable set (probe below).
- `validate_provider_form` (M1's mutant site) therefore cannot be Kani-proved at all: its non-AKIA logic is inseparable from the AKIA `Regex::new` in one function body. M1's catch must come from the bolero/example-test side, and the produced/verified split will say so plainly. (Whether that dents the pre-registered success criteria is the owner's call on the report, not something Stage 1 redefines mid-flight.)

## Tier 2 probe sequence (stub T5 + fabricated receiver T6)

- Probe v1 (no `-Z stubbing`): compile error — in Kani 0.68 `#[kani::stub]` requires the unstable `stubbing` feature, enabled as `cargo kani -Z stubbing`. Noted in T5; the CI lane uses the same flag.
- Probe v2 (`-Z stubbing`, `mem::zeroed` receiver, all three scenarios in one harness): compiled — **the stub does prune regex codegen; no ICE** — and the run ended FAILED at 94 s on exactly 1 check: Kani's safety check on `mem::zeroed`'s `write_bytes`. (Interpretation corrected later: Kani runs a safety pass first, so v2's speed was the safety pass failing early, not a completed property verification.)
- Probe v3 (`MaybeUninit` receiver): TIMEOUT 600 s (nondeterministic receiver state). A re-run after a runtime restart also timed out.
- Probe v4 (aligned byte-buffer receiver): symex blow-up (memchr_naive unwound 1,331×, 4.3 GB), operator-killed.
- Probe v5 (transmute-from-zero-words receiver, combined harness): same symex blow-up (memchr_naive 1,204×), operator-killed. Conclusion: the blow-up is intrinsic to the combined harness's reachable set (serde_json BTreeMap + String/`format!` machinery), not to the receiver device.
- **Decomposed probes (final form):**
  - `tier2_probe_depth_fail_closed`: **VERIFICATION SUCCESSFUL, 72 s, 0 failed checks.** Tier 2 is feasible.
  - `tier2_probe_key_scan` (M3 property): **TIMEOUT at 600 s** with the default solver (symex complete, ~3.9 GB), and again symex-bound under `--solver kissat` (memchr_naive at 1,682 unwinds and climbing when killed) — the solver is not the bottleneck; symbolic execution of the serde_json/String machinery is. Disposition: the key-scan property is **produced but not Kani-verified**; it is carried by the cargo-test drivers using the real scanner (where regex executes natively), and the M3 mutant check runs against those drivers. Stated in the produced/verified split.
  - `tier2_probe_clean_object`: removed from the set unrun — same machinery as key-scan; its assertion is covered by the test-side drivers.

## Incident: over-broad `cargo fmt` (recovered)

During harness-set curation, a `cargo fmt` invocation reformatted the entire worktree (143 files) instead of only `src/secrets.rs`. The three pilot files (`src/secrets.rs`, `Cargo.toml`, `Cargo.lock`) were copied out, the whole tree was reverted with `git checkout -- .`, and the three files restored; `git status` confirms the diff is again exactly those three files. The formatting of `src/secrets.rs` itself (the intended target) is retained. No other file's content survives from the incident.

## Canonical harness results (final shipped set, this machine)

Verified (`VERIFICATION SUCCESSFUL`, 0 failed checks), canonical runs in `state/runs/final-*.log`:

| Harness | Time | Notes |
|---|---|---|
| `tier1_placeholder_battery` | 99 s | M5 property (concrete form) |
| `tier1_base64_battery` | 111 s | T1 OnceLock path included |
| `tier1_exempt_code_expression` | 88 s | |
| `tier1_exempt_source_reference` | 113 s | |
| `tier1_exempt_type_names` | 104 s | |
| `tier1_entropy_at_threshold` | 66 s | modulo T3 (stub, T3-S1) |
| `tier1_entropy_below_threshold` | 64 s | modulo T3 |
| `tier1_entropy_above_threshold` | 65 s | modulo T3 |
| `tier2_depth_fail_closed` | 67 s | modulo T5/T6 |

Sum of verified runtimes: 777 s sequential on this box (1.5 shared CPUs, ~8 GB shared RAM) — relevant to the CI lane's 15-minute ceiling; the lane is advisory and GitHub runners are substantially stronger, but the ceiling is a recorded risk, not a formality.

Produced but NOT verified (kept in the file, documented in situ; the CI lane enumerates only the verified nine):

| Harness | Outcome |
|---|---|
| `tier2_key_scan` (M3) | TIMEOUT 600 s (default solver) and symex-bound under kissat |
| `tier1_jwt_segments_some` / `tier1_jwt_segments_none` (M2) | CBMC out of memory (309 s / 416 s), 0 failed checks |
| `tier1_exempt_member_chain` | CBMC out of memory in 8-assertion and both 4-assertion split forms (275–469 s), 0 failed checks |

Root-cause summary across all failures: no produced harness ever failed a property check. The binding constraints on this machine are (1) the `regex` crate ICE (whole seam out of scope), (2) CBMC memory in `str::split`-iterator machinery (JWT segments, member chain, Tier 2 object traversal), (3) symbolic-content SAT cost (base64 round-trip), (4) nondeterministic HashMap seeds (entropy values). Every not-verified property is carried by an executable native carrier: the bolero drivers, `jwt_segments_cases`, `exemption_predicates_cases`, `provider_form_length_boundaries`, and the pre-existing suite (incl. `metadata_key_carrying_a_secret_is_detected` for M3).

## Mutant check (M1–M5) — ALL FIVE CAUGHT, no repair cycle needed

Each mutant was applied alone, its catching harness(es) run, and the mutation then reverted completely; the tree was verified back to the harness-only diff after the series (`git status`: only `src/secrets.rs`, `Cargo.toml`, `Cargo.lock` modified).

| Mutant | Edit | Catching harness / test | Result |
|---|---|---|---|
| M1 | `validate_provider_form`: `candidate.len() < form.min_len` → `<=` | `provider_form_length_boundaries` (cargo test) | FAILED as required, 36 s |
| M2 | `find_jwt_segments`: delete the too-many-dots rejection | `jwt_segments_cases` + `jwt_segments_property_sampled` (cargo test) | both FAILED as required, 54 s |
| M3 | `scan_value_inner`: delete the metadata-key scan branch | `metadata_key_carrying_a_secret_is_detected` (pre-existing test) | FAILED as required, 46 s |
| M4 | `meets_high_entropy`: `h >= 4.2` → `h > 4.2` | `tier1_entropy_at_threshold` (Kani) | VERIFICATION FAILED, exactly 1 failing check, 68 s |
| M5 | `is_placeholder`: pattern exemption gated on `trimmed.len() > 64` (never true for registered patterns) | `tier1_placeholder_battery` (Kani) | VERIFICATION FAILED, exactly 1 failing check, 69 s |

Notes: M2's and M3's Kani harnesses are in the produced-not-verified set (split/serde_json machine limits), so their catches ran on the native carriers — the same property functions and assertions, executed natively. M1 likewise (its function is above the regex seam). Kill criterion 5 (a mutant surviving after one repair cycle) did not fire; no repair cycles were needed at all.

## Formatting note

The pristine `src/secrets.rs` at `dd1affef` is not rustfmt-clean (verified: `rustfmt --check` on the HEAD version exits 1). The pilot's diff therefore includes 22 whitespace-only reflowed lines in pre-existing code alongside the additive harness modules, and `rustfmt --check` on the changed file passes. No production behaviour change: the full test suite result is recorded below.

## Zero-behaviour-change evidence

- `cargo test --bin neurostrata-mcp secrets::` — **59 passed, 0 failed** (52 pre-existing + 7 pilot-added: 3 bolero property drivers, `provider_form_length_boundaries`, `jwt_header_validation_cases`, `exemption_predicates_cases`, `jwt_segments_cases`).
- Full bin suite (`cargo test --bin neurostrata-mcp`, 2,064 tests): **2,052 passed, 8 failed, 4 ignored** in 351 s. Attribution, run first-hand: the same 8 tests were run on the pristine `dd1affef` tree in the identical environment — 7 fail there too (embedder fallback, GLiNER fake bundle, 4× Laya ONNX, trainer ONNX bundle: the ONNX/model class, consistent with this machine's local ONNX Runtime 1.28 substitution for the build's expected prebuilt and absent model assets). The 8th (`events::subscribers::export_freshness_dirty::the_flag_is_scoped_to_one_namespace_and_one_flip_stays_one_row`) passes in isolation on BOTH trees and failed only inside the parallel full-suite run — a load flake, not a pilot effect (the pilot diff touches only `src/secrets.rs`, dev-dependencies, and a workflow file).
- `rustfmt --edition 2021 --check src/secrets.rs` — passes.
- Environment note: all cargo invocations in this worktree require `ORT_LIB_LOCATION=<local onnxruntime 1.28.0>/lib` + `ORT_PREFER_DYNAMIC_LINK=1` (the ort-sys build script's prebuilt download is blocked here; the hatch was found by reading the build script — see the ort entries earlier in this log). The CI lane needs neither variable.

## Ladder rung 1 — trivial harness on `is_placeholder` (toolchain + cfg(kani) wiring)

Setup edits (harness-only, permitted class):
- `Cargo.toml`: a declared `kani = "0.68"` dependency does not resolve — crates.io only carries `kani` 0.0.x; the verifier injects the `kani` crate through its compiler driver, so no manifest entry is needed (or possible). Removed again. `bolero = "0.13"` added under `[dev-dependencies]`; resolved and fetched cleanly as bolero 0.13.4.
- `src/secrets.rs`: appended a `#[cfg(kani)] mod kani_proofs` with the symbolic-string helper and the rung-1 harness.

**Run 1** — `timeout 1800 cargo kani --harness rung1_is_placeholder_never_panics`: **FAILED at 401 s, before any verification.** Cargo exited 101 during dependency codegen:

```
error: ort-sys@2.0.0-rc.15: ort-sys failed to download prebuilt binaries from `https://cdn.pyke.io/0/pyke:ort-rs/ms@1.31.0/x86_64-unknown-linux-gnu.tar.lzma2`: http status: 403
error: build script logged errors
error: Failed to execute cargo (exit status: 101). Found 0 compilation errors.
```

`ort-sys` (via fastembed) is an unrelated dependency whose build script downloads prebuilt ONNX Runtime binaries; the CDN answers 403 in this environment. The Kani target dir (`target/kani`) is separate, so the download is attempted fresh. This is an environment blocker for full-crate codegen, not a property of the subject code. Investigation: locate a local `libonnxruntime` (ort-sys honours `ORT_LIB_LOCATION` and then skips the download).

**Run 2** — same command + `ORT_LIB_LOCATION=~/workspace/neurostrata/worktrees/upstream-fixes-a/target/debug`: **FAILED at 8 s** — `ort-sys could not link to the ONNX Runtime build in ...`. Reading the build script (`build/main.rs`, `build/vars.rs`, `build/dynamic_link.rs` of ort-sys 2.0.0-rc.15) shows why: with a system lib path set, it attempts *static* linking unless `ORT_PREFER_DYNAMIC_LINK=1` is set; the local dist ships only shared libraries. With dynamic linking preferred, the script emits link directives and returns success without a probe. A full local ORT 1.28.0 dist exists at `~/workspace/plans/ort-dist/onnxruntime-linux-x64-1.28.0/lib` (the local tree's rc.13-era runtime; the version gap vs rc.15's 1.31 prebuilt is immaterial here — the native library is never executed by Kani, and the harness reachability set never enters ort code). All later cargo invocations in this worktree (Kani and test alike) therefore carry `ORT_LIB_LOCATION=~/workspace/plans/ort-dist/onnxruntime-linux-x64-1.28.0/lib ORT_PREFER_DYNAMIC_LINK=1`. This is a local-environment workaround only; the CI lane runs where the prebuilt download works and needs neither variable.

**Run 4 (v2 generator)** — harness rewritten to alphabet-restricted generation (`any_str`: per-byte `kani::any_where` over a 20-byte alphabet, `String::push` loop, ≤16 bytes). Codegen was cached; CBMC reached 4.5 GB RSS in disk-wait (swap thrash) within ~4 minutes with no verdict; the run was killed by the operator to protect the shared machine. The bottleneck is therefore not UTF-8 validation alone. Bisect probes added (trivial assertion / generator-only / concrete-only `is_placeholder`) to isolate whether the cost sits in the generator, in the function's std-string internals (`str::contains` uses the TwoWay searcher, a known CBMC heavyweight), or in baseline crate GOTO size.

**Bisect results:**
- `probe_trivial` (one symbolic `u8`, no strings): **VERIFICATION SUCCESSFUL, 82 s end-to-end** including top-crate re-codegen. Baseline toolchain cost is healthy.
- `probe_gen_only` (the v2 push-loop generator alone, no production call): CBMC passed 4.3 GB RSS at ~3 min and still climbing; killed by the operator. **The generator is the bottleneck** — Kani's Vec model under a symbolic-length push loop is the explosion, not the subject code.
- Generator rewritten (v3): one fixed `[u8; N]` symbolic array, `kani::assume(alphabet.contains(b))` per byte, one `to_vec` allocation, `from_utf8_unchecked` (harness-local safety argument: alphabet bytes are ASCII). `probe_placeholder_concrete` (concrete battery over `is_placeholder`) still to run to price the function's own internals.
- v3 also exploded (CBMC 5.4 GB, killed). The v3 log's unwind histogram identified the culprit precisely: `core::slice::memchr::memchr_naive` unwound 1,575 times and `memchr_aligned` 819 times — `<[u8]>::contains` lowers to memchr, and sixteen symbolic membership searches compounded past the machine's means.
- Generator v4 (manual membership loop over the alphabet, no `contains`): **`probe_gen_only` VERIFICATION SUCCESSFUL, 85 s** including codegen. Generator problem solved. Standing warning for the harness set: production functions that call memchr/TwoWay-backed search (`str::contains`, `slice::contains`) may price similarly per call — each Tier 1 harness is therefore also a per-function price probe, and the ladder order below surfaces that before the full set is committed to.

**Run 3** — with both env vars: dependency codegen completed (167 crates), the top crate codegen'd with `--reachability=harnesses`, and CBMC began verifying `secrets::kani_proofs::rung1_is_placeholder_never_panics`. **No verdict was recorded:** CBMC was still solving at 1,137 s elapsed (RSS ~3.2 GB) when the runtime restarted and killed the run; the run's `/tmp` log was lost in the restart (the facts above were transcribed from live observation before it). Rung 1's split verdict: **toolchain + cfg(kani) wiring PROVEN** (full-crate codegen, harness discovery, GOTO generation, CBMC solving all observed); **the naive input model REJECTED on cost** — fully-symbolic UTF-8 strings (arbitrary bytes + `from_utf8` validation) do not solve within this machine's means, and the pre-registered CI ceiling is 15 minutes for the whole lane. The harness generator is therefore switched to the standard remedy — symbolic strings over a small alphabet of structurally significant bytes — and rung 1 is re-run in that form. This narrows the panic-freedom claim to alphabet strings; multibyte/UTF-8 behaviour remains covered by the existing example tests and the bolero drivers (stated here so the narrowing is on record, not silent).
