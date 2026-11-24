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

echo 'rust-quality-gate tests passed'
