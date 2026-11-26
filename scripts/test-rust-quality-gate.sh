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

# A stub `cargo` for the audit legs (10-14): a real advisory fixture is
# impractical offline (no RustSec DB), so the stub keys `cargo audit`
# off markers embedded as comments in the lockfile under audit — the
# minimal deterministic fixture this harness can support. A lockfile
# carrying `# audit-vuln-marker` reports one high advisory
# (RUSTSEC-9999-0001, exit 1, mirroring real cargo-audit); a lockfile
# carrying `# audit-baseline-broken` fails a BASELINE (--file) audit
# with no output and a non-zero exit; anything else is verified clean
# (no output, exit 0). `cargo clippy` succeeds silently; every other
# subcommand delegates to the real cargo.
export GATE_TEST_REAL_CARGO="$(command -v cargo)"
STUB_AUDIT_BIN="$TMP/bin-audit-stub"
mkdir -p "$STUB_AUDIT_BIN"
cat > "$STUB_AUDIT_BIN/cargo" <<'STUBEOF'
#!/bin/bash
if [ "${1:-}" = "audit" ]; then
    shift
    file="Cargo.lock"
    is_base=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --file) file="$2"; is_base=1; shift 2 ;;
            --file=*) file="${1#--file=}"; is_base=1; shift ;;
            *) shift ;;
        esac
    done
    if [ "$is_base" = "1" ] && grep -q 'audit-baseline-broken' "$file" 2>/dev/null; then
        exit 2
    fi
    if grep -q 'audit-vuln-marker' "$file" 2>/dev/null; then
        printf 'Crate:     vulncrate\nVersion:   0.1.0\nTitle:     stub advisory fixture\nID:        RUSTSEC-9999-0001\nSeverity:  7.5 (high)\n'
        exit 1
    fi
    exit 0
fi
if [ "${1:-}" = "clippy" ]; then
    exit 0
fi
exec "${GATE_TEST_REAL_CARGO:-$HOME/.cargo/bin/cargo}" "$@"
STUBEOF
cat > "$STUB_AUDIT_BIN/cargo-audit" <<'STUBEOF'
#!/bin/bash
echo 'cargo-audit 0.0.0-stub'
exit 0
STUBEOF
chmod +x "$STUB_AUDIT_BIN/cargo" "$STUB_AUDIT_BIN/cargo-audit"
AUDIT_PATH="$STUB_AUDIT_BIN:$PATH"

add_lock_marker() {
    # $1 = repo, $2 = marker name, $3 = commit message
    printf '# %s\n' "$2" >> "$1/Cargo.lock"
    git -C "$1" add Cargo.lock
    git -C "$1" commit -q -m "$3"
}

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

new_nested_repo() {
    # Fixture for a bootstrapped repo whose crate lives under native/
    # with NO root Cargo.toml: the baseline commit is the clean nested
    # crate itself, with origin/develop pinned at it.
    local repo="$1"
    mkdir -p "$repo/native/src"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    cat > "$repo/native/Cargo.toml" <<'EOF'
[package]
name = "nested-fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
EOF
    printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$repo/native/src/lib.rs"
    (cd "$repo/native" && cargo generate-lockfile >/dev/null 2>&1)
    git -C "$repo" add native
    git -C "$repo" commit -q -m 'nested crate baseline'
    git -C "$repo" update-ref refs/remotes/origin/develop HEAD
    if [ -e "$repo/Cargo.toml" ]; then
        echo 'FAIL: nested fixture must have no root Cargo.toml' >&2; exit 1
    fi
}

add_nested_dirty_line() {
    # Formatted cleanly (fmt must not fire) but a clippy needless_return
    # on the added lines, inside the nested crate.
    printf '\npub fn bad() -> String {\n    return "x".to_string();\n}\n' >> "$1/native/src/lib.rs"
    git -C "$1" add native/src/lib.rs
    git -C "$1" commit -q -m 'dirty nested addition'
}

add_nested_clean_line() {
    printf '\npub fn also_clean() -> i32 {\n    2\n}\n' >> "$1/native/src/lib.rs"
    git -C "$1" add native/src/lib.rs
    git -C "$1" commit -q -m 'clean nested addition'
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

# Leg 10 (regression, Codex P1 — advisory baseline): the branch forked
# BEFORE an upstream advisory fix. The merge-base lockfile and the
# branch lockfile are IDENTICAL (both carry the advisory); only
# origin/develop's lockfile has the fix. The push makes an unrelated
# manifest edit. Comparing against origin/develop blocks on the
# INHERITED advisory; comparing against the resolved push baseline
# ($BASE = merge-base) must pass.
audit_inherited="$TMP/audit-inherited"
new_repo "$audit_inherited"
add_lock_marker "$audit_inherited" audit-vuln-marker 'vulnerable merge-base'
sed -i '/audit-vuln-marker/d' "$audit_inherited/Cargo.lock"
git -C "$audit_inherited" add Cargo.lock
git -C "$audit_inherited" commit -q -m 'upstream advisory fix'
git -C "$audit_inherited" update-ref refs/remotes/origin/develop HEAD
git -C "$audit_inherited" reset -q --hard HEAD~1
printf '# unrelated manifest edit\n' >> "$audit_inherited/Cargo.toml"
git -C "$audit_inherited" add Cargo.toml
git -C "$audit_inherited" commit -q -m 'unrelated manifest edit'
set +e
out10=$(run_gate "$audit_inherited" "$AUDIT_PATH")
st10=$?
set -e
if [ "$st10" -ne 0 ] || grep -q 'new high/critical advisory' <<< "$out10"; then
    printf 'FAIL: inherited advisory (branch lockfile == merge-base lockfile) must not block the gate\n%s\n' "$out10" >&2
    exit 1
fi

# Leg 11 (converse): a genuinely NEW advisory introduced by the
# branch's own lockfile change still blocks.
audit_new="$TMP/audit-new"
new_repo "$audit_new"
add_lock_marker "$audit_new" audit-vuln-marker 'add vulnerable dependency'
set +e
out11=$(run_gate "$audit_new" "$AUDIT_PATH")
st11=$?
set -e
if [ "$st11" -eq 0 ] || ! grep -q 'FAIL: new high/critical advisory introduced by this push: RUSTSEC-9999-0001' <<< "$out11"; then
    printf 'FAIL: a new advisory introduced by the branch lockfile change must block the gate\n%s\n' "$out11" >&2
    exit 1
fi

# Leg 12 (converse): a failed BASELINE audit stays fail-closed in the
# established warn posture — "audit NOT verified", never a block on an
# unverified baseline and never a "no new advisories" pass claim.
audit_base_broken="$TMP/audit-base-broken"
new_repo "$audit_base_broken"
add_lock_marker "$audit_base_broken" audit-baseline-broken 'broken baseline lockfile'
git -C "$audit_base_broken" update-ref refs/remotes/origin/develop HEAD
add_lock_marker "$audit_base_broken" audit-vuln-marker 'add vulnerable dependency'
set +e
out12=$(run_gate "$audit_base_broken" "$AUDIT_PATH")
st12=$?
set -e
if [ "$st12" -ne 0 ] || ! grep -q 'audit NOT verified' <<< "$out12" \
    || grep -q 'new high/critical advisory' <<< "$out12"; then
    printf 'FAIL: a failed baseline audit must warn audit NOT verified without blocking or claiming no new advisories\n%s\n' "$out12" >&2
    exit 1
fi

# Leg 13 (empty-tree baseline): a single-commit repo whose root
# lockfile carries a high advisory. $BASE is the git empty tree —
# there is NO baseline lockfile. The gate's added-content philosophy
# evaluates a root commit in full (legs 6-7), so the baseline is the
# empty advisory set and the root lockfile's advisory counts as new:
# the gate must block with the advisory finding.
audit_root="$TMP/audit-root"
new_root_repo "$audit_root" clean
printf '# audit-vuln-marker\n' >> "$audit_root/Cargo.lock"
git -C "$audit_root" add Cargo.lock
git -C "$audit_root" commit -q --amend --no-edit
[ "$(git -C "$audit_root" rev-list --count HEAD)" = "1" ] || { echo 'FAIL: audit root fixture must stay a single commit' >&2; exit 1; }
set +e
out13=$(run_gate "$audit_root" "$AUDIT_PATH")
st13=$?
set -e
if [ "$st13" -eq 0 ] || ! grep -q 'FAIL: new high/critical advisory introduced by this push: RUSTSEC-9999-0001' <<< "$out13"; then
    printf 'FAIL: a high advisory in a root commit lockfile (empty-tree baseline) must block the gate\n%s\n' "$out13" >&2
    exit 1
fi

# Leg 14 (empty-tree control): a single-commit repo with a clean
# lockfile passes the audit section on the merits.
audit_root_clean="$TMP/audit-root-clean"
new_root_repo "$audit_root_clean" clean
set +e
out14=$(run_gate "$audit_root_clean" "$AUDIT_PATH")
st14=$?
set -e
if [ "$st14" -ne 0 ]; then
    printf 'FAIL: a clean root commit must pass the gate (empty-tree baseline)\n%s\n' "$out14" >&2
    exit 1
fi

# Leg 15 (regression, Codex P1 — diff prefixes): with
# diff.noprefix=true, Git emits `+++ src/lib.rs` instead of
# `+++ b/src/lib.rs`, so a parser keyed on the default b/ prefix
# never sets its current file and silently builds an EMPTY added-line
# set — dirty added lines then pass unchecked. The gate must
# normalize prefixes in its diff invocation (--default-prefix), so
# the same dirty addition must block under this config.
noprefix_dirty="$TMP/noprefix-dirty"
new_repo "$noprefix_dirty"
git -C "$noprefix_dirty" config diff.noprefix true
add_dirty_line "$noprefix_dirty"
set +e
out15=$(run_gate "$noprefix_dirty" "$PATH")
st15=$?
set -e
if [ "$st15" -eq 0 ] || ! grep -q 'FAIL' <<< "$out15"; then
    printf 'FAIL: dirty added lines under diff.noprefix=true must block the gate (prefix normalization)\n%s\n' "$out15" >&2
    exit 1
fi

# Leg 16 (regression, Codex P1 — diff color): with color.ui=always,
# Git wraps every diff line in ANSI escapes (`ESC[1m+++ b/src/lib.rs`),
# so a parser keyed on the plain `+++ b/` header never sets its
# current file and silently builds an EMPTY added-line set — dirty
# added lines then pass unchecked. The gate must suppress color in
# its diff invocations (--no-color), so the same dirty addition must
# block under this config. color.diff=always is the same vector via
# the diff-specific knob and is pinned in the same leg.
color_dirty="$TMP/color-dirty"
new_repo "$color_dirty"
git -C "$color_dirty" config color.ui always
add_dirty_line "$color_dirty"
set +e
out16=$(run_gate "$color_dirty" "$PATH")
st16=$?
set -e
if [ "$st16" -eq 0 ] || ! grep -q 'FAIL' <<< "$out16"; then
    printf 'FAIL: dirty added lines under color.ui=always must block the gate (color suppression)\n%s\n' "$out16" >&2
    exit 1
fi
color_diff_dirty="$TMP/color-diff-dirty"
new_repo "$color_diff_dirty"
git -C "$color_diff_dirty" config color.diff always
add_dirty_line "$color_diff_dirty"
set +e
out16b=$(run_gate "$color_diff_dirty" "$PATH")
st16b=$?
set -e
if [ "$st16b" -eq 0 ] || ! grep -q 'FAIL' <<< "$out16b"; then
    printf 'FAIL: dirty added lines under color.diff=always must block the gate (color suppression)\n%s\n' "$out16b" >&2
    exit 1
fi

# Leg 17 (regression, Codex P1 — external diff driver): with
# diff.external=/bin/true, Git hands the diff to the external driver
# and the attribution pipeline receives empty (non-unified) output —
# the added-line set silently comes out empty and dirty added lines
# pass unchecked. The gate must refuse external diff drivers in its
# diff invocations (--no-ext-diff), so the same dirty addition must
# block under this config.
extdiff_dirty="$TMP/extdiff-dirty"
new_repo "$extdiff_dirty"
git -C "$extdiff_dirty" config diff.external /bin/true
add_dirty_line "$extdiff_dirty"
set +e
out17=$(run_gate "$extdiff_dirty" "$PATH")
st17=$?
set -e
if [ "$st17" -eq 0 ] || ! grep -q 'FAIL' <<< "$out17"; then
    printf 'FAIL: dirty added lines under diff.external=/bin/true must block the gate (external diff suppression)\n%s\n' "$out17" >&2
    exit 1
fi

# Leg 18 (regression, census sibling — textconv driver): a textconv
# driver assigned via attributes (here the repo-local info/attributes,
# the user-controlled form) with diff.<driver>.textconv=/bin/true
# empties the attribution diff exactly like an external driver does.
# The gate must disable textconv in its diff invocations
# (--no-textconv), so the same dirty addition must block.
textconv_dirty="$TMP/textconv-dirty"
new_repo "$textconv_dirty"
printf '*.rs diff=evil\n' > "$textconv_dirty/.git/info/attributes"
git -C "$textconv_dirty" config diff.evil.textconv /bin/true
add_dirty_line "$textconv_dirty"
set +e
out18=$(run_gate "$textconv_dirty" "$PATH")
st18=$?
set -e
if [ "$st18" -eq 0 ] || ! grep -q 'FAIL' <<< "$out18"; then
    printf 'FAIL: dirty added lines under a textconv driver (diff.evil.textconv=/bin/true) must block the gate (textconv suppression)\n%s\n' "$out18" >&2
    exit 1
fi

# Leg 19 (regression, Codex P1 — -diff attribute): with `*.rs -diff`
# in attributes (here the repo-local info/attributes), Git emits a
# "Binary files ... differ" notice instead of hunks, silently
# emptying the attribution set exactly like the color/external/
# textconv vectors. The gate must force textual diffs (--text), so
# the same dirty addition must block.
nodiff_dirty="$TMP/nodiff-dirty"
new_repo "$nodiff_dirty"
printf '*.rs -diff\n' > "$nodiff_dirty/.git/info/attributes"
add_dirty_line "$nodiff_dirty"
set +e
out19=$(run_gate "$nodiff_dirty" "$PATH")
st19=$?
set -e
if [ "$st19" -eq 0 ] || ! grep -q 'FAIL' <<< "$out19"; then
    printf 'FAIL: dirty added lines under a -diff attribute must block the gate (text diff forcing)\n%s\n' "$out19" >&2
    exit 1
fi

# Leg 20 (regression, Codex P1 — nested manifest): a bootstrapped
# repo whose crate lives under native/ with NO root Cargo.toml. The
# gate's clippy leg must resolve the owning manifest instead of
# assuming a root workspace: a CLEAN push in the nested crate passes
# on the merits (pre-fix it blocked with "workspace does not build —
# clippy produced no diagnostics" because cargo found no root
# manifest).
nested_clean="$TMP/nested-clean"
new_nested_repo "$nested_clean"
add_nested_clean_line "$nested_clean"
set +e
out20=$(run_gate "$nested_clean" "$PATH")
st20=$?
set -e
if [ "$st20" -ne 0 ] || ! grep -q 'pass' <<< "$out20"; then
    printf 'FAIL: clean push in a nested crate (native/, no root Cargo.toml) must pass the gate\n%s\n' "$out20" >&2
    exit 1
fi

# Leg 21 (converse): a clippy warning on an ADDED line inside the
# nested crate still BLOCKS — proving the nested clippy run actually
# lints the crate and attributes its diagnostics (with the native/
# path prefix restored), rather than silently skipping it.
nested_dirty="$TMP/nested-dirty"
new_nested_repo "$nested_dirty"
add_nested_dirty_line "$nested_dirty"
set +e
out21=$(run_gate "$nested_dirty" "$PATH")
st21=$?
set -e
if [ "$st21" -eq 0 ] || ! grep -q 'new clippy diagnostic on added line: native/src/lib.rs' <<< "$out21"; then
    printf 'FAIL: clippy warning on an added line in the nested crate must block the gate\n%s\n' "$out21" >&2
    exit 1
fi

# Leg 22 (regression, Codex P1 — CARGO_TERM_COLOR=always): with
# CARGO_TERM_COLOR=always, cargo wraps the severity in ANSI escapes
# (`src/lib.rs:6:5: ESC[1mESC[33mwarningESC[0m:`), so the gate's
# diagnostic regex never matches and a warning on an added line is
# silently missed — the gate passes (fail-open). The gate must force
# color off on its own cargo invocations, so the same warning must
# BLOCK under this env. The fixture line is fmt-clean (clippy
# needless_return only), isolating the clippy leg; leg 1 is the
# default-env control for the same warning shape.
color_env_dirty="$TMP/color-env-dirty"
new_repo "$color_env_dirty"
printf '\npub fn bad() -> String {\n    return "x".to_string();\n}\n' >> "$color_env_dirty/src/lib.rs"
git -C "$color_env_dirty" add src/lib.rs
git -C "$color_env_dirty" commit -q -m 'warn addition'
set +e
out22=$(cd "$color_env_dirty" && CARGO_TERM_COLOR=always bash "$GATE" 2>&1)
st22=$?
set -e
if [ "$st22" -eq 0 ] || ! grep -q 'new clippy diagnostic on added line' <<< "$out22"; then
    printf 'FAIL: clippy warning on an added line under CARGO_TERM_COLOR=always must block the gate (color forced off at invocation)\n%s\n' "$out22" >&2
    exit 1
fi

# Leg 23 (regression, Codex P1 — nested manifest detection): a
# manifest/lockfile-ONLY push in a nested crate (native/Cargo.toml +
# native/Cargo.lock changed, NO .rs change) whose lockfile carries a
# high advisory. Pre-fix, change detection named only the root,
# crates/* and src-tauri manifests, so this push exited 0 with "no
# Rust sources or manifests changed" and the advisory was never
# audited. Post-fix the tree-wide glob detection engages the gate
# and the per-lockfile audit blocks on the new advisory.
nested_manifest_audit="$TMP/nested-manifest-audit"
new_nested_repo "$nested_manifest_audit"
printf '# manifest-only edit\n' >> "$nested_manifest_audit/native/Cargo.toml"
printf '# audit-vuln-marker\n' >> "$nested_manifest_audit/native/Cargo.lock"
git -C "$nested_manifest_audit" add native/Cargo.toml native/Cargo.lock
git -C "$nested_manifest_audit" commit -q -m 'nested manifest/lockfile only, advisory lockfile'
set +e
out23=$(run_gate "$nested_manifest_audit" "$AUDIT_PATH")
st23=$?
set -e
if [ "$st23" -eq 0 ] || ! grep -q 'FAIL: new high/critical advisory introduced by this push: RUSTSEC-9999-0001' <<< "$out23"; then
    printf 'FAIL: a nested manifest/lockfile-only push with an advisory lockfile must block the gate\n%s\n' "$out23" >&2
    exit 1
fi

# Leg 24 (regression, same P1 — nested lockfile-only): only
# native/Cargo.lock changes. The lockfile alone is gate-relevant at
# any depth: the audit leg audits it per-path and blocks.
nested_lock_only="$TMP/nested-lock-only"
new_nested_repo "$nested_lock_only"
printf '# audit-vuln-marker\n' >> "$nested_lock_only/native/Cargo.lock"
git -C "$nested_lock_only" add native/Cargo.lock
git -C "$nested_lock_only" commit -q -m 'nested lockfile only, advisory lockfile'
set +e
out24=$(run_gate "$nested_lock_only" "$AUDIT_PATH")
st24=$?
set -e
if [ "$st24" -eq 0 ] || ! grep -q 'FAIL: new high/critical advisory introduced by this push: RUSTSEC-9999-0001' <<< "$out24"; then
    printf 'FAIL: a nested lockfile-only push with an advisory lockfile must block the gate\n%s\n' "$out24" >&2
    exit 1
fi

# Leg 25 (regression, same P1 — nested manifest-only, broken dep):
# a manifest-ONLY push in the nested crate adding an unresolvable
# path dependency. No .rs changed, so no clippy leg keyed on sources
# can see it; the owning-manifest clippy derived from the changed
# manifest itself must fail closed ("crate at native/Cargo.toml does
# not build"). Runs under the real cargo (the audit stub's clippy
# always succeeds, which would hide the build failure).
nested_manifest_broken="$TMP/nested-manifest-broken"
new_nested_repo "$nested_manifest_broken"
printf '\n[dependencies]\nbaddep = { path = "../does-not-exist" }\n' >> "$nested_manifest_broken/native/Cargo.toml"
git -C "$nested_manifest_broken" add native/Cargo.toml
git -C "$nested_manifest_broken" commit -q -m 'nested manifest only, unresolvable dependency'
set +e
out25=$(run_gate "$nested_manifest_broken" "$PATH")
st25=$?
set -e
if [ "$st25" -eq 0 ] || ! grep -q 'crate at native/Cargo.toml does not build' <<< "$out25"; then
    printf 'FAIL: a nested manifest-only push with an unresolvable dependency must block the gate via the owning manifest\n%s\n' "$out25" >&2
    exit 1
fi

# Leg 26 (fast-exit pin): a docs-only push in a Rust repo still
# takes the genuinely-unrelated fast exit — tree-wide manifest
# detection must not widen into checking unrelated pushes.
docs_only="$TMP/docs-only"
new_repo "$docs_only"
printf '# docs\n' > "$docs_only/README.md"
git -C "$docs_only" add README.md
git -C "$docs_only" commit -q -m 'docs only'
set +e
out26=$(run_gate "$docs_only" "$PATH")
st26=$?
set -e
if [ "$st26" -ne 0 ] || ! grep -q 'no Rust sources or manifests changed' <<< "$out26"; then
    printf 'FAIL: a docs-only push must take the no-op fast exit\n%s\n' "$out26" >&2
    exit 1
fi

# Leg 27 (regression, Codex P1 — verified non-Rust baseline): the
# baseline commit is a non-Rust repo (README only — NO Cargo.lock
# anywhere). The push adds the whole crate: Cargo.toml, src/lib.rs
# and a Cargo.lock carrying a high advisory. Pre-fix, the baseline
# lockfile lookup failed, BASE_OK stayed 0, and the advisory only
# warned "audit NOT verified" while the gate passed (fail-open).
# Post-fix, the baseline commit exists and positively has no
# lockfile at this path (git cat-file), so it is a verified-empty
# baseline and the advisory counts as new: the gate must block.
new_nonrust_repo() {
    local repo="$1"
    mkdir -p "$repo"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    printf '# just docs\n' > "$repo/README.md"
    git -C "$repo" add README.md
    git -C "$repo" commit -q -m 'non-Rust baseline'
    git -C "$repo" update-ref refs/remotes/origin/develop HEAD
    if [ -e "$repo/Cargo.lock" ] || [ -e "$repo/Cargo.toml" ]; then
        echo 'FAIL: non-Rust fixture must have no Cargo files at baseline' >&2; exit 1
    fi
}
nonrust_first_crate="$TMP/nonrust-first-crate"
new_nonrust_repo "$nonrust_first_crate"
mkdir -p "$nonrust_first_crate/src"
cat > "$nonrust_first_crate/Cargo.toml" <<'EOF'
[package]
name = "gate-fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
EOF
printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$nonrust_first_crate/src/lib.rs"
(cd "$nonrust_first_crate" && cargo generate-lockfile >/dev/null 2>&1)
printf '# audit-vuln-marker\n' >> "$nonrust_first_crate/Cargo.lock"
git -C "$nonrust_first_crate" add Cargo.toml Cargo.lock src/lib.rs
git -C "$nonrust_first_crate" commit -q -m 'add Rust crate with vulnerable lockfile'
set +e
out27=$(run_gate "$nonrust_first_crate" "$AUDIT_PATH")
st27=$?
set -e
if [ "$st27" -eq 0 ] || ! grep -q 'FAIL: new high/critical advisory introduced by this push: RUSTSEC-9999-0001' <<< "$out27"; then
    printf 'FAIL: an advisory in a lockfile added to a verified non-Rust baseline must block the gate\n%s\n' "$out27" >&2
    exit 1
fi

# Leg 28 (negative, same P1 — existing-but-unparseable baseline):
# the baseline DOES have a lockfile at the audited path (nested
# native/Cargo.lock), but it is unparseable (the stub's baseline
# audit fails on it). Verified-absent must NOT swallow this shape:
# the gate keeps the warn-only "audit NOT verified" posture — no
# block, and no silent "no new advisories" pass claim either. This
# pins the verified-absent vs unverified distinction per-path.
nested_base_broken="$TMP/nested-base-broken"
new_nested_repo "$nested_base_broken"
printf '# audit-baseline-broken\n' >> "$nested_base_broken/native/Cargo.lock"
git -C "$nested_base_broken" add native/Cargo.lock
git -C "$nested_base_broken" commit -q -m 'unparseable baseline lockfile'
git -C "$nested_base_broken" update-ref refs/remotes/origin/develop HEAD
sed -i '/audit-baseline-broken/d' "$nested_base_broken/native/Cargo.lock"
printf '# audit-vuln-marker\n' >> "$nested_base_broken/native/Cargo.lock"
git -C "$nested_base_broken" add native/Cargo.lock
git -C "$nested_base_broken" commit -q -m 'add vulnerable dependency'
set +e
out28=$(run_gate "$nested_base_broken" "$AUDIT_PATH")
st28=$?
set -e
if [ "$st28" -ne 0 ] || ! grep -q 'audit NOT verified' <<< "$out28" \
    || grep -q 'new high/critical advisory' <<< "$out28"; then
    printf 'FAIL: an existing-but-unparseable baseline lockfile must warn audit NOT verified without blocking or claiming no new advisories\n%s\n' "$out28" >&2
    exit 1
fi

# Leg 29 (regression, Codex P2 — source-owning crates with a root
# manifest): a root workspace plus a standalone native/ crate (its own
# [workspace], so the root workspace run never builds it). A
# source-only push adding a needless_return warning in native/ must
# BLOCK via the owning manifest — pre-fix no leg ran that crate.
new_root_plus_native_repo() {
    local repo="$1"
    new_repo "$repo"
    mkdir -p "$repo/native/src"
    cat > "$repo/native/Cargo.toml" <<'EOF'
[package]
name = "native-fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[workspace]
EOF
    printf 'pub fn nclean() -> i32 {\n    1\n}\n' > "$repo/native/src/lib.rs"
    (cd "$repo/native" && cargo generate-lockfile >/dev/null 2>&1)
    git -C "$repo" add native
    git -C "$repo" commit -q -m 'standalone native crate baseline'
    git -C "$repo" update-ref refs/remotes/origin/develop HEAD
}
root_native_dirty="$TMP/root-native-dirty"
new_root_plus_native_repo "$root_native_dirty"
printf '\npub fn bad() -> String {\n    return "x".to_string();\n}\n' >> "$root_native_dirty/native/src/lib.rs"
git -C "$root_native_dirty" add native/src/lib.rs
git -C "$root_native_dirty" commit -q -m 'dirty standalone native addition'
set +e
out29=$(run_gate "$root_native_dirty" "$PATH")
st29=$?
set -e
if [ "$st29" -eq 0 ] || ! grep -q 'new clippy diagnostic on added line: native/src/lib.rs' <<< "$out29"; then
    printf 'FAIL: clippy warning on an added line in a standalone crate beside a root workspace must block the gate\n%s\n' "$out29" >&2
    exit 1
fi

# Leg 30 (converse): a CLEAN source-only push in the same standalone
# crate passes on the merits — the source-owning run must not
# false-block (e.g. on path-prefix misattribution).
root_native_clean="$TMP/root-native-clean"
new_root_plus_native_repo "$root_native_clean"
printf '\npub fn also_clean() -> i32 {\n    2\n}\n' >> "$root_native_clean/native/src/lib.rs"
git -C "$root_native_clean" add native/src/lib.rs
git -C "$root_native_clean" commit -q -m 'clean standalone native addition'
set +e
out30=$(run_gate "$root_native_clean" "$PATH")
st30=$?
set -e
if [ "$st30" -ne 0 ] || ! grep -q 'pass' <<< "$out30"; then
    printf 'FAIL: clean push in a standalone crate beside a root workspace must pass the gate\n%s\n' "$out30" >&2
    exit 1
fi

# Leg 31 (regression, Codex P2 — file-local rustfmt config): a
# src/rustfmt.toml with hard_tabs = true makes TAB-indented added
# lines correctly formatted (cargo fmt -- --check passes on this
# fixture). Pre-fix the gate formatted stdin from the repo root,
# ignoring the local config, and blocked the push demanding spaces.
local_fmt_clean="$TMP/local-fmt-clean"
new_repo "$local_fmt_clean"
printf 'hard_tabs = true\n' > "$local_fmt_clean/src/rustfmt.toml"
printf 'pub fn clean() -> i32 {\n\t1\n}\n' > "$local_fmt_clean/src/lib.rs"
git -C "$local_fmt_clean" add src/rustfmt.toml src/lib.rs
git -C "$local_fmt_clean" commit -q -m 'hard-tabs baseline'
git -C "$local_fmt_clean" update-ref refs/remotes/origin/develop HEAD
printf '\npub fn tabbed() -> i32 {\n\t7\n}\n' >> "$local_fmt_clean/src/lib.rs"
git -C "$local_fmt_clean" add src/lib.rs
git -C "$local_fmt_clean" commit -q -m 'tabbed addition'
set +e
out31=$(run_gate "$local_fmt_clean" "$PATH")
st31=$?
set -e
if [ "$st31" -ne 0 ] || ! grep -q 'pass' <<< "$out31"; then
    printf 'FAIL: tab-indented added lines valid under a file-local rustfmt.toml (hard_tabs) must pass the gate\n%s\n' "$out31" >&2
    exit 1
fi

# Leg 32 (converse): SPACE-indented added lines under the same
# hard_tabs config are misformatted and must still BLOCK — the
# config must actually drive the check, not disable it.
local_fmt_dirty="$TMP/local-fmt-dirty"
new_repo "$local_fmt_dirty"
printf 'hard_tabs = true\n' > "$local_fmt_dirty/src/rustfmt.toml"
printf 'pub fn clean() -> i32 {\n\t1\n}\n' > "$local_fmt_dirty/src/lib.rs"
git -C "$local_fmt_dirty" add src/rustfmt.toml src/lib.rs
git -C "$local_fmt_dirty" commit -q -m 'hard-tabs baseline'
git -C "$local_fmt_dirty" update-ref refs/remotes/origin/develop HEAD
printf '\npub fn spaced() -> i32 {\n    7\n}\n' >> "$local_fmt_dirty/src/lib.rs"
git -C "$local_fmt_dirty" add src/lib.rs
git -C "$local_fmt_dirty" commit -q -m 'space-indented addition'
set +e
out32=$(run_gate "$local_fmt_dirty" "$PATH")
st32=$?
set -e
if [ "$st32" -eq 0 ] || ! grep -q 'needs formatting on added lines' <<< "$out32"; then
    printf 'FAIL: space-indented added lines under a file-local hard_tabs rustfmt.toml must block the gate\n%s\n' "$out32" >&2
    exit 1
fi

# Leg 33 (regression, Codex P2 — diagnostic paths with spaces): a
# module loaded via #[path = "has space.rs"] gets diagnostics like
# `src/has space.rs:6:5: warning: ...`. Pre-fix the ASCII-only path
# extraction discarded them and the gate passed a needless_return on
# an added line; post-fix the complete path is parsed and it blocks.
spaced_dirty="$TMP/spaced-dirty"
new_repo "$spaced_dirty"
printf '#[path = "has space.rs"]\npub mod spaced;\n\npub fn clean() -> i32 {\n    1\n}\n' > "$spaced_dirty/src/lib.rs"
printf 'pub fn sclean() -> i32 {\n    1\n}\n' > "$spaced_dirty/src/has space.rs"
git -C "$spaced_dirty" add src/lib.rs 'src/has space.rs'
git -C "$spaced_dirty" commit -q -m 'spaced-path module baseline'
git -C "$spaced_dirty" update-ref refs/remotes/origin/develop HEAD
printf '\npub fn bad() -> String {\n    return "x".to_string();\n}\n' >> "$spaced_dirty/src/has space.rs"
git -C "$spaced_dirty" add 'src/has space.rs'
git -C "$spaced_dirty" commit -q -m 'dirty spaced-path addition'
set +e
out33=$(run_gate "$spaced_dirty" "$PATH")
st33=$?
set -e
if [ "$st33" -eq 0 ] || ! grep -q 'new clippy diagnostic on added line: src/has space.rs' <<< "$out33"; then
    printf 'FAIL: clippy warning on an added line in a space-named module file must block the gate\n%s\n' "$out33" >&2
    exit 1
fi

# Leg 34 (regression, Codex P2 — quoted Git paths): a clippy
# violation on an added line in a NON-ASCII-named file
# (src/café.rs). Git C-quotes such pathnames by default in
# --name-only output ("src/caf\303\251.rs") and in diff headers,
# so pre-fix the changed-file set, the attribution parser and the
# fmt leg all skipped the file and the gate passed (fail-open),
# while the identical violation in src/cafe.rs blocked (leg 1
# shape). Post-fix the gate decodes the path and blocks.
nonascii_dirty="$TMP/nonascii-dirty"
mkdir -p "$nonascii_dirty/src"
git -C "$nonascii_dirty" init -q
git -C "$nonascii_dirty" config user.email test@example.com
git -C "$nonascii_dirty" config user.name test
cat > "$nonascii_dirty/Cargo.toml" <<'EOF'
[package]
name = "gate-fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/café.rs"
EOF
printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$nonascii_dirty/src/café.rs"
(cd "$nonascii_dirty" && cargo generate-lockfile >/dev/null 2>&1)
git -C "$nonascii_dirty" add Cargo.toml Cargo.lock 'src/café.rs'
git -C "$nonascii_dirty" commit -q -m baseline
git -C "$nonascii_dirty" update-ref refs/remotes/origin/develop HEAD
printf '\npub fn bad() -> String {\n    return "x".to_string();\n}\n' >> "$nonascii_dirty/src/café.rs"
git -C "$nonascii_dirty" add 'src/café.rs'
git -C "$nonascii_dirty" commit -q -m 'dirty non-ascii addition'
set +e
out34=$(run_gate "$nonascii_dirty" "$PATH")
st34=$?
set -e
if [ "$st34" -eq 0 ] || ! grep -q 'new clippy diagnostic on added line: src/café.rs' <<< "$out34"; then
    printf 'FAIL: clippy warning on an added line in a non-ASCII-named file must block the gate (quoted Git paths decoded)\n%s\n' "$out34" >&2
    exit 1
fi

# Leg 35 (regression, Codex P2 — per-file edition): a bootstrapped
# repo whose ONLY crate lives under native/ on edition = "2024"
# (no root Cargo.toml). The added line uses a let-chain and is
# misformatted per rustfmt edition 2024. Pre-fix the fmt leg read
# the edition only from a root Cargo.toml (default 2021), rustfmt
# errored on the let-chain, the leg warned-and-skipped, and the
# gate passed. Post-fix the edition comes from the owning
# manifest (native/Cargo.toml) and the gate blocks on fmt.
nested_ed2024="$TMP/nested-ed2024"
mkdir -p "$nested_ed2024/native/src"
git -C "$nested_ed2024" init -q
git -C "$nested_ed2024" config user.email test@example.com
git -C "$nested_ed2024" config user.name test
cat > "$nested_ed2024/native/Cargo.toml" <<'EOF'
[package]
name = "nested-fixture"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
EOF
printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$nested_ed2024/native/src/lib.rs"
(cd "$nested_ed2024/native" && cargo generate-lockfile >/dev/null 2>&1)
git -C "$nested_ed2024" add native
git -C "$nested_ed2024" commit -q -m 'nested edition-2024 baseline'
git -C "$nested_ed2024" update-ref refs/remotes/origin/develop HEAD
if [ -e "$nested_ed2024/Cargo.toml" ]; then
    echo 'FAIL: edition fixture must have no root Cargo.toml' >&2; exit 1
fi
printf '\npub fn check(a: Option<i32>, b: Option<i32>) -> bool { if let Some(x) = a && let Some(y) = b { x == y } else { false } }\n' >> "$nested_ed2024/native/src/lib.rs"
git -C "$nested_ed2024" add native/src/lib.rs
git -C "$nested_ed2024" commit -q -m 'misformatted let-chain addition'
set +e
out35=$(run_gate "$nested_ed2024" "$PATH")
st35=$?
set -e
if [ "$st35" -eq 0 ] || ! grep -q 'needs formatting on added lines' <<< "$out35"; then
    printf 'FAIL: misformatted let-chain added lines in a nested-only edition-2024 crate must block the gate (per-file edition)\n%s\n' "$out35" >&2
    exit 1
fi

# Leg 36 (regression, Codex P2 — deleted crates): a deletion-only
# push that removes the last (only) nested crate. The changed
# manifests/sources no longer exist on disk, so pre-fix the
# owning-manifest set came out empty and §2's empty-set fallback
# ran `cargo clippy --workspace` in a manifest-less directory,
# blocking a valid push with "could not find Cargo.toml".
# Post-fix the gate recognizes the manifests were deleted by this
# very push and passes (the deleted crate has nothing to lint).
del_only="$TMP/del-only"
new_nested_repo "$del_only"
git -C "$del_only" rm -qr native
git -C "$del_only" commit -q -m 'delete the nested crate'
set +e
out36=$(run_gate "$del_only" "$PATH")
st36=$?
set -e
if [ "$st36" -ne 0 ] || ! grep -q 'pass' <<< "$out36" \
    || grep -q 'could not find' <<< "$out36"; then
    printf 'FAIL: a deletion-only push removing the last nested crate must pass the gate, not block on the missing root manifest\n%s\n' "$out36" >&2
    exit 1
fi

# Leg 37 (converse, same P2 — live uncovered file keeps the
# fail-closed fallback): a push with NO manifest anywhere that
# adds a live .rs file (no crate deleted — nothing was ever
# owned). The empty owning-manifest set is NOT explained by
# deletions, so the existing workspace fallback must still run
# and fail closed exactly as before.
uncovered_live="$TMP/uncovered-live"
mkdir -p "$uncovered_live/src"
git -C "$uncovered_live" init -q
git -C "$uncovered_live" config user.email test@example.com
git -C "$uncovered_live" config user.name test
printf '# docs only baseline\n' > "$uncovered_live/README.md"
git -C "$uncovered_live" add README.md
git -C "$uncovered_live" commit -q -m 'non-Rust baseline'
git -C "$uncovered_live" update-ref refs/remotes/origin/develop HEAD
printf 'pub fn orphan() -> i32 {\n    1\n}\n' > "$uncovered_live/src/lib.rs"
git -C "$uncovered_live" add src/lib.rs
git -C "$uncovered_live" commit -q -m 'orphan rust file, no manifest anywhere'
set +e
out37=$(run_gate "$uncovered_live" "$PATH")
st37=$?
set -e
if [ "$st37" -eq 0 ] || ! grep -q 'does not build' <<< "$out37"; then
    printf 'FAIL: a live .rs file with no owning manifest anywhere (and no deleted crate) must keep the fail-closed workspace fallback\n%s\n' "$out37" >&2
    exit 1
fi

# Leg 38 (regression, Codex P2 — lockfile relocation): the baseline
# commit has a crate at old/ whose Cargo.lock carries a high
# advisory; the push's ONLY change relocates the crate to new/ with
# IDENTICAL lockfile content (Git reports R100 renames). Pre-fix,
# the lockfile's new path is absent at $BASE, so the verified-absent
# branch treated its advisory baseline as empty and the gate
# blocked on the INHERITED advisory as if newly introduced.
# Post-fix, lineage resolution (rename detection / content match)
# finds the baseline at old/Cargo.lock and the push passes.
new_movable_repo() {
    # $1 = repo, $2 = 1 if the baseline lockfile carries the advisory
    local repo="$1" marker="$2"
    mkdir -p "$repo/old/src"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    cat > "$repo/old/Cargo.toml" <<'EOF'
[package]
name = "movable-fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[workspace]
EOF
    printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$repo/old/src/lib.rs"
    (cd "$repo/old" && cargo generate-lockfile >/dev/null 2>&1)
    if [ "$marker" = "1" ]; then
        printf '# audit-vuln-marker\n' >> "$repo/old/Cargo.lock"
    fi
    git -C "$repo" add old
    git -C "$repo" commit -q -m 'crate at old/ baseline'
    git -C "$repo" update-ref refs/remotes/origin/develop HEAD
}
relocated="$TMP/relocated"
new_movable_repo "$relocated" 1
git -C "$relocated" mv old new
git -C "$relocated" commit -q -m 'relocate crate old to new, lockfile unchanged'
set +e
out38=$(run_gate "$relocated" "$AUDIT_PATH")
st38=$?
set -e
if [ "$st38" -ne 0 ] || grep -q 'new high/critical advisory' <<< "$out38"; then
    printf 'FAIL: relocating a crate with an identical advisory lockfile must not block on the inherited advisory (baseline follows the move)\n%s\n' "$out38" >&2
    exit 1
fi

# Leg 38b (control, same P2): a genuinely NEW lockfile at a new path
# introducing an advisory that exists NOWHERE at $BASE must still
# block — lineage resolution must not whitewash new lockfiles.
brand_new_lock="$TMP/brand-new-lock"
new_nonrust_repo "$brand_new_lock"
mkdir -p "$brand_new_lock/new/src"
cat > "$brand_new_lock/new/Cargo.toml" <<'EOF'
[package]
name = "brand-new-fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[workspace]
EOF
printf 'pub fn clean() -> i32 {\n    1\n}\n' > "$brand_new_lock/new/src/lib.rs"
(cd "$brand_new_lock/new" && cargo generate-lockfile >/dev/null 2>&1)
printf '# audit-vuln-marker\n' >> "$brand_new_lock/new/Cargo.lock"
git -C "$brand_new_lock" add new
git -C "$brand_new_lock" commit -q -m 'add brand-new crate with advisory lockfile'
set +e
out38b=$(run_gate "$brand_new_lock" "$AUDIT_PATH")
st38b=$?
set -e
if [ "$st38b" -eq 0 ] || ! grep -q 'FAIL: new high/critical advisory introduced by this push: RUSTSEC-9999-0001' <<< "$out38b"; then
    printf 'FAIL: a genuinely new lockfile introducing a new advisory must still block the gate\n%s\n' "$out38b" >&2
    exit 1
fi

# Leg 38c (control, same P2): a relocation whose lockfile content
# CHANGED during the move to ADD a new advisory must still block on
# that advisory — the lineage baseline (the old lockfile, audited on
# its own merits) must not whitewash advisories introduced by the
# move itself.
relocated_dirty="$TMP/relocated-dirty"
new_movable_repo "$relocated_dirty" 0
git -C "$relocated_dirty" mv old new
printf '# audit-vuln-marker\n' >> "$relocated_dirty/new/Cargo.lock"
git -C "$relocated_dirty" add new/Cargo.lock
git -C "$relocated_dirty" commit -q -m 'relocate crate and add advisory'
set +e
out38c=$(run_gate "$relocated_dirty" "$AUDIT_PATH")
st38c=$?
set -e
if [ "$st38c" -eq 0 ] || ! grep -q 'FAIL: new high/critical advisory introduced by this push: RUSTSEC-9999-0001' <<< "$out38c"; then
    printf 'FAIL: an advisory added to a lockfile during its relocation must still block the gate\n%s\n' "$out38c" >&2
    exit 1
fi

echo 'rust-quality-gate tests passed'
