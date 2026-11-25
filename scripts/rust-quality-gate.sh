#!/bin/bash
# scripts/rust-quality-gate.sh
# Pre-push guard: strict Rust quality gate on changed code only (delta-based).
#
# Philosophy (matches task-coverage-gate.sh and Task 15b): the gate NEVER
# blocks on legacy backlog — .github/quality-baseline.md owns that. It
# fails only when THIS push makes quality worse:
#   1. per-file rustfmt output differs on added lines (a real unified
#      diff is generated via rustfmt --emit stdout piped through stdin —
#      rustfmt --check's own format has no @@ headers and its file-arg
#      mode prepends a filename header, so neither may be parsed naively).
#   2. `cargo clippy` reports warnings/errors located on added lines
#      (an unbuildable tree with zero diagnostics fails regardless).
#   3. the human-readable `cargo audit` report shows high/critical
#      advisories NOT already in the base lockfile (severity comes from
#      the report's `Severity:` lines — JSON advisories carry `cvss`
#      but no `severity` key, so JSON severity filtering is vacuous).
#
# Runs only when the push touches Rust sources or manifests; otherwise it
# exits 0 silently. Bypass for urgent pushes (loud, logged):
#   RUST_QUALITY_SKIP=1 git push   # CI remains the backstop
#
# Install: registered in scripts/hooks/pre-push PROJECT_GATES; activate
# with `bash scripts/install-hooks.sh` (idempotent).

set -u

GATE="rust-quality-gate"

command -v git >/dev/null 2>&1 || { echo "$GATE: no git — skipping"; exit 0; }

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
cd "$ROOT" || exit 0

# Cargo output is parsed programmatically below (clippy diagnostics
# in §2/§2b, the cargo audit report in §3). A user-set
# CARGO_TERM_COLOR=always makes cargo wrap diagnostic severities in
# ANSI escapes (`src/lib.rs:6:5: ESC[1mESC[33mwarningESC[0m:`), so
# the diagnostic regex never matches and warnings on added lines are
# silently missed — the gate passes code it never verified. Force
# color off for every cargo invocation this script makes (the
# clippy/audit invocations also carry --color never, so the parse
# sites stay protected even if this export is ever lost).
export CARGO_TERM_COLOR=never

if [ "${RUST_QUALITY_SKIP:-0}" = "1" ]; then
    echo "$GATE: SKIPPED (RUST_QUALITY_SKIP=1) — quality NOT verified for this push" >&2
    exit 0
fi

# ── Change base: what is this push relative to? ─────────────────────────
# The hook exports HOOK_PUSH_REFS (one "<local ref> <local sha>
# <remote ref> <remote sha>" line per updated ref). Prefer the exact
# pushed range when pushing the checked-out branch; warn loudly about
# any other pushed ref (other branches, tags), which is NOT locally
# verified — CI remains the backstop for those. Otherwise fall back to
# the merge-base with the integration target, so the range is always
# the branch's own delta: after a rebase the remote tip predates the
# rewritten base and @{u}..HEAD would balloon to include upstream
# changes (e.g. a tree-wide reformat), false-blocking as "added" lines.
# A branch with no upstream takes the same merge-base path, so first
# pushes check every commit, not just HEAD~1. Last resort
# (no origin/develop available, e.g. offline): the last commit only —
# except a root commit, which has no parent: there HEAD~1 does not
# exist, so the baseline is the git empty tree and the initial commit
# diffs in full (the first push of a freshly bootstrapped project).
BASE=""
CURBRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
HEAD_SHA="$(git rev-parse HEAD 2>/dev/null || true)"
if [ -n "${HOOK_PUSH_REFS:-}" ]; then
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        lref="${line%% *}"; rest="${line#* }"
        lsha="${rest%% *}"; rest="${rest#* }"
        rref="${rest%% *}"; rsha="${rest##* }"
        [ -n "$lref" ] || continue
        case "$lsha" in
            0000000000000000000000000000000000000000) continue ;; # branch deletion: nothing to check
        esac
        if [ "$lref" = "refs/heads/$CURBRANCH" ] && [ "$lsha" = "$HEAD_SHA" ] && [ -n "$HEAD_SHA" ]; then
            # Trust the pushed base only when history is linear from it:
            # after a rebase the old remote tip still exists but is no
            # longer an ancestor, and using it would attribute upstream
            # changes to this push. Unknown SHAs fall through to the
            # merge-base path below (fail-closed, never vacuous).
            if [ "$rsha" != "0000000000000000000000000000000000000000" ] \
                && git cat-file -e "$rsha^{commit}" 2>/dev/null \
                && git merge-base --is-ancestor "$rsha" HEAD 2>/dev/null; then
                BASE="$rsha"
            fi
        else
            echo "$GATE: note: pushed ref $lref is not the checked-out HEAD — not locally verified (CI remains backstop)" >&2
        fi
    done <<< "$HOOK_PUSH_REFS"
fi
if [ -z "$BASE" ] || [ "$BASE" = "0000000000000000000000000000000000000000" ] || ! git cat-file -e "$BASE^{commit}" 2>/dev/null; then
    BASE=""
    if git rev-parse --verify --quiet "@{u}" >/dev/null 2>&1 \
        && git merge-base --is-ancestor "@{u}" HEAD 2>/dev/null; then
        BASE="@{u}"
    else
        BASE="$(git merge-base HEAD origin/develop 2>/dev/null || true)"
        if [ -z "$BASE" ]; then
            if git rev-parse --verify --quiet "HEAD~1" >/dev/null 2>&1; then
                BASE="HEAD~1"
            else
                # Root commit: no parent exists, so diff against the
                # git empty tree — the initial commit is evaluated in
                # full instead of failing on a nonexistent baseline.
                BASE="4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            fi
        fi
    fi
fi

# The gate checks the WORKING TREE as it stands (not just committed
# HEAD): rustfmt/clippy/audit all inspect worktree files, so attribution
# must use worktree numbering too — otherwise uncommitted edits could
# hide findings in the code actually being pushed (or vice versa).
# Every git diff below carries --no-color --no-ext-diff --no-textconv
# --text: the gate parses diff output programmatically, so user git
# config and attributes must never be able to reshape it (see the
# attribution diff below).
CHANGED_RS="$(git diff --no-color --no-ext-diff --no-textconv --text --name-only "$BASE" -- '*.rs')" || {
    echo "$GATE: FAIL: cannot compute the changed-file set against $BASE — quality NOT verified for this push" >&2
    exit 1
}
# Manifest detection is tree-wide, not a hardcoded crate-directory
# list: a bootstrapped repo may keep its crate under ANY directory
# (native/, tools/foo/, ...), and a manifest/lockfile-only push there
# changes the dependency set exactly like a root one. The glob-magic
# pathspecs match Cargo.toml/Cargo.lock at the root and at any depth
# (plain '**/Cargo.toml' would miss the root files). Every relevance
# decision below (clippy §2/§2c, the src-tauri leg §2b, the audit
# trigger §3) consumes this same set, so no layout is gate-relevant
# for one leg and invisible to another.
CHANGED_MANIFEST="$(git diff --no-color --no-ext-diff --no-textconv --text --name-only "$BASE" -- ':(glob)**/Cargo.toml' ':(glob)**/Cargo.lock')" || {
    echo "$GATE: FAIL: cannot compute the changed-manifest set against $BASE — quality NOT verified for this push" >&2
    exit 1
}

if [ -z "$CHANGED_RS" ] && [ -z "$CHANGED_MANIFEST" ]; then
    echo "$GATE: no Rust sources or manifests changed — nothing to check"
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "$GATE: warning: cargo not found in PATH — gate NOT run for this push (CI remains backstop)" >&2
    exit 0
fi

FAIL=0

# python3 is a hard prerequisite, not an optional filter: it builds the
# added-line attribution set below and parses every finding filter
# (fmt overlap, audit advisories). If it is missing or unusable, those
# filters would run on silently emptied inputs and the gate could
# report a pass it never verified — FAIL CLOSED instead, before any
# code path can reach the pass verdict.
if ! command -v python3 >/dev/null 2>&1 || ! python3 -c 'import sys' >/dev/null 2>&1; then
    echo "$GATE: FAIL: python3 is required to attribute findings to added lines, but it is missing or unusable in PATH — quality NOT verified for this push" >&2
    exit 1
fi

# Added-line set for every touched .rs file (format "path:lineno",
# worktree numbering — the same files rustfmt/clippy inspect). Both the
# fmt and clippy attributions below match findings against this set, so
# legacy findings in touched files never block the push — only lines
# this push adds. Mirrors task-coverage-gate.sh (+-line inspection).
# Kept in a temp file, not the environment: large diffs exceed the
# per-string environment limit and python would fail to start. The
# pipeline's exit status is checked explicitly below: a failed
# attribution run must fail closed, never pass on an empty set.
# --default-prefix is load-bearing: user config such as
# diff.noprefix=true (or custom src/dst prefixes) would otherwise
# make Git emit `+++ src/lib.rs` instead of `+++ b/src/lib.rs`,
# the parser below would never set its current file, and the
# attribution set would silently come out empty — bypassing both
# the fmt and clippy checks with no logged skip.
# --no-color --no-ext-diff --no-textconv --text are load-bearing
# for the same reason: color.ui/color.diff=always wraps every line
# in ANSI escapes (the `+++ b/` header no longer matches at line
# start), diff.external (or GIT_EXTERNAL_DIFF) hands the diff to an
# external driver whose output is not unified diff at all, a
# textconv driver (diff.<driver>.textconv) rewrites file contents
# before diffing, and a `-diff` attribute makes Git emit a
# binary-change notice instead of hunks — each silently empties
# the attribution set the same way.
ADDED_FILE="$(mktemp -t rustgate-added.XXXXXX)"
trap 'rm -f "$ADDED_FILE"' EXIT
git diff --default-prefix --no-color --no-ext-diff --no-textconv --text -U0 "$BASE" -- '*.rs' | python3 -c '
import re, sys
cur, new_line, out = None, 0, set()
for raw in sys.stdin:
    line = raw.rstrip("\n")
    if line.startswith("+++ b/"):
        cur, new_line = line[6:], 0
        continue
    m = re.match(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", line)
    if m:
        new_line = int(m.group(1))
        continue
    if cur is None:
        continue
    if line.startswith("+") and not line.startswith("+++"):
        out.add(f"{cur}:{new_line}")
        new_line += 1
    elif line.startswith(" "):
        new_line += 1
print("\n".join(sorted(out)))
' > "$ADDED_FILE" 2>/dev/null
ADDED_RC=${PIPESTATUS[1]}
if [ "$ADDED_RC" -ne 0 ]; then
    echo "$GATE: FAIL: could not build the added-line attribution set (python3 exited $ADDED_RC) — quality NOT verified for this push" >&2
    exit 1
fi

# ── 1. fmt: fail only when added lines are misformatted ──────────────────
# rustfmt --check prints its OWN diff format ("Diff in <file>:<line>:" +
# bare -/+ lines, no unified @@ headers), so it must NOT be parsed as
# unified diff. Instead generate a real unified diff against rustfmt's
# stdout and reuse the added-line attribution below. NOTE: the file is
# piped via stdin — `rustfmt --emit stdout <file>` prepends a filename
# header that would diff-dirty every file; stdin mode emits clean output.
EDITION="$(grep -m1 '^edition[[:space:]]*=' Cargo.toml 2>/dev/null | grep -oE '"[^"]+"' | tr -d '"' || true)"
[ -n "$EDITION" ] || EDITION="2021"
if ! command -v rustfmt >/dev/null 2>&1; then
    echo "$GATE: warning: rustfmt not found — fmt NOT verified (run \`cargo fmt\` before pushing; CI fmt remains backstop)" >&2
else
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        [ -f "$f" ] || continue
        FORMATTED="$(rustfmt --edition "$EDITION" --emit stdout < "$f" 2>/dev/null || true)"
        if [ -z "$FORMATTED" ]; then
            echo "$GATE: warning: rustfmt could not format $f (syntax?) — fmt NOT verified for this file (clippy/build remain backstop)" >&2
            continue
        fi
        FMT_DIFF="$(diff -U0 "$f" <(printf '%s\n' "$FORMATTED") 2>/dev/null || true)"
        [ -n "$FMT_DIFF" ] || continue
        OVERLAP="$(printf '%s\n' "$FMT_DIFF" | FILE="$f" ADDED_FILE="$ADDED_FILE" python3 -c '
import os, re, sys
f = os.environ.get("FILE", "")
added = set()
try:
    with open(os.environ.get("ADDED_FILE", "")) as fh:
        for ln in fh:
            ln = ln.strip()
            if ln.startswith(f + ":"):
                added.add(ln)
except OSError:
    pass
out, cur, hit = [], [], False
def flush():
    global out, cur, hit
    if hit:
        out.extend(cur)
    cur, hit = [], False
for raw in sys.stdin:
    line = raw.rstrip("\n")
    m = re.match(r"^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@", line)
    if m:
        flush()
        s, c = int(m.group(1)), int(m.group(2) or 1)
        hit = any(f"{f}:{n}" in added for n in range(s, s + c))
    cur.append(line)
flush()
print("MATCH" if out else "no")
print("\n".join(out))
')"
        OVERLAP_RC=$?
        if [ "$OVERLAP_RC" -ne 0 ]; then
            echo "$GATE: FAIL: could not attribute rustfmt findings for $f (python3 exited $OVERLAP_RC) — fmt NOT verified for this file" >&2
            FAIL=1
            continue
        fi
        if [ "$(printf '%s\n' "$OVERLAP" | head -n 1)" = "MATCH" ]; then
            echo "$GATE: FAIL: touched file needs formatting on added lines (run \`cargo fmt\`): $f" >&2
            printf '%s\n' "$OVERLAP" | tail -n +2 | head -n 20 >&2
            FAIL=1
        fi
    done <<< "$CHANGED_RS"
fi

# ── 2. clippy: fail only on diagnostics located on added lines ──────────
# A broken build (no parseable diagnostics) fails regardless — quality
# cannot be verified. Deny-level lints elsewhere in the tree are backlog
# (same rule as warnings): only added lines gate the push.
# (ADDED_FILE computed once above and reused here.)
# --workspace: the root package alone would skip workspace members
# (e.g. crates/neurostrata-wasm, which ships without its own lockfile
# and is covered by the root lockfile for audit). --all-targets: the
# default lib+bins scope skips #[cfg(test)] modules, integration tests
# in tests/, and examples/ — new test/example code must be linted too.
# --all-features: without it, feature-gated code is never compiled and
# gets no coverage at all (e.g. src/synaptic_mesh/nats.rs behind the
# root package's `nats-mesh` feature). The workspace's feature set is
# safe to enable wholesale: `nats-mesh = []` enables no dependencies,
# and crates/ontobug's `cli` only enables its optional clap dependency
# (already in the lockfile); no feature pulls system dependencies or
# is mutually exclusive with another. Diagnostics stay attributed per
# added line below, so the wider scope costs build time, not false
# blocks — pre-existing warnings in newly-visible gated code sit on
# untouched lines and never block.
# Shared attribution for one clippy run: $1 = raw output, $2 = exit
# code, $3 = subject for the build-failure message, $4 = path prefix
# cargo emitted diagnostic paths relative to ("" for the root
# workspace run; the crate's directory for --manifest-path runs,
# whose paths come out relative to the manifest's directory and must
# be prefixed back to repo-root form before matching ADDED_FILE).
# Same fail-closed rule everywhere: no diagnostics plus a nonzero
# exit means quality cannot be verified.
clippy_leg() {
    local out="$1" code="$2" subject="$3" prefix="$4"
    local diags diag dpath dline
    diags="$(printf '%s\n' "$out" | grep -E ':[0-9]+:[0-9]+: (warning|error)' || true)"
    if [ -z "$diags" ] && [ "$code" -ne 0 ]; then
        echo "$GATE: FAIL: $subject does not build — clippy produced no diagnostics (exit $code)" >&2
        printf '%s\n' "$out" | tail -n 20 >&2
        FAIL=1
        return
    fi
    [ -n "$diags" ] || return 0
    while IFS= read -r diag; do
        [ -n "$diag" ] || continue
        if [ -n "$prefix" ]; then
            case "$diag" in
                "$prefix"*) ;;
                *) diag="$prefix$diag" ;;
            esac
        fi
        # short format: path:line:col: warning|error: message
        dpath="$(printf '%s' "$diag" | grep -oE '^[a-zA-Z0-9_./+-]+\.rs' || true)"
        dline="$(printf '%s' "$diag" | grep -oE '^[a-zA-Z0-9_./+-]+\.rs:[0-9]+' | grep -oE '[0-9]+$' || true)"
        if [ -n "$dpath" ] && [ -n "$dline" ] \
            && grep -qxF "$dpath:$dline" "$ADDED_FILE" 2>/dev/null; then
            echo "$GATE: FAIL: new clippy diagnostic on added line: $diag" >&2
            FAIL=1
        fi
    done <<< "$diags"
}

if [ -f Cargo.toml ]; then
    CLIPPY_OUT="$(cargo clippy --locked --workspace --all-targets --all-features --message-format=short --color never 2>&1)"
    clippy_leg "$CLIPPY_OUT" "$?" "workspace" ""
else
    # No root manifest: a bootstrapped repo may keep its crate under a
    # subdirectory (e.g. native/) with no root Cargo.toml at all. The
    # workspace invocation above then fails with "could not find
    # `Cargo.toml`" and blocks EVERY push, clean or not. Instead,
    # resolve the owning manifest for each changed .rs file — the
    # nearest Cargo.toml in the file's directory or any ancestor up
    # to the repo root — and run clippy once per distinct owning
    # manifest. A changed file with no owning manifest anywhere above
    # it is reported NOT verified rather than silently skipped; if no
    # changed file has an owning manifest, fall back to the root
    # workspace invocation, which fails closed exactly as before.
    OWNING_MANIFESTS=""
    UNCOVERED_RS=""
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        d="$(dirname "$f")"
        found=""
        while :; do
            if [ -f "$d/Cargo.toml" ]; then
                found="$d/Cargo.toml"
                break
            fi
            [ "$d" = "." ] && break
            d="$(dirname "$d")"
        done
        if [ -n "$found" ]; then
            OWNING_MANIFESTS="${OWNING_MANIFESTS}${found}
"
        else
            UNCOVERED_RS="${UNCOVERED_RS}${f}
"
        fi
    done <<< "$CHANGED_RS"
    # A manifest/lockfile-ONLY push changes no .rs file, so the loop
    # above finds no owning manifest and the fallback below would
    # fail closed on the missing root manifest without ever checking
    # the crate that actually changed. Derive the owning manifests
    # from the changed manifests themselves: a changed Cargo.toml is
    # its own owning manifest, and a changed Cargo.lock is owned by
    # the sibling Cargo.toml beside it (when one exists).
    while IFS= read -r p; do
        [ -n "$p" ] || continue
        case "$p" in
            */Cargo.toml|Cargo.toml)
                [ -f "$p" ] && OWNING_MANIFESTS="${OWNING_MANIFESTS}${p}
"
                ;;
            */Cargo.lock|Cargo.lock)
                sib="${p%Cargo.lock}Cargo.toml"
                [ -f "$sib" ] && OWNING_MANIFESTS="${OWNING_MANIFESTS}${sib}
"
                ;;
        esac
    done <<< "$CHANGED_MANIFEST"
    OWNING_MANIFESTS="$(printf '%s' "$OWNING_MANIFESTS" | sort -u)"
    if [ -z "$OWNING_MANIFESTS" ]; then
        CLIPPY_OUT="$(cargo clippy --locked --workspace --all-targets --all-features --message-format=short --color never 2>&1)"
        clippy_leg "$CLIPPY_OUT" "$?" "workspace" ""
    else
        while IFS= read -r m; do
            [ -n "$m" ] || continue
            mdir="$(dirname "$m")"
            mprefix=""
            [ "$mdir" != "." ] && mprefix="$mdir/"
            M_OUT="$(cargo clippy --locked --manifest-path "$m" --all-targets --all-features --message-format=short --color never 2>&1)"
            clippy_leg "$M_OUT" "$?" "crate at $m" "$mprefix"
        done <<< "$OWNING_MANIFESTS"
        while IFS= read -r f; do
            [ -n "$f" ] || continue
            echo "$GATE: warning: no Cargo.toml found above $f — clippy NOT verified for this file (CI remains backstop)" >&2
        done <<< "$UNCOVERED_RS"
    fi
fi

# ── 2c. clippy (changed nested manifests, root workspace present) ──────
# With a root Cargo.toml, §2's workspace run covers workspace members,
# but a manifest/lockfile-only push in a nested crate changes no .rs
# file, so nothing above ever builds that crate on its own — and a
# nested crate outside the workspace (a bootstrapped layout next to a
# root crate) is not covered by the workspace run at all. Run clippy
# once per distinct owning manifest derived from the changed
# manifests (a changed Cargo.toml owns itself; a changed Cargo.lock
# is owned by its sibling Cargo.toml), so a broken or unresolvable
# dependency introduced by a manifest-only push fails closed here.
# Diagnostics are attributed exactly like §2's nested runs (manifest
# dir prefixed back to repo-root form); a manifest-only push adds no
# .rs lines, so in practice this leg fires on build failure, which is
# the point. Root and src-tauri manifests are excluded: §2 and §2b
# own those.
if [ -f Cargo.toml ]; then
    CHANGED_OWNING=""
    while IFS= read -r p; do
        [ -n "$p" ] || continue
        case "$p" in
            */Cargo.toml) cand="$p" ;;
            */Cargo.lock) cand="${p%Cargo.lock}Cargo.toml" ;;
            *) continue ;;
        esac
        case "$cand" in
            src-tauri/Cargo.toml) continue ;;
        esac
        [ -f "$cand" ] && CHANGED_OWNING="${CHANGED_OWNING}${cand}
"
    done <<< "$CHANGED_MANIFEST"
    CHANGED_OWNING="$(printf '%s' "$CHANGED_OWNING" | sort -u)"
    while IFS= read -r m; do
        [ -n "$m" ] || continue
        mdir="$(dirname "$m")"
        M_OUT="$(cargo clippy --locked --manifest-path "$m" --all-targets --all-features --message-format=short --color never 2>&1)"
        clippy_leg "$M_OUT" "$?" "crate at $m" "$mdir/"
    done <<< "$CHANGED_OWNING"
fi

# ── 2b. clippy (src-tauri): the excluded standalone crate ────────────────
# The root workspace EXCLUDES src-tauri (standalone package, own
# lockfile), so the --workspace run above never checks it: a push
# touching only src-tauri sources passed with new warnings there.
# When this push touches src-tauri .rs files or its manifests, run
# clippy for that crate on its own and attribute its diagnostics
# exactly like the workspace ones. The manifests count as a touch:
# a dependency-only change (src-tauri/Cargo.toml or Cargo.lock) can
# break the crate or change what it compiles — the tree-wide glob
# detection above names those files in CHANGED_MANIFEST, and this
# leg keys off that set, so a Tauri-only manifest push is checked
# here instead of exiting above as "no Rust sources or manifests
# changed".
# Cargo emits its paths relative to src-tauri/
# (src/...), so prefix them back to repo-root form before matching
# ADDED_FILE. Same fail-closed rule as the workspace run: no
# diagnostics plus a nonzero exit means quality cannot be verified.
# Deliberately NO --all-features here, unlike the workspace run: the
# crate's only feature is `custom-protocol`, which gates no src-tauri
# source (no cfg(feature) in src/) — it only flips the tauri
# dependency into release/frontend-bundled build mode, so enabling it
# would add build-mode risk with zero lint-coverage gain.
TAURI_CHANGED="$(printf '%s\n' "$CHANGED_RS" | grep '^src-tauri/' || true)"
TAURI_MANIFEST_CHANGED="$(printf '%s\n' "$CHANGED_MANIFEST" | grep -E '^src-tauri/Cargo\.(toml|lock)$' || true)"
if [ -n "$TAURI_CHANGED" ] || [ -n "$TAURI_MANIFEST_CHANGED" ]; then
    TAURI_OUT="$(cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --message-format=short --color never 2>&1)"
    TAURI_CODE=$?
    TAURI_DIAG="$(printf '%s\n' "$TAURI_OUT" | grep -E ':[0-9]+:[0-9]+: (warning|error)' | sed -e '/^src-tauri\//b' -e 's|^|src-tauri/|' || true)"
    if [ -z "$TAURI_DIAG" ] && [ "$TAURI_CODE" -ne 0 ]; then
        echo "$GATE: FAIL: src-tauri crate does not build — clippy produced no diagnostics (exit $TAURI_CODE)" >&2
        printf '%s\n' "$TAURI_OUT" | tail -n 20 >&2
        FAIL=1
    elif [ -n "$TAURI_DIAG" ]; then
        while IFS= read -r diag; do
            [ -n "$diag" ] || continue
            dpath="$(printf '%s' "$diag" | grep -oE '^[a-zA-Z0-9_./+-]+\.rs' || true)"
            dline="$(printf '%s' "$diag" | grep -oE '^[a-zA-Z0-9_./+-]+\.rs:[0-9]+' | grep -oE '[0-9]+$' || true)"
            if [ -n "$dpath" ] && [ -n "$dline" ] \
                && grep -qxF "$dpath:$dline" "$ADDED_FILE" 2>/dev/null; then
                echo "$GATE: FAIL: new clippy diagnostic on added line: $diag" >&2
                FAIL=1
            fi
        done <<< "$TAURI_DIAG"
    fi
fi

# ── 3. audit: fail only on NEW high/critical advisories vs base ──────────
# Severity source: the human-readable `cargo audit` report (`Severity:
# 7.5 (high)`). The JSON advisory objects carry `cvss` but NO `severity`
# key, so filtering JSON on severity silently matches nothing and the
# gate would never fire. Both sides parsed from text; IDs compared.
# The comparison runs PER LOCKFILE this push touches: for every
# changed Cargo.lock, and for the sibling lockfile of every changed
# Cargo.toml (a manifest-only edit is audited through the lockfile it
# resolves against). Each lockfile's baseline is the SAME path at the
# resolved push baseline ($BASE), never origin/develop: a branch
# forked before an upstream advisory fix has that advisory in its
# merge-base lockfile too, and diffing against origin/develop would
# block it for an advisory it inherited rather than introduced.
parse_advisories() {
    printf '%s' "$1" | python3 -c '
import re, sys
text = sys.stdin.read()
for block in re.split(r"\n\s*\n", text):
    m_id = re.search(r"^ID:\s*(\S+)", block, re.M)
    if not m_id:
        continue
    m_sev = re.search(r"^Severity:\s*(?:[\d.]+\s*)?\((\w+)\)", block, re.M)
    if not m_sev:
        m_sev = re.search(r"^Severity:\s*(\w+)", block, re.M)
    print(m_id.group(1), (m_sev.group(1).lower() if m_sev else "unknown"))
' 2>/dev/null
}

# audit_leg <lockfile-path>: audit one lockfile's current state and
# diff its high/critical advisory IDs against the same path at $BASE.
audit_leg() {
    local lock="$1"
    local audit_text audit_rc parsed parse_rc new_ids
    local base_sev base_ok tmpbase base_text base_rc aid
    if [ "$lock" = "Cargo.lock" ]; then
        # (no `set -e` in this script, so a failing substitution is safe
        # to inspect via $? on the next line)
        audit_text="$(cargo audit --color never 2>/dev/null)"
        audit_rc=$?
    else
        audit_text="$(cargo audit --file "$lock" --color never 2>/dev/null)"
        audit_rc=$?
    fi
    if [ -z "$audit_text" ] && [ "$audit_rc" -eq 0 ]; then
        # Clean tree: cargo audit prints nothing on success. An empty
        # report with exit 0 is a verified-clean result, not missing
        # data — do not confuse it with the offline case below.
        return 0
    fi
    if [ -z "$audit_text" ]; then
        echo "$GATE: warning: cargo audit produced no output (offline DB?) — audit NOT verified" >&2
        return 0
    fi
    # A failed parse must never read as "no advisories": warn
    # loudly that the audit is NOT verified (this section's
    # established unverifiable-audit posture) instead of
    # proceeding on an emptied advisory set.
    parsed="$(parse_advisories "$audit_text")"
    parse_rc=$?
    new_ids=""
    if [ "$parse_rc" -ne 0 ]; then
        echo "$GATE: warning: could not parse the cargo audit report (python3 exited $parse_rc) — audit NOT verified (CI audit remains backstop)" >&2
        return 0
    fi
    new_ids="$(printf '%s\n' "$parsed" | awk '$2=="high"||$2=="critical"{print $1}')"
    if [ -z "$new_ids" ]; then
        echo "$GATE: no new high/critical advisories in this push"
        return 0
    fi
    base_sev=""
    base_ok=0
    if [ "$BASE" = "4b825dc642cb6eb9a060e54bf8d69288fbee4904" ]; then
        # Empty-tree baseline (first push, root commit):
        # there is no baseline lockfile — that is verified
        # by the baseline's identity, not missing data. A
        # root commit is evaluated in full (the same
        # added-content rule fmt/clippy apply above), so
        # the baseline advisory set is empty and every
        # high/critical advisory in the push counts as new.
        base_ok=1
    elif git cat-file -e "$BASE:$lock" 2>/dev/null; then
        # The baseline HAS a lockfile at this path: audit it. (A failed
        # `git show` below, or a failed audit/parse of what it shows,
        # leaves base_ok=0 — an existing-but-unverifiable baseline
        # keeps the warn-only NOT-verified posture, never a pass.)
        tmpbase="$(mktemp -t rustgate-base-lock.XXXXXX)"
        if git show "$BASE:$lock" > "$tmpbase" 2>/dev/null; then
            base_text="$(cargo audit --file "$tmpbase" --color never 2>/dev/null)"
            base_rc=$?
            if [ "$base_rc" -eq 0 ]; then
                # Verified-clean baseline: cargo audit prints
                # nothing on success. An empty set here is
                # evidence of absence, not missing data — any
                # high/critical ID in the push is therefore new.
                base_ok=1
            elif [ -n "$base_text" ]; then
                # Only a successful parse counts as a verified
                # baseline; a failed one leaves base_ok=0 and
                # falls into the NOT-verified warning below.
                if base_sev="$(parse_advisories "$base_text")"; then
                    base_ok=1
                fi
            fi
        fi
        rm -f "$tmpbase"
    elif git cat-file -e "$BASE^{commit}" 2>/dev/null; then
        # Verified-absent baseline: the resolved baseline commit
        # EXISTS and positively has NO lockfile at this path (the
        # cat-file probe above failed on an existing commit — not a
        # failed show or a parse failure, which look identical to
        # missing data if only `git show` is consulted). This is the
        # empty-tree precedent generalized: a repo whose baseline is
        # non-Rust (or whose crate at this path is new) has a
        # verified-empty advisory baseline, so every high/critical
        # advisory in the newly added lockfile counts as new and
        # blocks — instead of warning "audit NOT verified" and
        # passing (fail-open) on the very push that introduces the
        # dependency set.
        base_ok=1
        base_sev=""
    fi
    if [ "$base_ok" != "1" ]; then
        echo "$GATE: warning: base lockfile audit unavailable — cannot diff advisory baseline; audit NOT verified (CI audit remains backstop)" >&2
        return 0
    fi
    while IFS= read -r aid; do
        [ -n "$aid" ] || continue
        if ! printf '%s\n' "$base_sev" | grep -qE "^${aid}[[:space:]]"; then
            echo "$GATE: FAIL: new high/critical advisory introduced by this push: $aid" >&2
            FAIL=1
        fi
    done <<< "$new_ids"
}

if [ -n "$CHANGED_MANIFEST" ]; then
    if ! command -v cargo-audit >/dev/null 2>&1 && ! cargo audit --version >/dev/null 2>&1; then
        echo "$GATE: warning: cargo-audit not installed — audit NOT run (run \`cargo install cargo-audit\`; CI audit remains backstop)" >&2
    else
        AUDIT_LOCKS=""
        while IFS= read -r p; do
            [ -n "$p" ] || continue
            case "$p" in
                */Cargo.lock|Cargo.lock)
                    AUDIT_LOCKS="${AUDIT_LOCKS}${p}
"
                    ;;
                */Cargo.toml|Cargo.toml)
                    sib="${p%Cargo.toml}Cargo.lock"
                    [ -f "$sib" ] && AUDIT_LOCKS="${AUDIT_LOCKS}${sib}
"
                    ;;
            esac
        done <<< "$CHANGED_MANIFEST"
        AUDIT_LOCKS="$(printf '%s' "$AUDIT_LOCKS" | sort -u)"
        if [ -z "$AUDIT_LOCKS" ]; then
            # A manifest changed but no lockfile exists anywhere it
            # resolves against (e.g. the lockfile itself was deleted
            # by this push): audit the root path anyway — cargo audit
            # then reports NOT verified rather than the push passing
            # with the audit leg silently skipped.
            AUDIT_LOCKS="Cargo.lock"
        fi
        while IFS= read -r audit_lock; do
            [ -n "$audit_lock" ] || continue
            audit_leg "$audit_lock"
        done <<< "$AUDIT_LOCKS"
    fi
else
    echo "$GATE: no manifest changes — audit skipped (deps unchanged)"
fi

if [ "$FAIL" -ne 0 ]; then
    echo "$GATE: BLOCKED — fix the items above, or push with RUST_QUALITY_SKIP=1 (logged, CI backstop applies)" >&2
    exit 1
fi

echo "$GATE: pass (fmt/clippy clean on touched files; no new high/critical advisories)"
exit 0
