#!/bin/bash
# scripts/rust-quality-gate.sh
# Pre-push guard: strict Rust quality gate on changed code only (delta-based).
#
# Philosophy (matches task-coverage-gate.sh and Task 15b): the gate NEVER
# blocks on legacy backlog — .github/quality-baseline.md owns that. It
# fails only when THIS push makes quality worse:
#   1. `cargo fmt --check` reports diffs in touched .rs files.
#   2. `cargo clippy` reports warnings in touched .rs files
#      (hard compile errors fail regardless of location).
#   3. `cargo audit` reports high/critical advisories NOT already
#      present in the base branch lockfile.
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

# ── Diff range: upstream tracking branch, else last commit ───────────────
if git rev-parse --verify --quiet "@{u}" >/dev/null 2>&1; then
    RANGE="@{u}..HEAD"
else
    RANGE="HEAD~1..HEAD"
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

# ── 1. fmt: fail only when a rustfmt hunk overlaps added lines ───────────
EDITION="$(grep -m1 '^edition[[:space:]]*=' Cargo.toml 2>/dev/null | grep -oE '"[^"]+"' | tr -d '"' || true)"
[ -n "$EDITION" ] || EDITION="2021"
if ! command -v rustfmt >/dev/null 2>&1; then
    echo "$GATE: warning: rustfmt not found — fmt NOT verified (run \`cargo fmt\` before pushing; CI fmt remains backstop)" >&2
else
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        [ -f "$f" ] || continue
        RUSTFMT_OUT="$(rustfmt --edition "$EDITION" --check "$f" 2>&1 || true)"
        [ -n "$RUSTFMT_OUT" ] || continue
        OVERLAP="$(printf '%s\n' "$RUSTFMT_OUT" | FILE="$f" ADDED="$ADDED_LINES" python3 -c '
import os, re, sys
f = os.environ.get("FILE", "")
added = set(x for x in os.environ.get("ADDED", "").split("\n") if x.startswith(f + ":"))
flag = False
for raw in sys.stdin:
    line = raw.rstrip("\n")
    m = re.match(r"^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@", line)
    if m:
        s, c = int(m.group(1)), int(m.group(2) or 1)
        if any(f"{f}:{n}" in added for n in range(s, s + c)):
            flag = True
print("yes" if flag else "no")
' || true)"
        if [ "$OVERLAP" = "yes" ]; then
            echo "$GATE: FAIL: touched file needs formatting on added lines (run \`cargo fmt\`): $f" >&2
            printf '%s\n' "$RUSTFMT_OUT" | head -n 15 >&2
            FAIL=1
        fi
    done <<< "$CHANGED_RS"
fi

# ── 2. clippy: fail only on diagnostics located on added lines ──────────
# A broken build (no parseable diagnostics) fails regardless — quality
# cannot be verified. Deny-level lints elsewhere in the tree are backlog
# (same rule as warnings): only added lines gate the push.
# (ADDED_LINES computed once above and reused here.)
CLIPPY_OUT="$(cargo clippy --locked --message-format=short 2>&1)"
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
if [ -n "$CHANGED_MANIFEST" ]; then
    if ! command -v cargo-audit >/dev/null 2>&1 && ! cargo audit --version >/dev/null 2>&1; then
        echo "$GATE: warning: cargo-audit not installed — audit NOT run (run \`cargo install cargo-audit\`; CI audit remains backstop)" >&2
    else
        AUDIT_JSON="$(cargo audit --json 2>/dev/null || true)"
        if [ -z "$AUDIT_JSON" ]; then
            echo "$GATE: warning: cargo audit produced no output (offline DB?) — audit NOT verified" >&2
        else
            NEW_IDS="$(printf '%s' "$AUDIT_JSON" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)
vulns = data.get("vulnerabilities", {}).get("list", []) if isinstance(data, dict) else []
for v in vulns:
    adv = v.get("advisory", {}) if isinstance(v, dict) else {}
    sev = (adv.get("severity") or "").lower()
    if sev in ("high", "critical"):
        print(adv.get("id", ""))
' 2>/dev/null || true)"
            if [ -n "$NEW_IDS" ]; then
                BASE_IDS=""
                if git rev-parse --verify --quiet "origin/develop" >/dev/null 2>&1; then
                    TMPBASE="$(mktemp -t rustgate-base-lock.XXXXXX)"
                    if git show "origin/develop:Cargo.lock" > "$TMPBASE" 2>/dev/null; then
                        BASE_JSON="$(cargo audit --file "$TMPBASE" --json 2>/dev/null || true)"
                        BASE_IDS="$(printf '%s' "$BASE_JSON" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)
vulns = data.get("vulnerabilities", {}).get("list", []) if isinstance(data, dict) else []
for v in vulns:
    adv = v.get("advisory", {}) if isinstance(v, dict) else {}
    print(adv.get("id", ""))
' 2>/dev/null || true)"
                    fi
                    rm -f "$TMPBASE"
                fi
                while IFS= read -r aid; do
                    [ -n "$aid" ] || continue
                    if ! printf '%s\n' "$BASE_IDS" | grep -qxF "$aid"; then
                        echo "$GATE: FAIL: new high/critical advisory introduced by this push: $aid" >&2
                        FAIL=1
                    fi
                done <<< "$NEW_IDS"
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
