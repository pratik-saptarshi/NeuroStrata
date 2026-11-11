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

if [ "${RUST_QUALITY_SKIP:-0}" = "1" ]; then
    echo "$GATE: SKIPPED (RUST_QUALITY_SKIP=1) — quality NOT verified for this push" >&2
    exit 0
fi

# ── Diff range: exactly the commits this push introduces ─────────────────
# @{u}..HEAD is exact for linear pushes, but after a rebase the remote
# tip predates the rewritten base and the range balloons to include
# upstream changes (e.g. a tree-wide reformat landing on develop),
# which then false-block as "added" lines. Detect the rewrite (remote
# tip no longer an ancestor of HEAD) and fall back to the merge-base
# with the integration target, so the range is always the branch's own
# delta. A branch with no upstream takes the same merge-base path, so
# first pushes check every commit, not just HEAD~1. Last resort
# (no origin/develop available, e.g. offline): the last commit only.
if git rev-parse --verify --quiet "@{u}" >/dev/null 2>&1 \
    && git merge-base --is-ancestor "@{u}" HEAD 2>/dev/null; then
    RANGE="@{u}..HEAD"
else
    RANGE="$(git merge-base HEAD origin/develop 2>/dev/null || true)..HEAD"
    if [ "$RANGE" = "..HEAD" ]; then
        RANGE="HEAD~1..HEAD"
    fi
fi

CHANGED_RS="$(git diff --name-only "$RANGE" -- '*.rs' || true)"
CHANGED_MANIFEST="$(git diff --name-only "$RANGE" -- 'Cargo.toml' 'Cargo.lock' 'crates/*/Cargo.toml' || true)"

if [ -z "$CHANGED_RS" ] && [ -z "$CHANGED_MANIFEST" ]; then
    echo "$GATE: no Rust sources or manifests changed — nothing to check"
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "$GATE: warning: cargo not found in PATH — gate NOT run for this push (CI remains backstop)" >&2
    exit 0
fi

FAIL=0

# Added-line set for every touched .rs file (format "path:lineno",
# new-file numbering). Both the fmt and clippy attributions below match
# findings against this set, so legacy findings in touched files never
# block the push — only lines this push adds. Mirrors
# task-coverage-gate.sh (+-line inspection).
ADDED_LINES="$(git diff -U0 "$RANGE" -- '*.rs' | python3 -c '
import re, sys
cur, new_line, out = None, 0, set()
for raw in sys.stdin:
    line = raw.rstrip("\n")
    if line.startswith("+++ b/"):
        cur = line[6:]
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
' || true)"

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
        OVERLAP="$(printf '%s\n' "$FMT_DIFF" | FILE="$f" ADDED="$ADDED_LINES" python3 -c '
import os, re, sys
f = os.environ.get("FILE", "")
added = set(x for x in os.environ.get("ADDED", "").split("\n") if x.startswith(f + ":"))
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
' || true)"
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
# (ADDED_LINES computed once above and reused here.)
# --workspace: the root package alone would skip workspace members
# (e.g. crates/neurostrata-wasm, which ships without its own lockfile
# and is covered by the root lockfile for audit). --all-targets: the
# default lib+bins scope skips #[cfg(test)] modules, integration tests
# in tests/, and examples/ — new test/example code must be linted too.
# Diagnostics stay attributed per added line below, so the wider scope
# costs build time, not false blocks.
CLIPPY_OUT="$(cargo clippy --locked --workspace --all-targets --message-format=short 2>&1)"
CLIPPY_CODE=$?
CLIPPY_DIAG="$(printf '%s\n' "$CLIPPY_OUT" | grep -E ':[0-9]+:[0-9]+: (warning|error)' || true)"
if [ -z "$CLIPPY_DIAG" ] && [ "$CLIPPY_CODE" -ne 0 ]; then
    echo "$GATE: FAIL: workspace does not build — clippy produced no diagnostics (exit $CLIPPY_CODE)" >&2
    printf '%s\n' "$CLIPPY_OUT" | tail -n 20 >&2
    FAIL=1
elif [ -n "$CLIPPY_DIAG" ]; then
    while IFS= read -r diag; do
        [ -n "$diag" ] || continue
        # short format: path:line:col: warning|error: message
        dpath="$(printf '%s' "$diag" | grep -oE '^[a-zA-Z0-9_./+-]+\.rs' || true)"
        dline="$(printf '%s' "$diag" | grep -oE '^[a-zA-Z0-9_./+-]+\.rs:[0-9]+' | grep -oE '[0-9]+$' || true)"
        if [ -n "$dpath" ] && [ -n "$dline" ] \
            && printf '%s\n' "$ADDED_LINES" | grep -qxF "$dpath:$dline"; then
            echo "$GATE: FAIL: new clippy diagnostic on added line: $diag" >&2
            FAIL=1
        fi
    done <<< "$CLIPPY_DIAG"
fi

# ── 3. audit: fail only on NEW high/critical advisories vs base ──────────
# Severity source: the human-readable `cargo audit` report (`Severity:
# 7.5 (high)`). The JSON advisory objects carry `cvss` but NO `severity`
# key, so filtering JSON on severity silently matches nothing and the
# gate would never fire. Both sides parsed from text; IDs compared.
if [ -n "$CHANGED_MANIFEST" ]; then
    if ! command -v cargo-audit >/dev/null 2>&1 && ! cargo audit --version >/dev/null 2>&1; then
        echo "$GATE: warning: cargo-audit not installed — audit NOT run (run \`cargo install cargo-audit\`; CI audit remains backstop)" >&2
    else
        AUDIT_TEXT="$(cargo audit 2>/dev/null || true)"
        if [ -z "$AUDIT_TEXT" ]; then
            echo "$GATE: warning: cargo audit produced no output (offline DB?) — audit NOT verified" >&2
        else
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
' 2>/dev/null || true
            }
            NEW_IDS="$(parse_advisories "$AUDIT_TEXT" | awk '$2=="high"||$2=="critical"{print $1}')"
            if [ -z "$NEW_IDS" ]; then
                echo "$GATE: no new high/critical advisories in this push"
            elif ! git rev-parse --verify --quiet "origin/develop" >/dev/null 2>&1; then
                echo "$GATE: warning: origin/develop unavailable — cannot diff advisory baseline; audit NOT verified (CI audit remains backstop)" >&2
            else
                BASE_SEV=""
                BASE_OK=0
                TMPBASE="$(mktemp -t rustgate-base-lock.XXXXXX)"
                if git show "origin/develop:Cargo.lock" > "$TMPBASE" 2>/dev/null; then
                    BASE_TEXT="$(cargo audit --file "$TMPBASE" 2>/dev/null || true)"
                    [ -n "$BASE_TEXT" ] && BASE_SEV="$(parse_advisories "$BASE_TEXT")" && BASE_OK=1
                fi
                rm -f "$TMPBASE"
                if [ "$BASE_OK" != "1" ]; then
                    echo "$GATE: warning: base lockfile audit unavailable — cannot diff advisory baseline; audit NOT verified (CI audit remains backstop)" >&2
                else
                    while IFS= read -r aid; do
                        [ -n "$aid" ] || continue
                        if ! printf '%s\n' "$BASE_SEV" | grep -qE "^${aid}[[:space:]]"; then
                            echo "$GATE: FAIL: new high/critical advisory introduced by this push: $aid" >&2
                            FAIL=1
                        fi
                    done <<< "$NEW_IDS"
                fi
            fi
        fi
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
