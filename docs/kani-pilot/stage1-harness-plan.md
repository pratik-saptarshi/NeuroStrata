# Kani Pilot Stage 1 — Harness Plan (from the T4 inventory)

**Written:** 2026-11-17, Stage 1 entry, from a first-hand read of `src/secrets.rs` (1,415 lines) at `dd1affef`.
**Governing documents:** `pre-registration.md` (frozen) and `g2-bridge-design.md` in this directory. Any deviation from either is an amendment, appended and dated, never a silent edit.

## T4 inventory result (pattern-engine and trusted seams)

- **Pattern engine = the `regex` crate.** Four compiled regexes (`provider_re`, `kv_re`, `jwt_re`, `he_re`) built in `SecretScanner::build()`, used via `find_iter`/`captures_iter` in `scan_inner`/`detect_jwt`, plus one ad-hoc `Regex::new("^[0-9A-Z]{16}$")` inside `validate_provider_form` (AKIA suffix). Kani cannot model the regex matching engine → candidate extraction is the pilot's principal trusted seam (register T4, resolved below).
- **serde_json** is used in `is_valid_jwt_header` (parse of the decoded header) and throughout `scan_value_inner` (the `Value` tree). Parsing under Kani is heavy; bounded tiny inputs only, or the JSON-interpretation step is modelled (see harness tiers).
- **HashMap + f64 `log2`** in `shannon_entropy_bits_per_char` / `meets_high_entropy` (register T3 territory).
- **OnceLock statics (T1):** the scanner singleton (line ~322) and the base64 decode table inside `base64_decode`. Both immutable lookup data.
- Everything else in the file is plain synchronous Rust over strings/slices — the provable decision core.

## Harness architecture (two tiers)

**Tier 1 — direct proofs over pure decision functions** (no scanner instance, no regex matching):
- `find_jwt_segments`: exactly-three-segments iff `Some`; segment contents are the dot-split parts (panic-freedom + boundary exactness). *Catches M2.*
- `base64_decode`: round-trip against a reference encoder for bounded inputs; invalid characters → `None`; padding handling; never panics. (The census's escaped panic was a UTF-8 slice panic in JWT handling — panic-freedom of the JWT path is a headline property.)
- `validate_provider_form`: for symbolic candidates over a bounded alphabet/length — a candidate is accepted iff it starts with a registered prefix (case-insensitive) and meets that form's `min_len`; category mapping is exact. AKIA path reaches the ad-hoc `Regex::new` — handled per the proof-of-life ladder result (assume-exclusion of the AKIA branch in the symbolic harness + a dedicated AKIA harness; if Kani cannot execute the ad-hoc regex compile at all, this becomes an amendment decision, reported before proceeding). *Catches M1.*
- `is_placeholder`: the registered placeholder patterns and shapes scan clean; near-miss strings (placeholder as substring of a longer token) are NOT exempt. *Catches M5 (predicate-level).*
- `looks_like_member_chain`, `looks_like_code_expression`, `looks_like_source_reference`, `looks_like_type_annotation`, `is_known_type_name`: exemption predicates match their documented shapes and reject password-shaped counterexamples (e.g. `p@ss.w0rd` is not a member chain).
- `meets_high_entropy` / `shannon_entropy_bits_per_char`: threshold and class requirements on constructed token families, including boundary cases. *Catches M4* (concrete boundary tokens are acceptable for mutant-catching; symbolic coverage attempted first).

**Tier 2 — structural proofs with the extraction seam modelled** (registered oracle):
- `scan_value_inner` structure: keys are scanned as well as values (the M3 property), depth > MAX_DEPTH fails closed, oversized string leaves get their distinct category. Model: `scan_inner` replaced under cfg(kani) by an oracle that rejects iff the input contains a planted marker substring — the proof then covers recursion/key/depth structure, not matcher internals. *Catches M3.* The oracle is a new register entry (T5) filed before use.
- `scan_chunked` structure (if feasible): termination and no-gap coverage of chunk boundaries with `scan_inner` as a nondeterministic oracle.

**Not claimed:** extraction completeness of the four regexes (that every secret-shaped string in a text is found as a candidate) — that property lives in the regex engine and the wiring, rests on the existing example tests, and will be stated as such in the result report, beside the produced/verified split.

## Proof-of-life ladder (run in this order when the toolchain lands)

1. Trivial harness on `is_placeholder` (toolchain + cfg(kani) wiring proof).
2. `find_jwt_segments` + `base64_decode` symbolic harnesses.
3. `validate_provider_form` symbolic harness — learn what the AKIA ad-hoc `Regex::new` does under Kani.
4. `SecretScanner::scan_str` on a short concrete string — learn whether regex compile+match over concrete input is survivable; if yes, selected end-to-end harnesses become possible and Tier 2 shrinks.
5. `meets_high_entropy` concrete boundary harness — learn the float/HashMap cost.

Each rung's result is recorded in `state/stage1-log.md` as it lands. A rung that fails decides the architecture above; it does not silently shrink the claims — the result report carries whatever split the ladder produces.

## Mutant → harness map (pre-registered mutants, pre-registration.md)

| Mutant | Site | Catching harness tier |
|---|---|---|
| M1 provider predicate inversion | `validate_provider_form` | Tier 1 |
| M2 JWT boundary off-by-one | `find_jwt_segments` | Tier 1 |
| M3 metadata-key branch deletion | `scan_value_inner` | Tier 2 (oracle) |
| M4 entropy threshold `>=` → `>` | `meets_high_entropy` | Tier 1 |
| M5 placeholder exemption removal | `is_placeholder` | Tier 1 |

## Register additions anticipated

- **T4 (resolved):** regex candidate extraction is a trusted seam; decision logic after extraction is what is proven; extraction completeness is not claimed.
- **T5 (to file before Tier 2 runs):** `scan_inner` oracle model for structural harnesses — rejects iff input contains the planted marker; bounds stated in the harness source.

— Musebot-Astro, Stage 1 entry, 2026-11-17.
