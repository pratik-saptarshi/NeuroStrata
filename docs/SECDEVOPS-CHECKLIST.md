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
| `rustfmt --check` (edition from `Cargo.toml`) | pre-push hook (`scripts/rust-quality-gate.sh`, wired in `scripts/hooks/pre-push` `PROJECT_GATES`) | hunks overlapping added lines only | **block** (`cargo fmt` fixes) |
| `cargo clippy --locked` (short format) | same | diagnostics on added lines only; unbuildable tree blocks unconditionally | **block** |
| `cargo audit` (high/critical) | same, manifest changes only | advisory IDs absent from `origin/develop` lockfile | **block** |
| `cargo deny` | CI report-only (no `deny.toml`; 358 license defaults unresolved) | advisories/bans deltas only | warn |
| `cargo fmt`/`clippy`/`audit`/`deny`/`gui`/win+mac legs | CI report-only | baseline counts | never gate |

Bypass (urgent pushes only, loud and logged):
`RUST_QUALITY_SKIP=1 git push`. CI remains the backstop. Missing
toolchains (`cargo`, `rustfmt`, `cargo-audit`) warn-and-pass with an
install hint rather than blocking — a hook must never brick pushes
from environments without the toolchain.

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
- Audit delta: HEAD vs `origin/develop` lockfiles both report exactly
  `{RUSTSEC-2026-0194, RUSTSEC-2026-0195}` → no false block.
- Hook active via `bash scripts/install-hooks.sh` (copies
  `scripts/hooks/pre-push` to the common git dir shared by worktrees).

## 5. Adopting this in another Rust project

1. Copy `scripts/rust-quality-gate.sh` (no hardcoded paths; edition
   auto-detected; uses `@{u}..HEAD`, falls back to `HEAD~1..HEAD`).
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
