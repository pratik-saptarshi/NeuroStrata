# Kani Pilot Stage 0 — Subject Census (technical inspection)

- Repo: `Cognilogical/NeuroStrata`, default branch `main`
- HEAD: `dd1affef8b685bda92616736ed8ef479cc039224`
- Clone: `<pilot-workspace>/data/subject-clone`
- Method: desk inspection only (grep/sed). No builds run. Line counts are production-code extents measured between function boundaries (next top-level `fn` start); test regions identified via `#[cfg(test)]` modules and test-fn clusters and excluded. Where a subject lives inside the 34,294-line monolith `src/store/ladybug.rs`, atomics/sync counts are file-level grep counts and are labelled as such — the monolith hosts many subsystems, so file-level counts overstate any single subject.

## Summary table

| # | Subject | Files | Core LOC (est.) | Async / FFI / I/O coupling | Atomics & sync (grep) | Extractability | Blast radius |
|---|---------|-------|----------------|---------------------------|----------------------|---------------|--------------|
| 1 | Store-identity decision core | `src/store/ladybug.rs` (identity fns), `src/traits.rs` (`compatible`) | ~160 (two sub-cores: ~105 embedding identity + ~55 target resolution) | Sync decision fns; inputs arrive via cached sidecar state (file I/O at open) and one FFI query wrapper; `NEUROSTRATA_MODEL` env read in a fallback path | File-level (monolith): Atomic 24, Mutex 27, RwLock 8, OnceLock 4, thread:: 4, RefCell 3, Cell 3 | Good — target-resolution half is already pure; embedding half needs a snapshot input struct (small–medium) | High |
| 2 | Guard / sandbox policy decisions | `src/guard/validator.rs`, `semantic.rs`, `sandbox.rs`, `fsm.rs`, `models.rs` | ~750 (validate ~150 + trigger-matching cluster ~590 + FSM ~55) | `validate`/`candidate_rules` async + store I/O; sandbox probe spawns podman subprocess; decision fns themselves sync & pure; clock passed in as `now` | validator/semantic/fsm/models: **0 across all categories**; sandbox: Atomic 2 | Good — verdict core already pure; async shell stays outside (small–medium) | High |
| 3 | Secrets-scanning at write surfaces | `src/secrets.rs` | ~950 (production ~1,250 minus ~120 const tables, ~30 types, tests ~165) | **None** — zero env/fs/time/rand hits; sync; no FFI | OnceLock 2 real statics (immutable tables); Mutex/RwLock grep hits are string literals — effective 0 | Already a pure core — minimal extraction | High |
| 4 | Retention gate | `src/retention/gate.rs` | ~60 pure decision (file 439 total; production 213) | Shell does git/fs I/O (`compute_worktree_state`), reads `Instant::now`, mutates global latency windows | Atomic 2, Mutex 9, OnceLock/LazyLock 4 (static latency windows) | Pure core already factored (`retention_gate_with`) — trivial, but small | Medium |
| 5 | Staged-publication atomicity | `src/store/ladybug.rs` (ingest generation fns) | ~140 (staging ~90 + publish ~47 + abort similar) | Heavy: async (`write_with_deadline`, post-publish refresh `.await`), FFI Cypher via `conn.query`, `Utc::now` in staging rows; atomicity delegated to single-statement DB semantics | File-level monolith counts (see #1); no Rust-level atomics carry the guarantee | Poor — only a pointer-transition shadow (~30–50 lines) is sequential; the real guarantee lives in LadybugDB | High |
| 6 | Vocabulary decision envelope | `src/store/ladybug.rs` (`parse_edge_specs`, `write_shape_warnings`), `src/traits.rs` (`memory_vocabulary`) | ~180 (146 + 31) | Pure over `&MemoryPayload`; sync; no I/O, clock, env, or FFI in the decision fns (target-existence checks live in subject 1's FFI wrapper) | Decision fns: none; traits.rs file-level: Atomic 2, OnceLock 2 (vocabulary cache) | Good — already a pure function; small extraction (types live in the monolith) | Medium |

## Per-subject notes

### 1. Store-identity decision core — found, as two distinct sub-cores

**(a) Embedding-descriptor identity** (`src/store/ladybug.rs`):
- `check_data_bearing_identity` (line 5335, ~25 lines): refuse if the recorded sidecar is unparseable; refuse if a data-bearing store's recorded descriptor ≠ the serving descriptor.
- `effective_embedding_descriptor` (~line 5363, ~15 lines): precedence serving → recorded → configured → legacy placeholder.
- `verify_or_record_embedding_descriptor` (line 5419, ~60 lines est.): first descriptor recorded in-process is authoritative; a later incompatible one is rejected.
- `EmbeddingDescriptor::compatible` (`src/traits.rs:291`) is literally `self == other` — the decision reduces to full descriptor equality.
- `FirstWriteFlip::flip_store_state_to_data` (line 4486) performs the sidecar state flip on first write (I/O shell, not decision).
- Coupling: the decision fns are synchronous and comparison-only, but they read `&self` cached state loaded from the sidecar file at open, and the legacy fallback reads `NEUROSTRATA_MODEL` from the environment. No FFI and no async in the decision itself.
- Pure core shape: `decide(recorded: Option<RecordedState>, serving: Option<Descriptor>, configured: Option<Descriptor>) -> Allow | Refuse(reason) | Record(descriptor)`. Everything else (sidecar read/write, registry, embedder binding) stays outside. Extraction cost: **small–medium** (~160 lines of decision code, but it must be lifted out of the monolith and given a snapshot input).
- Blast radius: **high**. A wrong *allow* mixes incompatible embedding spaces into one store, silently corrupting recall semantics for every stored memory; a wrong *refuse* bricks access to a healthy store.

**(b) Write-target identity resolution** (`src/store/ladybug.rs`):
- `declared_target_candidates` (line 15229, ~20 lines, pure), `pick_declared_target` (line 15250, ~35 lines, pure): qualified generation-scoped id wins, then the exact declared id, then a unique suffix match, else `Missing`/`Ambiguous`.
- `resolve_target_on_write` (line 15285, ~35 lines) is the FFI shell: one `conn.query` (Cypher) feeding `pick_declared_target`.
- Read-side sibling: `dedup_hits` (line 13207, ~18 lines, pure) keeps the best-priority hit per id.
- Extraction cost: **small** — the core already exists in exactly the inputs-in/decision-out shape.
- Blast radius: **medium–high**. A wrong pick links an edge to another namespace's or a superseded generation's memory (cross-project contamination); a wrong `Missing` silently drops declared relationships.

### 2. Guard / sandbox policy decisions — found (`src/guard/`, 2,962 lines incl. 396 test-only testkit)

- `GuardValidator::validate` (`validator.rs:42`, async, ~150 lines): fetches candidate rules, matches triggers, composes the verdict, bumps hit counts.
- `matches_trigger` (`validator.rs:218`) plus its helper cluster (file-mutation target/shebang extraction, shell-command surface extraction, branch matching, ~590 lines to the next major section): **pure, synchronous** string/surface decision logic.
- `SemanticEvaluator::candidate_rules` (`semantic.rs:151`, async store search) with pure helpers `rules_from_rows`, `is_live_rule_row(metadata, now)` — the clock is passed in, not read.
- `sandbox.rs` (372 lines): `probe_podman_available` (line 16) shells out to podman; probe result cached (the file's Atomic 2). A validate without the sandbox never probes.
- `fsm.rs` (109 lines): `evaluate_fsm_guard` (~55 lines) is a fully pure task-gate FSM decision (allow / block-with-named-next-transition).
- Synchronization census is remarkable: validator, semantic, fsm, and models files have **zero** Atomic/Mutex/RwLock/OnceLock/LazyLock/thread/RefCell/UnsafeCell/Cell hits.
- Pure core shape: `evaluate(rules: &[BehavioralRule], action: &Action) -> Verdict` — already 90% factored; rule fetching, hit-count persistence, and the podman probe stay outside. Extraction cost: **small–medium**.
- Blast radius: **high**. This is a safety boundary: a false negative lets a prohibited action execute; a false positive blocks legitimate work and erodes trust in the gate.

### 3. Secrets-scanning at write surfaces — found (`src/secrets.rs`, 1,415 lines)

- `SecretScanner::scan_str` (line 332) and `scan_value` (line 391) return `Option<Rejection>`; `scan_entry_point` (line 805) is the write-surface entry. Supporting heuristics: provider-form table (~90 lines of consts), placeholder patterns, high-entropy token check (Shannon entropy, line 652), JWT segment validation, code-expression/source-reference exemptions (~lines 162–330).
- Coupling: **none found**. Grep for `env::`, `fs::`, `File`, `SystemTime`, `Instant`, `rand` returns zero hits. No async, no FFI, no clock.
- Synchronization: two `OnceLock` statics (the scanner singleton at line 322 and the base64 decode table at line 765), both immutable-after-init lookup data. The Mutex/RwLock grep hits are string literals in a known-type-name list (line 223).
- Extraction cost: **minimal** — it already is a pure synchronous core (`text | JSON value → Option<Rejection>`). A Kani harness mainly needs bounded string/JSON inputs (note the existing `MAX_SCAN_BYTES` 256,000 and depth cap 8, which a harness would shrink).
- Blast radius: **high**. A false negative writes a live credential into a persistent, searchable, mesh-synced memory store — a secret at rest that then propagates. A false positive rejects a legitimate memory write (visible data loss of that write).

### 4. Retention gate — found, with a naming caveat (`src/retention/gate.rs`, 439 lines)

- Caveat: this gate is **not** memory expiry/TTL. It verifies that a *task receipt* is retained — present in the `ReceiptStore` and still bound to its worktree's current state — **and** that plan/upsert p95 latencies sit inside the 100 ms budget: `retention_gate_with(receipt_exists, p95_plan, p95_upsert) = receipt_exists && budget_verified(...)`.
- Pure decision core ≈ 60 lines: `budget_verified` (line 6), `p95_ms` (line 12), `retention_gate` / `retention_gate_with` (lines 119/129).
- Shell: `receipt_is_retained` (line 163) reads the store, calls `crate::task::receipt::compute_worktree_state` (git/filesystem I/O), reads `Instant::now`, and records samples into **global** latency windows — static `Mutex<VecDeque>` state plus last-observed slots (file census: Mutex 9, Atomic 2, OnceLock/LazyLock 4). The gate functions themselves are synchronous; no direct FFI.
- Extraction cost: **trivial** for the decision (already factored as `retention_gate_with`); `p95_ms` over a sample slice is independently provable. The stateful window machinery is the only part needing care, and it is small.
- Blast radius: **medium**. A wrong pass certifies work whose receipt is gone or stale (audit-trail integrity) or blesses a latency regression; a wrong fail blocks task-completion flows. Nothing data-destructive.

### 5. Staged-publication atomicity — found, but it is a DB-semantics property, not a Rust decision core

- Location: ingest-generation machinery in `src/store/ladybug.rs` — staging-row creation (the function ending at `publish_generation`, ~90 lines including the retention-protection comments), `publish_generation` (line 11926, ~47 lines), `abort_generation` (line 11973).
- The mechanism: each ingest writes into a staging generation; publishing reads `(active_generation, previous_generation)` and flips the pointer in **one** MERGE statement so the pair updates atomically, then deletes only its own staging row. Rollback target = the recorded previous generation.
- Coupling: maximal for this census — async (`write_with_deadline`, plus `refresh_lexical_after_pointer_change().await` after the flip), FFI (Cypher through `conn: &Connection` into LadybugDB), wall clock (`chrono::Utc::now()` stamped on staging rows), and multi-ingest concurrency is the entire point (overlapping ingests' staging rows are retention-protected until each publishes or aborts).
- Extraction assessment: **poor fit for a sequential Kani core**. A pure pointer-transition state machine (~30–50 lines: stage → publish → previous-becomes-rollback) could be extracted and proven, but that proves the transition table, not the atomicity — the guarantee is LadybugDB's single-statement execution, which Kani cannot see. Extraction cost medium, verification value low–medium.
- Blast radius: **high**. A wrong flip publishes a half-ingested generation to readers, clobbers the rollback pointer, or deletes a concurrent ingest's staging row.

### 6. Vocabulary decision envelope — found (`src/store/ladybug.rs`)

- `parse_edge_specs` (line 5644, ~146 lines): parses a payload's declared relation fields into `EdgeSpec`s, skipping invalid items with `WriteShapeWarning`s (soft-fail: the write proceeds). Includes the access-topology shape check (lines 5741–5766; vocabulary v5's five declared keys) as its section (E).
- `write_shape_warnings` (line 5790, ~31 lines) and the `WriteShapeWarning` type (~line 5623). The vocabulary itself comes from `crate::traits::memory_vocabulary()`.
- Coupling: the decision fns are pure over `&MemoryPayload` — synchronous, no I/O, no clock/env, no FFI. (Whether a qualified target *exists* is decided elsewhere, by subject 1b's FFI-backed resolution; this envelope only shapes and warns.)
- Extraction cost: **small** — already inputs-in/decision-out; the work is lifting `EdgeSpec`/warning types and the vocabulary table reference out of the monolith and bounding JSON inputs for a harness.
- Blast radius: **medium**. A wrong accept writes malformed edges/topology facts that poison graph traversal and ontology grounding; a wrong skip silently drops declared relationships. The soft-fail design caps the downside — defects surface as warnings, not lost writes.

## Other decision-shaped components noticed (not named by the panel)

1. **`src/rebac.rs` (469 lines) — the strongest unnamed candidate.** `allowed(user, resource_owner, relation, acl)` (line 4), `allowed_with_roles` (line 15), `check(predicate, user, resource)` (line 45), `resolve_principal` (line 53), plus metadata extractors/validators (`extract_visible_to`, `validate_visible_to`, `extract_acl_with_roles`). Pure, synchronous access-control decisions over slices and JSON metadata. Blast radius high: a wrong *allow* exposes memories across principals. On shape alone this is a better Kani subject than #4 or #5.
2. **`src/guard/fsm.rs::evaluate_fsm_guard`** — counted inside subject 2 but worth naming separately: a ~55-line, zero-dependency pure FSM (claim → ContextLoaded → TaskClaimed → Mutating, with the Breath rule). The cheapest possible pilot subject.
3. **`src/store/wal_header.rs` (333 lines)** — `read_header(bytes) -> HeaderRead` (line 48), `wal_checksum_flag` (line 74): pure byte-slice parsing/validation decisions guarding WAL integrity. Small, correctness-critical.
4. **`src/synaptic_mesh/conflict.rs` (572 lines)** — conflict registration/resolution decisions for mesh sync, but filesystem-coupled (tmp-file + rename overlay persistence), so a tier below the three above.
5. **`src/doc_gate.rs` (970 lines)** — not inspected in depth; flagged as unexamined, name suggests another gate-shaped component.

## Bottom line — genuinely extractable sequential cores

- **Already pure, minimal work:** secrets scanner (#3), vocabulary envelope (#6), guard trigger-matching + FSM (#2 core), store target resolution (#1b), ReBAC `allowed`/`check` (unnamed).
- **Extractable with a small refactor:** embedding-identity decision (#1a — snapshot input struct), retention decision (#4 — already factored, but tiny and lower value).
- **Not a genuine sequential core:** staged-publication atomicity (#5) — its guarantee lives in LadybugDB statement semantics, invisible to Kani.
