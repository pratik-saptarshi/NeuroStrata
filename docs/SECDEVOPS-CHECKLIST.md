# SECDevOps Checklist — Local Quality Governance (Rust)

Owner: repository maintainers. Applies to every Rust project in this
organization (currently: NeuroStrata root workspace; `src-tauri` is a
separate crate — see §5).

## 1. Rule

No push may make Rust quality **worse**. The local gate is
**delta-only**: it fails exclusively on findings located on lines the
push adds, or on dependency advisories the push introduces. Legacy
backlog is owned by `.github/quality-baseline.md` and Task 15b slices —
the gate never demands a tree-wide sweep.

## 2. What runs where

| Check | Where | Scope | Verdict on failure |
|---|---|---|---|
| `rustfmt` per-file unified diff | pre-push hook (`scripts/rust-quality-gate.sh`, wired in `scripts/hooks/pre-push` `PROJECT_GATES` **ahead of the task gate** so task-tool bypasses can't skip it; mirrored in the `SCAFFOLDED_PRE_PUSH_HOOK` const so hook hygiene preserves it) | unified hunks overlapping added lines only (per-file `rustfmt --emit stdout` via stdin — never parse `--check` output, which has no `@@` hunks; matching hunk printed, not `head`) | **block** (`cargo fmt` fixes) |
| `cargo clippy --locked --workspace --all-targets` (short format) | same | diagnostics on added line numbers only (covers workspace members incl. test/example code; audit covers the wasm crate via the root lockfile) | **block** |
| `cargo audit` (human-readable report) | same, manifest changes only | NEW `high`/`critical` IDs vs base lockfile; unresolved base → warn-and-skip (never compare against an empty baseline) | **block** |
| Pushed refs | hook exports `HOOK_PUSH_REFS`; gate uses the matching line for its base | other branches/tags are *not* locally verified (loud note, CI backstop) | warn |
| Attribution set | temp file (env has per-string limits) | — | — |
| Working tree | gate checks the tree as it stands, including uncommitted edits | — | — |
| `cargo deny` | CI report-only (no `deny.toml`; 358 license defaults unresolved) | advisories/bans deltas only | warn |
| `cargo fmt`/`clippy`/`audit`/`deny`/`gui`/win+mac legs | CI report-only | baseline counts | never gate |

Bypass (urgent pushes only, loud and logged):
`RUST_QUALITY_SKIP=1 git push`. CI remains the backstop. Missing
toolchains (`cargo`, `rustfmt`, `cargo-audit`) warn-and-pass with an
install hint rather than blocking — a hook must never brick pushes
from environments without the toolchain.
Range semantics: the gate diffs the push against its base — `@{u}..HEAD`
for linear pushes; after a rebase (remote tip no longer an ancestor)
or with no upstream, the merge-base with `origin/develop`, so an
upstream tree-wide change can never inflate the range into false
blocks. The diff is taken against the working tree (not just committed
HEAD), so uncommitted edits are checked too. When the hook exports
`HOOK_PUSH_REFS`, the matching line supplies the exact base; other
pushed branches/tags are reported, not verified. Last resort
(no origin/develop available, e.g. offline): `HEAD~1`.

## 3. Severity policy (audit)

- **Block:** new `high`/`critical` advisory IDs vs the base-branch
  lockfile (fail-closed only on verified output; offline DB =
  warn-and-pass, never silent).
- **Track (no block):** pre-existing highs (e.g. `RUSTSEC-2026-0194`/
  `0195` quick-xml 0.37.5, capped by parents `sparesults`/`oxrdfxml`
  `^0.37`), `unsound`, `unmaintained`. Each needs a tracking task or
  accept-note; the gate compares ID sets so tracked items never
  re-block, and any *new* ID blocks.
- Informational warnings never block locally.

## 4. Verification record (2026-11-11, NeuroStrata)

- Skip path: no Rust changes → exit 0.
- FAIL path: synthetic badly-formatted + linted lines committed in a
  scratch worktree → gate names exactly the added lines, ignores
  legacy warnings in the same file, exits 1.
- PASS path: comment-only `.rs` addition on warm cache → exit 0.
- Audit delta: live report parses to `{0194, 0195: high}`,
  `{0243-0436, 026-0173: unknown (unmaintained)}`,
  `{026-0486: unknown (unsound)}`; base lockfile identical → no false
  block. Unsound/unmaintained surface as warnings by design (CI audit
  remains backstop).
- Hook active via `bash scripts/install-hooks.sh` (copies
  `scripts/hooks/pre-push` to the common git dir shared by worktrees).
  The `SCAFFOLDED_PRE_PUSH_HOOK` const in `src/task/mod.rs` carries the
  same gates (hash marker kept in sync — `scaffold_is_current_hook`
  test), so hook-hygiene refresh preserves, not removes, enforcement.
- Codex review rounds on PR #93: (1) JSON advisories lack `severity`
  → parse report `Severity:` lines; (2) `rustfmt --check` has no `@@`
  hunks → diff `--emit stdout` output; (3) project guards must precede
  task-gate early exits; (4) clippy needs `--workspace` (later
  extended to `--all-targets` for test/example code); (5) no-upstream
  branches take the merge-base path so first pushes check every
  commit; (6) missing base lockfile warn-skips audit instead of
  false-blocking; (7) other pushed refs are reported, not verified;
  (8) uncommitted edits are checked via worktree diff; (9) attribution
  set travels by temp file, not env. Each fixed, unit-tested, and
  re-validated e2e (FAIL/PASS/audit/uncommitted/foreign-ref paths).
- Codex review (PR #93) caught two vacuous-gate bugs in v1, fixed and
  re-validated: (a) JSON advisory objects carry `cvss` but no
  `severity` key — severity now comes from the report's `Severity:`
  lines; (b) `rustfmt --check` emits no `@@` hunks — the gate now
  diffs `rustfmt --emit stdout` (stdin mode, no filename header)
  output against the file. Parser unit tests + FAIL/PASS e2e
  re-run after each fix.

## 5. Adopting this in another Rust project

1. Copy `scripts/rust-quality-gate.sh` (no hardcoded paths; edition
   auto-detected; base = pushed range, then merge-base with
   `origin/develop`, then `HEAD~1`; worktree-inclusive).
2. Add `./scripts/rust-quality-gate.sh` to `PROJECT_GATES` in that
   repo's pre-push hook (before slow gates; fmt fails fast).
3. Run that repo's hook installer; verify the installed copy.
4. If the project is a multi-crate workspace with separate lockfiles
   (like `src-tauri` here), extend the audit step per lockfile or
   record the exclusion with reason.
5. Ensure `cargo-audit` is installed where pushes happen
   (`cargo install cargo-audit`); without it the audit step
   warn-passes by design.

## 6. Maintenance

- This checklist, not scattered `#[allow]`s, is where policy lives
  (same rule as the quality baseline: allowlists live in exactly one
  named place).
- Promoting any report-only CI leg to required happens only when its
  baseline count reaches zero — same bar as Task 15b.
- Changing gate semantics = code review on this file + the script.
  Never bypass the gate to land a PR; fix or `SKIP` loudly instead.

## 7. Prior art: rocinante vs tachi-rust vs this gate

Same goal (no red trunk, no bypass culture), three enforcement points:

- **rocinante — CI-required, scope-aware, single aggregator.** No
  local hooks. `ci.yml` has zero `continue-on-error`: `rust-quality-
  gates` (`ci_gate_tests`), `rust-lint` (fmt+clippy matrix),
  `rust-tests`, coverage and platform lanes all gate. A `ci-scope`
  job does path-based delta/release profiling (PRs get delta scope,
  `main`/`release/*` force full scope), and one aggregator `test`
  job ("All required CI stages completed") is the required status
  check alongside `codeql`. Supply chain is exception-proofed:
  `.cargo/audit.toml` ignore-list plus `check-security-advisory-
  exceptions.py` / `dependency-floor-proof.sh`. Lesson for us: an
  aggregator job shrinks required-context sprawl, and a checked-in
  ignore-list with proof scripts beats ad-hoc tracking comments.
- **tachi-rust — shift-left local gates + strict per-concern CI.**
  Local: `.pre-commit-config.yaml` (gitleaks wrapper + file-scoped
  local hooks so clean commits pay zero cost), `Makefile *-gate`
  targets (`supply-chain-gate`: `cargo audit` + `cargo deny check
  advisories bans licenses sources`; `workflow-gate` pin hygiene;
  `gitleaks-gate`), and manifest-driven `scripts/ci-local-runner.sh`
  (`local-full` vs `local-route-equivalent`) for local/CI parity.
  CI: dedicated `rustfmt.yml` (hard `--check` fail), `rust-clippy.yml`
  (`-D warnings` + SARIF), `rust-supply-chain.yml` (pinned
  cargo-audit 0.22.2 / cargo-deny 0.20.2, full deny policy incl.
  licenses+sources). Rulesets gate on scan/quality results
  (`code_quality`, CodeQL high+) with an *empty* required-checks
  list — enforcement at the finding level, not named jobs. Lesson
  for us: absolute gates work there because the tree is clean; our
  baseline debt is why this gate is delta-based instead.
- **This gate — local delta pre-push.** Fills the gap the other two
  designs assume away: a tree with 212 fmt hunks / 210 clippy
  diagnostics / 371 deny errors cannot adopt absolute gates without
  a tree-wide sweep (forbidden by Task 15b), and CI report-only
  means nothing stops new violations. Added-line attribution +
  new-advisory-vs-base is the strictest posture compatible with
  inherited debt. Borrow next (not in this PR): rocinante-style
  aggregator job, checked-in advisory ignore-list with proof, a
  `rust-toolchain.toml` pin, and `make quality-gate` discoverability.
