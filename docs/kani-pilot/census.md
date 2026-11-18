# NeuroStrata Kani Pilot — Stage 0 Defect Census (G1)

## Method and basis

- **Census basis:** upstream `Cognilogical/NeuroStrata`, default branch `main`, HEAD `dd1affef8b685bda92616736ed8ef479cc039224` dated 2026-11-17 15:04:59 -0500 (`chore(release): cut v1.18.1 (#260)`). Fresh clone taken 2026-11-17 for this census; no code was built or modified.
- **Sample:** the most recent **100** commits on `main` whose subject matches `^fix(\(|:)` (case-insensitive, Conventional-Commits style). 306 such commits exist on `main`, so the full 100 were available. The sample spans **2026-11-17 back to 2026-11-02** — a 16-day window dominated by large remediation waves (PR #52 rounds, Waves 43–52, WAL-recovery and taxonomy programmes), which is why bundled commits are common.
- **Per commit:** short sha, date, full subject, and changed files from `git show --stat/--numstat`. Each commit was classified on three axes: defect class (exactly one of CONC/ATOM/SCHEMA/SEC/SEQ/ERRH/BUILD/OTHER), primary component, and a Kani-catchable verdict (YES / PARTIAL / NO) with a one-line reason grounded in the subject, file set, churn distribution, and — for bundled or ambiguous commits — the commit body/diff.
- **Bundling convention:** many commits fix several defects at once. The class is the headline/dominant defect (subject order, then churn); secondary defects are named in the reason. The verdict applies to the commit as a whole: a pure sequential core bundled with FFI/async changes rates PARTIAL, not YES.
- **Verdict definitions:** YES = bounded sequential logic error in synchronous Rust with no FFI, atomics/threads, or async-runtime semantics in the defect path. PARTIAL = a sequential core exists but the defect manifests through integration, concurrency, FFI, or async seams. NO = inherently concurrency, FFI, async, I/O, environment, build, or non-Rust.

## Headline tallies

### Defect class (n=100)

| Class | Count | Share |
|---|---:|---:|
| SEQ | 29 | 29% |
| OTHER | 16 | 16% |
| ATOM | 12 | 12% |
| BUILD | 12 | 12% |
| SEC | 10 | 10% |
| SCHEMA | 9 | 9% |
| ERRH | 9 | 9% |
| CONC | 3 | 3% |

### Kani-catchable verdict (n=100)

| Verdict | Count |
|---|---:|
| YES | 1 |
| PARTIAL | 69 |
| NO | 30 |

### Class × Kani-catchable matrix

| Class | n | YES | PARTIAL | NO |
|---|---:|---:|---:|---:|
| CONC | 3 | 0 | 0 | 3 |
| ATOM | 12 | 0 | 11 | 1 |
| SCHEMA | 9 | 0 | 8 | 1 |
| SEC | 10 | 0 | 9 | 1 |
| SEQ | 29 | 1 | 28 | 0 |
| ERRH | 9 | 0 | 8 | 1 |
| BUILD | 12 | 0 | 0 | 12 |
| OTHER | 16 | 0 | 5 | 11 |
| **Total** | 100 | 1 | 69 | 30 |

### Primary component (grouped, n=100)

| Component area | Count |
|---|---:|
| src/store (LadybugDB store layer) | 16 |
| plugins (TypeScript) | 10 |
| src/rdf | 8 |
| scripts / .github / CI | 8 |
| src/synaptic_mesh | 7 |
| src/task | 5 |
| src/guard | 3 |
| docs | 3 |
| multi-area bundles | 3 |
| src/server.rs | 2 |
| src/main.rs | 2 |
| src/parser/ingest.rs + src/store | 2 |
| src/parser/ingest.rs | 2 |
| src/kernel + src/rdf/action.rs | 1 |
| src/kernel + src/connectors | 1 |
| src/parser/ingest.rs + src/embed.rs + src/store | 1 |
| src/server.rs + src/daemon.rs | 1 |
| src/parser/ingest.rs + src/rdf/action.rs | 1 |
| src/secrets.rs + src/embed.rs | 1 |
| src/formal_kernel.rs | 1 |
| src/graph_read + src/store/ladybug.rs | 1 |
| src/daemon.rs + src/events + plugins | 1 |
| src/daemon.rs | 1 |
| src/secrets.rs | 1 |
| src/server.rs + src/daemon.rs + src/main.rs | 1 |
| src/server.rs + src/store/ladybug.rs | 1 |
| src/lexical.rs + src/fts + src/store | 1 |
| src/secrets.rs + src/parser/ingest.rs | 1 |
| src/procmem.rs | 1 |
| src/embed.rs + src/parser/ingest.rs | 1 |
| src/lexical.rs | 1 |
| src/embed.rs | 1 |
| src/secrets.rs + src/parser + src/server.rs | 1 |
| src/source_context.rs | 1 |
| src/model_choice.rs | 1 |
| src/daemon.rs + src/server.rs + src/main.rs | 1 |
| src/server.rs + src/task/mod.rs | 1 |
| src/server.rs + src/store + src/lexical.rs | 1 |
| src/daemon.rs + src/main.rs + src/server.rs | 1 |
| src/main.rs + src/store/ladybug.rs | 1 |
| src/daemon.rs + src/judgment + src/graphql | 1 |
| src/secrets.rs + src/store/ladybug.rs | 1 |

## Reading of the census

- **Sequential logic dominates; concurrency is rare.** SEQ is the largest class (29) and CONC the smallest (3: an atomic hit_count, concurrent-edit loss in mesh import, and a wave bundle whose distinctive item is poisoned-mutex recovery). The defect population is not a concurrency population.
- **But only 1 of 100 is a clean Kani YES.** The lone YES is the `src/formal_kernel.rs` fix (pure const-fn arithmetic + cycle guard). Everything else sequential rates PARTIAL (69 of 100 overall) because the defect manifests through a seam Kani cannot model directly: the LadybugDB FFI store (`src/store` is the largest component area, 16), the async daemon/server, the Tantivy/ONNX integrations, or mesh I/O. A Kani pilot therefore lives or dies on extracting bounded sequential cores behind harnesses — the raw defect stream is not directly provable.
- **Multi-step consistency is the recurring failure mode.** ATOM (12) clusters in WAL recovery (4 commits), ingest generation swaps/carry-forward (2), the kinetic-action CAS path (2), and the mesh record pipeline (sign-after-rebuild ordering, twin-revert resolution). These are sequential cores with integration manifestations — the natural PARTIAL hunting ground.
- **Security fixes (10) are mostly seam gaps, not algorithm bugs:** secret scanning not wired into a path (×3), validation/ACL gates bypassed or unenforced (×4), credential resolution timing, bundle containment, and one TypeScript ReDoS. Their predicates are sequential; their defects are wiring.
- **Non-Rust and non-code mass is significant:** BUILD (12) + OTHER (16) = 28% of the sample, including nine PR #52 review-round bundles dominated by TypeScript plugin `dist/` files and one `fix(...)` commit (`fcd00648`) that changes only a plan document — no code at all.

## Existing-controls inventory

**Test functions (grep over `src/` and `tests/` at the census HEAD):**

| Attribute | src/ | tests/ | Total |
|---|---:|---:|---:|
| `#[test]` | 1,305 | 111 | 1,416 |
| `#[tokio::test]` (all forms) | 760 | 0 | 760 |
| **Total test functions** | | | **2,176** |

Plus 231 `#[cfg(test)]` module markers in `src/` and 17 integration-test files under `tests/`. No `#[rstest]` or `#[test_case]` usage. All tests are example-based; CI runs them single-threaded (`cargo test --locked -- --test-threads=1`).

**CI (`.github/workflows/ci.yml`)** — jobs: `build` (`cargo build --release --locked` on ubuntu/windows/macos-15, plus an `include_str!`-target presence check), `test` (`cargo test` for the main crate, the `neurostrata-wasm` package, and the release binary; plus `scripts/test-remediation-eval.sh`), `test-report`, `plugins` (`node --test plugins/core/test.mjs`), `scripts` (shell suites: backup, gate-receipt, gate-receipt-live, disk-hygiene, main-ruleset), `audit` (`cargo audit` 0.22.1), and `release-references` (changelog/reference consistency scripts). A separate `gate-receipt.yml` enforces the merge-gate receipt on PR review/comment events. **No clippy, rustfmt, Miri, or sanitizer jobs** appear in any workflow.

**Nightly / release:** `nightly.yml` runs scheduled cross-platform release builds (with a platform guard so one OS failing does not block the other, and a Strawberry-Perl OpenSSL step for Windows). `release.yml` verifies and builds release artifacts on push/dispatch.

**Property / fuzz / concurrency tooling: absent.** No `proptest`, `quickcheck`, `bolero`, `loom`, `honggfuzz`, `arbitrary`, or `libfuzzer-sys` in `Cargo.toml` or `Cargo.lock`; no `fuzz/` directory; no `proptest!`/loom/fuzz attributes anywhere in the tree; and no `#[kani]` harness or Kani reference in the tree, manifests, or workflows.

**Adjacent formal control:** a Lean 4 proof tree under `proofs/` (with `tests/lean_parity_test.rs` parity tests) already covers parts of the formal kernel and taxonomy — several census commits (e.g. `91153e5a`, `37a1c3f7`, `41952d34`) touch proofs alongside Rust. This is the one existing verification mechanism in Kani's niche, and it is spec-parity oriented rather than code-level bounded model checking.

## Full per-commit classification

| # | Sha | Date | Subject | Class | Component | Kani | Reason |
|---:|---|---|---|---|---|---|---|
| 1 | `acb22cd8` | 2026-11-17 | fix(taxonomy): align auto-briefing probe with view visibility and cover probe fallback (#257, #258) (#259) | SEQ | src/rdf/taxonomy.rs | PARTIAL | Auto-briefing probe ignored view-visibility rules; sequential predicate logic, but exercised through store-backed taxonomy views. |
| 2 | `7b9a89d4` | 2026-11-17 | fix(release,ops): sync release body from changelog and codify main merge-gate ruleset (#216, #227) (#256) | BUILD | scripts + .github/workflows | NO | Release-body sync and merge-gate ruleset are release/CI tooling (shell + workflow config), no Rust logic. |
| 3 | `a26b4282` | 2026-11-17 | fix(action,kernel): address post-merge Codex findings from #251 and #252 (#247, #250) (#255) | SEQ | src/kernel + src/rdf/action.rs | PARTIAL | Pool overrides applied in wrong precedence order and legacy post-images not normalized; sequential ordering logic manifesting via kernel/store integration. |
| 4 | `aab91e43` | 2026-11-17 | fix(task): resolve MCP claim worktree scope and exempt git push from zero-action start (#243) (#253) | SEQ | src/task/mod.rs + src/server.rs | PARTIAL | MCP claim worktree scope and zero-action gate exemption were resolved against the wrong scope; sequential gate logic tied to git/worktree state. |
| 5 | `8652a76c` | 2026-11-17 | fix(taxonomy,mesh): namespace-qualify taxonomy row IDs and preserve export collisions (#246, #249) (#254) | SCHEMA | src/rdf/taxonomy.rs + src/synaptic_mesh/worker.rs | PARTIAL | Taxonomy row IDs not namespace-qualified and export collisions dropped; identity/representation defect at the storage-export seam. |
| 6 | `23c29ead` | 2026-11-17 | fix(action): resolve staged twin revert heads and verify full upsert post-image (#247, #248) (#251) | ATOM | src/rdf/action.rs | PARTIAL | Staged twin revert heads resolved inconsistently and upsert post-image not fully verified; multi-step revert consistency with a sequential CAS-like core in store-backed code. |
| 7 | `d371730b` | 2026-11-17 | fix(connectors): pre-seed default pools and persist pool registration (#250) (#252) | SEQ | src/kernel + src/connectors | PARTIAL | Default connector pools not pre-seeded and registrations not persisted; initialization-order logic manifesting through kernel ports. |
| 8 | `d1fbbeaf` | 2026-11-17 | fix(ops): pin CI toolchain/actions, add cargo-audit gate, harden disk-hygiene (#210, #215) (#239) | BUILD | .github/workflows + scripts + Cargo.toml/lock | NO | CI toolchain/action pinning, cargo-audit gate and disk-hygiene scripts are build/CI configuration. |
| 9 | `37a1c3f7` | 2026-11-17 | fix(wal): a removed frozen WAL is a replay only on evidence (#228) (#245) | ATOM | src/store (ladybug.rs, wal_accounting.rs) | PARTIAL | A removed frozen WAL was replayed without evidence; WAL-recovery consistency decision with sequential accounting core over FFI store/filesystem state. |
| 10 | `ad5690b4` | 2026-11-16 | fix(embed\|ingest\|census): one bad input, one shrug, not the whole night (#213, #214, #224) (#244) | ERRH | src/parser/ingest.rs + src/embed.rs + src/store | PARTIAL | One bad input failed the whole embed/ingest batch; per-item error-isolation logic inside an async pipeline with an FFI embedder. |
| 11 | `6b84c5be` | 2026-11-16 | fix(recovery): recover #221/#222/#223 onto main (closes #209, #211, #212) (#237) | SEQ | src/store + src/storage + src/server.rs | PARTIAL | Skipped buffer-pool checkpoints were misrecorded, so the next open misread the shutdown state; sequential store-lifecycle logic over the LadybugDB FFI. |
| 12 | `131e0967` | 2026-11-16 | fix(gate-receipt): pin the gate to the base branch, unify the marker parser, fail closed (#225 #226 #229) (#238) | BUILD | scripts + .github/workflows (gate-receipt) | NO | Merge-gate receipt pinning/parser/fail-closed behaviour lives in shell scripts, fixtures and workflow config. |
| 13 | `8ea7711e` | 2026-11-16 | fix(hooks): pre-commit matches recorded paths in every spelling (#236) | SEQ | src/task/mod.rs + scripts/hooks | PARTIAL | Pre-commit hook failed to match recorded paths under alternate spellings; path-normalization logic at the git-hook seam. |
| 14 | `4fbb7c93` | 2026-11-16 | fix(mcp): answer unknown methods with method-not-found (#234) | ERRH | src/server.rs + src/daemon.rs | PARTIAL | Unknown MCP methods were not answered with method-not-found; protocol error-handling path in the async server dispatch. |
| 15 | `56d7f969` | 2026-11-16 | fix(wal): one zero-record accounting across all WAL recovery arms (#207) (#220) | ATOM | src/store (WAL accounting) | PARTIAL | Zero-record WAL accounting differed across recovery arms; cross-path consistency of a multi-step recovery, sequential core over FFI/filesystem state. |
| 16 | `2ff8156f` | 2026-11-16 | fix(recall): rank rule enforcement before fan-out and bound the linked-files census (#219) | SEQ | src/store/ladybug.rs + src/graph_read | PARTIAL | Rule enforcement ranked after fan-out and the linked-files census was unbounded; ordering/bounding logic over store query results. |
| 17 | `a569befd` | 2026-11-16 | fix(gate-receipt): gate on unresolved P1/P2 review threads, not just merge words (#217) (#218) | BUILD | scripts + .github/workflows (gate-receipt) | NO | Gating merges on unresolved P1/P2 review threads is CI tooling (scripts + workflow), not product Rust. |
| 18 | `1787f1b2` | 2026-11-15 | fix(action,ingest): keep duplicate slugs and ghost callees out of the firehouse (neurostrata-j0x4) (#205) | SEQ | src/parser/ingest.rs + src/rdf/action.rs | PARTIAL | Duplicate slugs and ghost callees were admitted; sequential validation logic at the ingest/store seam. |
| 19 | `dfba303f` | 2026-11-15 | fix(ingest): eliminate detect_jwt UTF-8 slice panic and bound FastEmbedder batch size | ERRH | src/secrets.rs + src/embed.rs | PARTIAL | detect_jwt panicked slicing a UTF-8 boundary (pure string code) but the same commit also bounds the FFI FastEmbedder batch, so the commit as a whole is integration-bound. |
| 20 | `d3623e3a` | 2026-11-15 | fix(taxonomy): resolve residual k3-arch findings on IRI parity, DirectedWalk proofs, and CLI outbox tee (neurostrata-ih7f) | SCHEMA | src/rdf + src/handlers/taxonomy.rs | PARTIAL | IRI parity drift between Rust taxonomy code and the formal spec, plus DirectedWalk proof wiring; representation-integration defect. |
| 21 | `41952d34` | 2026-11-15 | fix(taxonomy): resolve 5 release blockers for atomic registry, post-image DAG CAS, RDF/SPARQL parity, Lean 4 proofs, and mesh/SHACL wiring | ATOM | src/rdf + src/registry.rs + src/store | PARTIAL | Atomic registry and post-image DAG CAS blockers; multi-step consistency core embedded in store/mesh integration (plus RDF/SPARQL parity items). |
| 22 | `046b9be9` | 2026-11-15 | fix(server): align neurostrata_taxonomy JSON schema enums with formal_kernel and taxonomy parser (neurostrata-f0bn) | SCHEMA | src/server.rs | NO | Static JSON-schema enum list drifted from the parser/formal kernel; a declaration mismatch with no executable logic path for Kani to verify. |
| 23 | `91153e5a` | 2026-11-15 | fix(formal-kernel): const fn realization_status_gap_bps + cycle-guarded SKOS disjointness + unit tests (neurostrata-f0bn) | SEQ | src/formal_kernel.rs | YES | Pure const-fn basis-point computation and a cycle guard in the pure formal kernel; bounded sequential logic with no FFI, threads or async. |
| 24 | `1144ca82` | 2026-11-14 | fix(scenario): enforce atomic base-row fingerprint CAS inside commit_action_batch (neurostrata-f0bn) | ATOM | src/rdf/action.rs + src/store/ladybug.rs | PARTIAL | Base-row fingerprint CAS was not enforced inside commit_action_batch; multi-step commit consistency with sequential core over the FFI store. |
| 25 | `c96994b8` | 2026-11-14 | fix(guard): prevent global-no-python from rejecting Python or .py in prose edits (neurostrata-vkcg) | SEQ | src/guard/validator.rs | PARTIAL | global-no-python rule rejected 'Python'/'.py' appearing in prose; sequential matcher logic inside the daemon edit pipeline. |
| 26 | `0b1cdba3` | 2026-11-14 | fix(infographics): Taught SVG text that boxes have walls for a reason. Fixed all text overruns across 01..05. (neurostrata-ar3n) | OTHER | docs/assets (SVG infographics) | NO | SVG text overruns in documentation artwork; no code defect. |
| 27 | `45555738` | 2026-11-14 | fix(infographics): Because apparently 'SOCKED' and 'FOCKED' are not real security primitives. Fixed every single typo. (neurostrata-r954) | OTHER | docs/README + image assets | NO | Misspelled words in infographic images/README; content defect, no code. |
| 28 | `c0d86df0` | 2026-11-14 | fix(task-gate): scope strict pre-push gate to caller branch and worktree (neurostrata-7koo) | SEQ | src/task (gate.rs, mod.rs) | PARTIAL | Strict pre-push gate applied beyond the caller's branch/worktree; sequential scoping logic tied to git state. |
| 29 | `da3ba6d8` | 2026-11-14 | fix(wal): resolve WAL recovery, strict verification, artifact safety, and CLI no-open issues (#189-#196) (#201) | ATOM | src/store (WAL) + src/storage/snapshot.rs | PARTIAL | WAL recovery, strict verification, artifact safety and CLI no-open bundle; recovery-consistency core over FFI/filesystem state. |
| 30 | `4199bf64` | 2026-11-14 | fix(recall): put newer and more specific memories first in file recall (#200) | SEQ | src/graph_read + src/store/ladybug.rs | PARTIAL | File recall did not order newer/more specific memories first; ranking logic over store query results. |
| 31 | `23479533` | 2026-11-14 | fix(test): close the provenance fixture's store instead of leaking it (#199) | OTHER | src/store/ladybug.rs (test fixture) | NO | Provenance test fixture leaked its store instead of closing it; test-only resource hygiene against the FFI store. |
| 32 | `276e8c16` | 2026-11-14 | fix(store): stop retrying a checkpoint that does not fit in the buffer pool (#198) | ERRH | src/store/ladybug.rs | PARTIAL | A checkpoint that can never fit the buffer pool was retried indefinitely; retry/error policy around an FFI engine call. |
| 33 | `679f8cf5` | 2026-11-13 | fix(store): harden WAL recovery precedence, strict checksum gate, and verified reopen (#170) (#178) | ATOM | src/store (WAL) + src/storage/snapshot.rs | PARTIAL | WAL recovery precedence, strict checksum gate and verified reopen; multi-step recovery consistency over FFI/filesystem state. |
| 34 | `15ad0dff` | 2026-11-13 | fix(security): linearize extractRepoFilePaths and bump vulnerable deps (#169) (#177) | SEC | plugins/core (TypeScript) | NO | extractRepoFilePaths had super-linear (ReDoS) matching plus vulnerable dependency bumps; security fix in TypeScript plugin code Kani cannot read. |
| 35 | `9ad05d28` | 2026-11-13 | fix(backup): stage backup/restore copies atomically and honor snapshot retention (#171) (#179) | ATOM | scripts/backup.sh + src/main.rs + src/store | NO | Backup/restore copies were not staged atomically and retention was ignored; the mechanism is filesystem copy I/O in shell/Rust, inherently I/O-bound. |
| 36 | `b33dd04f` | 2026-11-13 | fix(store): implement OpenProvenance, shared width reconcile, fast-path retry, and fallback init (#172) (#180) | SCHEMA | src/store/ladybug.rs + src/model_choice.rs | PARTIAL | OpenProvenance, shared-width reconcile and fallback init for store identity; storage-integration semantics over the FFI store. |
| 37 | `cb61296e` | 2026-11-13 | fix(daemon,events): isolate token tests, retry timed-out freshness flips, and harden procmem (#173) (#181) | ERRH | src/daemon.rs + src/events + plugins (dist) | NO | Token-test isolation, retry of timed-out freshness flips and procmem hardening; async daemon/test-infrastructure behaviour, timing-dependent. |
| 38 | `4af00e95` | 2026-11-13 | fix(daemon): stop daemon tests from installing plugins into their folder (#176) | OTHER | src/daemon.rs (tests) | NO | Daemon tests installed plugins into their own folder; test-environment side effect, I/O-bound. |
| 39 | `a953e55e` | 2026-11-13 | fix(test): make cargo test compile on Windows again (#175) | BUILD | src/main.rs (cfg) | NO | cargo test compilation broken on Windows; platform cfg/build fix. |
| 40 | `91b745a8` | 2026-11-13 | fix(harness): leave custom hook commands alone when repairing installer paths (#174) | SEQ | src/task/harness_plugins.rs | PARTIAL | Installer path repair overwrote custom hook commands; sequential repair logic over filesystem config state. |
| 41 | `94ffcc6b` | 2026-11-13 | fix(secrets): scan metadata keys and name oversized metadata (#147 residuals) | SEC | src/secrets.rs | PARTIAL | Secret scanning missed metadata keys (and oversized metadata went unnamed); scanner core is sequential text logic but the defect is a coverage seam in server/store paths. |
| 42 | `8a5c5f09` | 2026-11-13 | fix(guard): make hit_count atomic and bound the rule listing | CONC | src/guard/semantic.rs + src/store | NO | Guard hit_count was a non-atomic counter raced by concurrent evaluations; inherently an atomics/concurrency defect. |
| 43 | `82de2702` | 2026-11-13 | fix(ingest): re-parse on a summary failure; scope abort; fail unreadable files | ERRH | src/parser/ingest.rs + src/store | PARTIAL | Summary failure did not trigger re-parse, abort was mis-scoped and unreadable files did not fail; error-path logic in the async ingest pipeline. |
| 44 | `b11f37a3` | 2026-11-13 | fix(metadata): authorize set_metadata and gate ownership rewrites | SEC | src/server.rs + src/daemon.rs + src/main.rs | PARTIAL | set_metadata lacked authorization and ownership rewrites were ungated; the authz predicate is sequential but manifests through server integration. |
| 45 | `2b3fe5b4` | 2026-11-12 | fix(hygiene): isolate chunk failures and gate the doctor watermark (#142) | ERRH | src/server.rs + src/store/ladybug.rs | PARTIAL | One failed chunk poisoned hygiene processing and the doctor watermark was ungated; failure-isolation logic across server/store integration. |
| 46 | `0a5d80d6` | 2026-11-12 | fix(lexical): filter both lexical routes to active generations (#149) | SEQ | src/lexical.rs + src/fts + src/store | PARTIAL | Both lexical routes leaked non-active generations; generation-filter logic over the store-backed index. |
| 47 | `3aaf6a1f` | 2026-11-12 | fix(ingest): a never-indexed skipped file is not a change (#148a) | SEQ | src/parser/ingest.rs | PARTIAL | A never-indexed skipped file was counted as a change; sequential bookkeeping logic driven by stored generation state. |
| 48 | `fc848a99` | 2026-11-12 | fix(secrets): stop source false positives and report skipped files (#147) | SEC | src/secrets.rs + src/parser/ingest.rs | PARTIAL | Secret scanner produced source false positives and skipped files went unreported; sequential matcher core with ingest-integration seams. |
| 49 | `1bf49898` | 2026-11-12 | fix(synaptic-mesh): preserve concurrent edits on archived records when superseded (neurostrata-ngd9) | CONC | src/synaptic_mesh/import.rs | NO | Concurrent edits to archived records were lost when superseded; the defect scenario is concurrent-edit racing in mesh import. |
| 50 | `332441c1` | 2026-11-12 | fix(procmem): validate the growth threshold and document platform limits (#141 follow-up) | SEQ | src/procmem.rs | PARTIAL | Memory-growth threshold was accepted unvalidated; bounded validation logic inside platform memory-monitoring code. |
| 51 | `b902b83f` | 2026-11-12 | fix(embed): guard the arena pin and parse truthy spellings (#135 follow-up) | SEQ | src/embed.rs + src/parser/ingest.rs | PARTIAL | Arena pin unguarded and truthy config spellings misparsed; sequential parsing logic at the FFI embedder configuration seam. |
| 52 | `d2eb1d4c` | 2026-11-12 | fix(lexical): literal code-token search and pre-TopDocs filtering (PAR-10) (#127) | SEQ | src/lexical.rs | PARTIAL | Code-token search was not literal and filtering happened after TopDocs; sequential filter logic inside the Tantivy-backed search integration. |
| 53 | `893b23fa` | 2026-11-12 | fix(embed): disable the CPU arena by default, with opt-out (#107) | OTHER | src/embed.rs | NO | CPU arena default flipped to opt-in; a runtime configuration default change whose behaviour is FFI-runtime-dependent. |
| 54 | `6954af2d` | 2026-11-12 | fix(source_context): make cargo test compile and pass on Windows (#103) | BUILD | multi-file src (Windows cfg) | NO | cargo test did not compile/pass on Windows across seven files; platform build fixes. |
| 55 | `c49653b7` | 2026-11-12 | fix(search): apply current-state visibility at retrieval merge and hydration (PAR-02) (#121) | SEQ | src/store/ladybug.rs | PARTIAL | Current-state visibility was applied at the wrong retrieval stage (merge/hydration); sequential filter logic over FFI store results. |
| 56 | `4476645b` | 2026-11-12 | fix(guard): list candidate rules instead of ranking by vector distance (#106) | SEQ | src/guard (mod.rs, semantic.rs, validator.rs) | PARTIAL | Guard ranked candidate rules by vector distance instead of listing them; selection-algorithm logic error over vector-store results. |
| 57 | `fa0af6c2` | 2026-11-12 | fix(ingest): abort generation when carry-forward fails (PAR-04) (#126) | ATOM | src/parser/ingest.rs | PARTIAL | Generation carry-forward failure did not abort the generation switch; multi-step ingest consistency with sequential core in the pipeline. |
| 58 | `cd16848c` | 2026-11-12 | fix(security): scan secrets on metadata updates and live ingest (PAR-03) (#125) | SEC | src/secrets.rs + src/parser + src/server.rs | PARTIAL | Secret scanning was not wired into metadata updates or live ingest; scanner core is sequential, the defect is an integration seam. |
| 59 | `449e9233` | 2026-11-12 | fix(plugins): repair existing Windows hook commands (#113) | SEQ | src/task/harness_plugins.rs | PARTIAL | Existing Windows hook commands were not repaired correctly; sequential repair logic over filesystem config state (sibling of the installer-path fix). |
| 60 | `20908f15` | 2026-11-12 | fix(store): fail closed on an unparseable store-state sidecar (#122) | ERRH | src/store/ladybug.rs + src/model_choice.rs | PARTIAL | An unparseable store-state sidecar was tolerated instead of failing closed; error-handling policy over FFI store state files. |
| 61 | `d25cab4c` | 2026-11-12 | fix(search): honor object ACL entries in lexical documents (PAR-01) (#120) | SEC | src/store/ladybug.rs | PARTIAL | Lexical documents ignored object ACL entries (visibility leak); the ACL predicate is sequential but evaluated over store-backed documents. |
| 62 | `ac523cd6` | 2026-11-12 | fix(source-context): accept canonicalizable marker paths (#123) (#130) | SEQ | src/source_context.rs | PARTIAL | Marker paths that canonicalize were rejected; path-acceptance logic at a filesystem-canonicalization seam. |
| 63 | `c77685b9` | 2026-11-11 | fix(ingest): keep the active generation when nothing changed (#108) | ATOM | src/parser/ingest.rs + src/store | PARTIAL | The active generation was replaced even when nothing changed; generation-swap consistency in the multi-step ingest pipeline. |
| 64 | `8a00da51` | 2026-11-11 | fix(mention): key the symbol-id cache by namespace (#116) | SEQ | src/store/ladybug.rs | PARTIAL | Symbol-id cache was keyed without namespace, colliding across namespaces; sequential keying logic inside the FFI store layer. |
| 65 | `0284de87` | 2026-11-11 | fix(guard): list only task rows in guard_validate's Zero-Action check (#105) | SEQ | src/server.rs | PARTIAL | guard_validate's Zero-Action check listed non-task rows; filter predicate logic evaluated over store-backed rows in a server handler. |
| 66 | `7b8c2b98` | 2026-11-10 | fix(setup): accept an empty embedder list in the choice writer (#101) | SEQ | src/model_choice.rs | PARTIAL | Choice writer rejected an empty embedder list; boundary-condition validation logic in a persistence writer. |
| 67 | `1dec4e3b` | 2026-11-09 | fix: PR #66 follow-up — store-identity defects in model choice (#78) | SCHEMA | src/store/ladybug.rs + src/main.rs + scripts/backup.sh | PARTIAL | Store-identity defects in model choice (PR #66 follow-up); identity semantics at the storage-integration seam over the FFI store. |
| 68 | `73bfb99b` | 2026-11-08 | fix(server): resolve the daemon bearer token at forward time (#64) | SEC | src/daemon.rs + src/server.rs + src/main.rs | PARTIAL | Daemon bearer <redacted> was resolved before forwarding instead of at forward time (stale credential use); sequential timing logic in the async daemon. |
| 69 | `8fdae4b7` | 2026-11-07 | fix(mesh): gate unix-only APIs in mesh import tests so they compile on Windows (#57) | BUILD | src/synaptic_mesh/import.rs (tests) | NO | Unix-only APIs used in mesh import tests broke Windows compilation; test cfg/build fix. |
| 70 | `52c7cfc5` | 2026-11-07 | fix(cli): gate the unix-only set_hook_executable test with cfg(unix) (#59) | BUILD | src/main.rs (test cfg) | NO | Unix-only set_hook_executable test lacked a cfg(unix) gate; test build fix. |
| 71 | `612bd717` | 2026-11-07 | fix(mesh): refuse diff-pass rollback by stale tracked bundle records (#60) | SEQ | src/synaptic_mesh/import.rs | PARTIAL | Diff-pass rollback accepted stale tracked bundle records; sequential staleness-check logic in mesh import over store state. |
| 72 | `764dbdc2` | 2026-11-06 | fix(mesh): keep drain bundles local-only and report bundle write failures (#55) | SEC | src/synaptic_mesh (import, storage, worker) | PARTIAL | Drain bundles could leave the local machine and bundle write failures went unreported; containment predicate is sequential but manifests through mesh I/O. |
| 73 | `00d782b5` | 2026-11-06 | fix(mesh): distinguish RDF-only from full-record signature verification (#54) | SEC | src/synaptic_mesh (import, record, worker) | PARTIAL | RDF-only verification was accepted where full-record signature verification was required; verification-mode logic in the mesh pipeline. |
| 74 | `7df9e41f` | 2026-11-06 | fix(mesh): route bundle-only records through validate_record and epoch_gate (#53) | SEC | src/synaptic_mesh/import.rs | PARTIAL | Bundle-only records bypassed validate_record and the epoch gate; validation-gate routing logic in mesh import. |
| 75 | `237c85e7` | 2026-11-06 | fix(ci): fail fast when an include_str! target is missing from the checkout (#48) | BUILD | .github/workflows + scripts | NO | CI did not fail fast when an include_str! target was missing from the checkout; CI/script tooling. |
| 76 | `e2beea01` | 2026-11-06 | fix(mesh): sign records after lifecycle preservation and RDF rebuild (#51) | ATOM | src/synaptic_mesh/worker.rs | PARTIAL | Records were signed before lifecycle preservation and RDF rebuild, invalidating the signature; step-ordering consistency of a multi-step pipeline. |
| 77 | `584567e0` | 2026-11-05 | fix(rdf): stop reading metadata.source as a string only (v1.16.1) | SCHEMA | src/rdf/shacl.rs + src/store + src/traits.rs | PARTIAL | metadata.source was read as string-only, mishandling other stored shapes; type handling at the storage/RDF seam. |
| 78 | `0175a24c` | 2026-11-05 | fix(ci): give nightly.yml the Strawberry-Perl OpenSSL step so the Windows nightly can build | BUILD | .github/workflows/nightly.yml | NO | Windows nightly lacked the Strawberry-Perl OpenSSL step; CI configuration. |
| 79 | `4a7f85e0` | 2026-11-05 | fix(v115): address round-9 Copilot review comments on PR #52 | OTHER | plugins (TypeScript dist) | NO | Round-9 Copilot review bundle dominated by TypeScript plugin dist files; heterogeneous changes, not Rust Kani can read. |
| 80 | `4b558582` | 2026-11-05 | fix(v115): address round-8 Copilot review comments on PR #52 | OTHER | plugins + src/task + scripts (bundle) | PARTIAL | Round-8 review bundle spanning plugin TypeScript and Rust task code; heterogeneous, with some sequential Rust logic among integration changes. |
| 81 | `ed169798` | 2026-11-05 | fix(v115): address round-7 Copilot review comments on PR #52 | OTHER | plugins (TypeScript dist) | NO | Round-7 Copilot review bundle confined to TypeScript plugin files; not Rust. |
| 82 | `b690fedc` | 2026-11-05 | fix(v115): address round-6 Copilot review comments on PR #52 | OTHER | plugins + src/task (bundle) | PARTIAL | Round-6 review bundle spanning plugin TypeScript and Rust task code; heterogeneous, with some sequential Rust logic among integration changes. |
| 83 | `fbb0e070` | 2026-11-05 | fix(plugins,task,hooks): address round-5 PR #52 Copilot review comments | OTHER | plugins + src/task + scripts/hooks (bundle) | PARTIAL | Round-5 review bundle spanning plugins, Rust task code and hooks; heterogeneous, with some sequential Rust logic among integration changes. |
| 84 | `ee19b95d` | 2026-11-05 | fix(v1.15.0): address round-4 PR #52 review feedback (#49) | OTHER | src/store + src/task + plugins (bundle) | PARTIAL | Round-4 review bundle whose largest churn is src/store/ladybug.rs; heterogeneous store-integration changes rather than one bounded defect. |
| 85 | `be11dad9` | 2026-11-05 | fix(plugins,task): address round-3 PR #52 Copilot review comments (#49) | OTHER | plugins (TypeScript dist) | NO | Round-3 Copilot review bundle dominated by TypeScript plugin dist files; the small Rust portion is incidental. |
| 86 | `118eb29d` | 2026-11-05 | fix(build,plugins,hooks): disable esaxx-rs static CRT on Windows and address round-2 PR #52 review comments (neurostrata-keov) | BUILD | plugins + Cargo.toml/lock + scripts (bundle) | NO | Headline fix disables esaxx-rs static CRT on Windows (build configuration) inside a round-2 review bundle. |
| 87 | `401b8201` | 2026-11-05 | fix(ci,review): resolve Ubuntu/Windows/macOS CI failures and all 17 PR #52 review threads (neurostrata-qjlc) | BUILD | CI + plugins + src/task/store (bundle) | NO | Commit resolves cross-platform CI failures and 17 review threads; build/CI-dominant heterogeneous bundle. |
| 88 | `71007d11` | 2026-11-04 | fix(server,task): expose cached subagent_prompt_injection and dedupe memories_for_file ledger recording (neurostrata-k082) | SEQ | src/server.rs + src/task/mod.rs | PARTIAL | Cached subagent_prompt_injection was not exposed and memories_for_file recorded duplicate ledger entries; sequential exposure/dedup logic in server/task integration. |
| 89 | `fe783257` | 2026-11-04 | fix: address reviewer + checker-free findings on v1.15.0 field audit (#49) | OTHER | plugins + src/task + scripts (bundle) | PARTIAL | v1.15.0 field-audit findings bundle spanning plugin TypeScript and Rust task/server code; heterogeneous, some sequential Rust logic. |
| 90 | `5556ffc8` | 2026-11-03 | fix(plugins/opencode): inline Plugin.define for zero-dependency runtime (Wave 59b) | OTHER | plugins/opencode (TypeScript) | NO | Plugin.define inlined for a zero-dependency runtime; TypeScript plugin packaging change, not Rust. |
| 91 | `a46ba0d3` | 2026-11-02 | fix(rdf,judgment,graphql,rag_chat,ingest): serialize SPARQL Graph results, forward base_iri, and harden fallbacks (Wave 52) | SCHEMA | src/rdf/federation.rs + src/graphql + src/judgment | PARTIAL | SPARQL Graph results were not serialized, base_iri was not forwarded and fallbacks were weak; representation/API-integration defects across bounded handlers. |
| 92 | `c86ca251` | 2026-11-02 | fix(server,store,setup,lexical,time_series): advertise procedure_perform namespace, escape enum Cypher literals, and log query/index errors (Wave 51) | SCHEMA | src/server.rs + src/store + src/lexical.rs | PARTIAL | procedure_perform namespace not advertised and enum Cypher literals unescaped; API-surface/schema alignment across server and store. |
| 93 | `29982e5b` | 2026-11-02 | fix(cli,server,daemon): check /shutdown HTTP status, remove last 3 unguarded .unwrap() calls, and log daemon errors (Wave 50) | ERRH | src/daemon.rs + src/main.rs + src/server.rs | PARTIAL | /shutdown HTTP status went unchecked, three unguarded unwrap()s remained and daemon errors were unlogged; error-handling paths in the async daemon. |
| 94 | `a52b3b6a` | 2026-11-02 | fix(store,cli): forward CLI mutations to daemon, checkpoint on drop/exit, fix qualified-target hygiene, and recover poisoned locks (Wave 49) | ATOM | src/main.rs + src/store/ladybug.rs | PARTIAL | CLI mutations were not forwarded to the daemon and checkpoints were skipped on drop/exit (plus poisoned-lock recovery); lifecycle completeness of a multi-step operation over daemon/FFI seams. |
| 95 | `a9f82844` | 2026-11-02 | fix(wave48): wire intake source_span/extractor_fingerprint, recover poisoned bus mutexes, log snapshot read_dir errors, and align graphql is_retired_at (neurostrata-sfip, neurostrata-74x6) | CONC | multi-area bundle (src/task/intake, src/events/bus, src/graphql, src/storage) | NO | Wave-48 bundle whose distinctive defect is recovering poisoned bus mutexes (locking), alongside intake field wiring and error logging; concurrency-inherent. |
| 96 | `6db0fd43` | 2026-11-02 | fix(wave47): remove read-only hygiene mutation cheat and log Cypher/JSON errors in ladybug.rs, clean up opencode/openclaw plugins, and sync pre-push sha256 marker (neurostrata-fc1v, neurostrata-d3y3, neurostrata-7j7s) | SEQ | src/store/ladybug.rs | PARTIAL | Read-only hygiene path mutated state (the 'mutation cheat') and Cypher/JSON errors went unlogged; sequential side-effect logic over the FFI store. |
| 97 | `dc76a4b2` | 2026-11-02 | fix(wave46): remediate Slice BJ-BN judgment/laya_onnx/ladybug logging, daemon export-graph polarity & storage/status POST, and graphql subscription error (neurostrata-66n0, neurostrata-g8ys, neurostrata-uopw) | SEQ | src/daemon.rs + src/judgment + src/graphql | PARTIAL | Wave-46 bundle whose crisp defect is inverted export-graph polarity, plus logging and a subscription error; sequential logic across daemon/integration seams. |
| 98 | `dc0c3907` | 2026-11-02 | fix(wave45): remove unused _depth in secrets.rs and rename used _namespace in ladybug.rs | OTHER | src/secrets.rs + src/store/ladybug.rs | NO | Wave-45 removes an unused parameter and renames another; dead-code hygiene with no behaviour defect. |
| 99 | `72c4a48b` | 2026-11-02 | fix(wave44): remediate Slice AZ-BG context_plan/task/archive/mesh schemas, registry lazy embed, CLI gate/import/dry-run hardening, and error logging (neurostrata-bahm, neurostrata-1lsx, neurostrata-oo10) | SCHEMA | multi-area bundle (src/server, src/task, src/synaptic_mesh, src/registry, src/parser) | PARTIAL | Wave-44 bundle remediating context_plan/task/archive/mesh schemas plus lazy-embed and CLI hardening; schema-integration dominant. |
| 100 | `fcd00648` | 2026-11-02 | fix(wave43): remediate Slice AV-AY supersede governs/MCP schemas, CLI export-graph/dry-run/lock/crash, neurostrata-wasm tracing/direction/CI, and types_oxrdf/community/rag_chat (neurostrata-tcu0, neurostrata-513r, neurostrata-oqic, neurostrata-2c81) | OTHER | docs (plan document only) | NO | The commit changes only a remediation plan document (+15/-0); no code is modified, so there is no defect fix for Kani to read. |
