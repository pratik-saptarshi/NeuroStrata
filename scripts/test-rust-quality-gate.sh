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

new_root_repo() {
    # Fixture for the FIRST push of a freshly bootstrapped project:
    # exactly ONE commit, no upstream, no refs/remotes/origin/develop.
    # $2 = clean: the initial commit is clippy/fmt clean.
    # $2 = dirty: the initial commit itself carries a misformatted,
    #              clippy-flagged function on its (only) lines.
    local repo="$1" state="$2"
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
    if [ "$state" = "dirty" ]; then
        printf 'pub fn bad( ) -> String {\n    return "x".to_string( );\n}\n' > "$repo/src/lib.rs"
    else
        printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$repo/src/lib.rs"
    fi
    (cd "$repo" && cargo generate-lockfile >/dev/null 2>&1)
    git -C "$repo" add Cargo.toml Cargo.lock src/lib.rs
    git -C "$repo" commit -q -m 'initial commit'
    # The shape under test: one commit, nothing else. Guard the fixture
    # itself so a future harness change cannot silently change the shape.
    [ "$(git -C "$repo" rev-list --count HEAD)" = "1" ] || { echo 'FAIL: root fixture must have exactly one commit' >&2; exit 1; }
    if git -C "$repo" rev-parse --verify --quiet '@{u}' >/dev/null 2>&1; then
        echo 'FAIL: root fixture must have no upstream' >&2; exit 1
    fi
    if git -C "$repo" rev-parse --verify --quiet refs/remotes/origin/develop >/dev/null 2>&1; then
        echo 'FAIL: root fixture must have no origin/develop ref' >&2; exit 1
    fi
}

drop_origin_develop() {
    git -C "$1" update-ref -d refs/remotes/origin/develop
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

# Leg 6 (regression, fresh-repo first push): a single-commit repo with
# no upstream and no origin/develop. The last-resort baseline HEAD~1
# does not exist, so the gate must diff the root commit against the
# git empty tree and evaluate its content — a CLEAN initial commit
# passes on the merits (before the fix this exited 1 with "cannot
# compute the changed-file set against HEAD~1").
root_clean="$TMP/root-clean"
new_root_repo "$root_clean" clean
set +e
out6=$(run_gate "$root_clean" "$PATH")
st6=$?
set -e
if [ "$st6" -ne 0 ] || ! grep -q 'pass' <<< "$out6"; then
    printf 'FAIL: clean initial commit in a fresh single-commit repo must pass on the merits\n%s\n' "$out6" >&2
    exit 1
fi

# Leg 7 (regression, fresh-repo first push): the same shape, but the
# initial commit carries a fmt/clippy violation — the gate must block
# WITH that finding, proving the initial commit's content was actually
# evaluated (not merely that the baseline error moved).
root_dirty="$TMP/root-dirty"
new_root_repo "$root_dirty" dirty
set +e
out7=$(run_gate "$root_dirty" "$PATH")
st7=$?
set -e
if [ "$st7" -eq 0 ] || grep -q 'cannot compute the changed-file set' <<< "$out7" \
    || ! grep -q 'FAIL' <<< "$out7"; then
    printf 'FAIL: dirty initial commit in a fresh single-commit repo must block with a content finding\n%s\n' "$out7" >&2
    exit 1
fi

# Leg 8 (regression, existing baseline shape): upstream present, no
# origin/develop — BASE stays @{u}; a dirty commit on top of the
# upstream baseline still blocks.
upstream_repo="$TMP/upstream"
new_repo "$upstream_repo"
drop_origin_develop "$upstream_repo"
git -C "$upstream_repo" branch base HEAD
git -C "$upstream_repo" branch --set-upstream-to=base >/dev/null
add_dirty_line "$upstream_repo"
set +e
out8=$(run_gate "$upstream_repo" "$PATH")
st8=$?
set -e
if [ "$st8" -eq 0 ] || ! grep -q 'FAIL' <<< "$out8" \
    || grep -q 'cannot compute the changed-file set' <<< "$out8"; then
    printf 'FAIL: dirty commit over an upstream baseline must block the gate\n%s\n' "$out8" >&2
    exit 1
fi

# Leg 9 (regression, existing baseline shape): origin/develop present
# is already pinned by legs 1-5 (new_repo sets refs/remotes/origin/
# develop). HEAD~1-valid shape: multi-commit repo, no upstream, no
# origin/develop — the last-resort HEAD~1 baseline keeps its
# last-commit-only behaviour.
# 9a: a dirty LAST commit blocks.
head1_dirty="$TMP/head1-dirty"
new_repo "$head1_dirty"
drop_origin_develop "$head1_dirty"
add_dirty_line "$head1_dirty"
set +e
out9=$(run_gate "$head1_dirty" "$PATH")
st9=$?
set -e
if [ "$st9" -eq 0 ] || ! grep -q 'FAIL' <<< "$out9" \
    || grep -q 'cannot compute the changed-file set' <<< "$out9"; then
    printf 'FAIL: dirty last commit with a valid HEAD~1 baseline must block the gate\n%s\n' "$out9" >&2
    exit 1
fi
# 9b: a dirty FIRST commit followed by a clean last commit passes —
# HEAD~1 diffs only the last commit (an empty-tree baseline would
# block here, so this pins the scope of the root-commit reroute).
head1_scoped="$TMP/head1-scoped"
new_root_repo "$head1_scoped" dirty
add_clean_line "$head1_scoped"
set +e
out9b=$(run_gate "$head1_scoped" "$PATH")
st9b=$?
set -e
if [ "$st9b" -ne 0 ]; then
    printf 'FAIL: HEAD~1 baseline must keep last-commit-only scope (dirty first commit, clean last commit must pass)\n%s\n' "$out9b" >&2
    exit 1
fi

echo 'rust-quality-gate tests passed'
