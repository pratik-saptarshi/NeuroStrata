#!/bin/bash
# Regression tests for scripts/rust-quality-gate.sh.
#
# The load-bearing case: python3 builds the added-line attribution set
# and every finding filter. A missing/unusable python3 must FAIL
# CLOSED (non-zero exit, "quality NOT verified") — never reach the
# pass verdict with an silently emptied attribution set.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/rust-quality-gate.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/rust-quality-gate-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# A PATH that contains every tool the gate needs EXCEPT python3:
# symlink the real tool dirs, skipping any python* entry.
STUB_BIN="$TMP/bin-nopython"
mkdir -p "$STUB_BIN"
for dir in "$HOME/.cargo/bin" /usr/bin /bin; do
    [ -d "$dir" ] || continue
    for f in "$dir"/*; do
        name="$(basename "$f")"
        case "$name" in
            python|python3|python3.*) continue ;;
        esac
        [ -e "$STUB_BIN/$name" ] || ln -s "$f" "$STUB_BIN/$name" 2>/dev/null || true
    done
done
if PATH="$STUB_BIN" command -v python3 >/dev/null 2>&1; then
    echo 'FAIL: test setup could not hide python3 from PATH' >&2
    exit 1
fi

new_repo() {
    local repo="$1"
    mkdir -p "$repo/src"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    cat > "$repo/Cargo.toml" <<'EOF'
[package]
name = "gate-fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
EOF
    printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$repo/src/lib.rs"
    (cd "$repo" && cargo generate-lockfile >/dev/null 2>&1)
    git -C "$repo" add Cargo.toml Cargo.lock src/lib.rs
    git -C "$repo" commit -q -m baseline
    git -C "$repo" update-ref refs/remotes/origin/develop HEAD
}

new_feature_repo() {
    # Fixture with a feature-gated module, mirroring the real tree:
    # root Cargo.toml's `nats-mesh = []` gates src/synaptic_mesh/nats.rs.
    # $2 = clean: gated module starts clippy-clean.
    # $2 = prebad: gated module starts with a pre-existing clippy warning
    #              (needless_return) already present at the baseline.
    local repo="$1" gated_state="$2"
    new_repo "$repo"
    cat >> "$repo/Cargo.toml" <<'EOF'

[features]
default = []
gated = []
EOF
    printf '#[cfg(feature = "gated")]\npub mod gated;\n\npub fn clean() -> i32 {\n    1\n}\n' > "$repo/src/lib.rs"
    if [ "$gated_state" = "prebad" ]; then
        printf 'pub fn gated_bad() -> String {\n    return "x".to_string();\n}\n' > "$repo/src/gated.rs"
    else
        printf 'pub fn gated_clean() -> i32 {\n    42\n}\n' > "$repo/src/gated.rs"
    fi
    (cd "$repo" && cargo generate-lockfile >/dev/null 2>&1)
    git -C "$repo" add Cargo.toml Cargo.lock src/lib.rs src/gated.rs
    git -C "$repo" commit -q -m 'feature-gated baseline'
    git -C "$repo" update-ref refs/remotes/origin/develop HEAD
}

add_gated_dirty_line() {
    # Formatted cleanly (fmt must not fire) but a clippy needless_return
    # on the added lines, inside feature-gated code: only a clippy run
    # that enables the feature can see it.
    printf '\npub fn gated_bad() -> String {\n    return "x".to_string();\n}\n' >> "$1/src/gated.rs"
    git -C "$1" add src/gated.rs
    git -C "$1" commit -q -m 'dirty gated addition'
}

add_dirty_line() {
    # Misformatted (rustfmt rewrites it) AND a clippy needless_return on
    # the added lines: with attribution working, the gate must block.
    printf '\npub fn bad( ) -> String {\n    return "x".to_string( );\n}\n' >> "$1/src/lib.rs"
    git -C "$1" add src/lib.rs
    git -C "$1" commit -q -m 'dirty addition'
}

add_clean_line() {
    printf '\npub fn also_clean() -> i32 {\n    2\n}\n' >> "$1/src/lib.rs"
    git -C "$1" add src/lib.rs
    git -C "$1" commit -q -m 'clean addition'
}

run_gate() {
    # $1 = repo, $2 = PATH to run under
    local repo="$1" path="$2" out status
    set +e
    out=$(cd "$repo" && PATH="$path" bash "$GATE" 2>&1)
    status=$?
    set -e
    printf '%s' "$out"
    return "$status"
}

# Leg 1 (control): python3 present + dirty added lines -> gate blocks.
dirty="$TMP/dirty"
new_repo "$dirty"
add_dirty_line "$dirty"
set +e
out1=$(run_gate "$dirty" "$PATH")
st1=$?
set -e
if [ "$st1" -eq 0 ] || ! grep -q 'FAIL' <<< "$out1"; then
    printf 'FAIL: dirty added lines with python3 present must block the gate\n%s\n' "$out1" >&2
    exit 1
fi

# Leg 2 (regression): python3 absent + the same dirty lines -> the gate
# must FAIL CLOSED, never report a pass on an empty attribution set.
dirty_np="$TMP/dirty-nopython"
new_repo "$dirty_np"
add_dirty_line "$dirty_np"
set +e
out2=$(run_gate "$dirty_np" "$STUB_BIN")
st2=$?
set -e
if [ "$st2" -eq 0 ] || ! grep -qi 'python3' <<< "$out2" || ! grep -q 'NOT verified' <<< "$out2"; then
    printf 'FAIL: missing python3 must fail closed with a clear prerequisite message\n%s\n' "$out2" >&2
    exit 1
fi

# Leg 3 (control): python3 present + clean added lines -> gate passes.
clean="$TMP/clean"
new_repo "$clean"
add_clean_line "$clean"
if ! run_gate "$clean" "$PATH" > /dev/null; then
    echo 'FAIL: clean added lines with python3 present must pass the gate' >&2
    exit 1
fi

# Leg 4 (regression): a clippy warning on an ADDED line inside
# feature-gated code must block. Without feature enablement the gated
# module is never compiled, so the warning passes unchecked.
gated_dirty="$TMP/gated-dirty"
new_feature_repo "$gated_dirty" clean
add_gated_dirty_line "$gated_dirty"
set +e
out4=$(run_gate "$gated_dirty" "$PATH")
st4=$?
set -e
if [ "$st4" -eq 0 ] || ! grep -q 'new clippy diagnostic on added line' <<< "$out4"; then
    printf 'FAIL: clippy warning on an added line in feature-gated code must block the gate\n%s\n' "$out4" >&2
    exit 1
fi

# Leg 5 (no false block): a PRE-EXISTING clippy warning inside
# feature-gated code (present at the baseline, on untouched lines)
# plus an unrelated clean added line must pass — enabling features
# surfaces legacy gated warnings, but added-line attribution must
# keep them from blocking.
gated_prebad="$TMP/gated-prebad"
new_feature_repo "$gated_prebad" prebad
add_clean_line "$gated_prebad"
set +e
out5=$(run_gate "$gated_prebad" "$PATH")
st5=$?
set -e
if [ "$st5" -ne 0 ]; then
    printf 'FAIL: pre-existing feature-gated warning on untouched lines must not block the gate\n%s\n' "$out5" >&2
    exit 1
fi

echo 'rust-quality-gate tests passed'
