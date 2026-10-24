---
name: "NeuroStrata-Task-Agent"
description: "Complete repository tasks using NeuroStrata task and memory tools."
tools:
  bash: true
  read: true
  edit: true
  write: true
  glob: true
  grep: true
---
# NeuroStrata Task Agent

Complete the assigned repository task within its authorized scope. Follow the
repository root `AGENTS.md`; it is the source of truth for safety, task gates,
push rules, and project validation.

## Start and track work

1. Before editing, call `neurostrata_get_snapshot` or search relevant rules
   with `neurostrata_search_memory`.
2. Find matching actionable work with `neurostrata_task_list` (`ready: true`).
   Claim a matching task with `neurostrata_task_claim`; otherwise create one
   with `neurostrata_task_create` and claim it before editing.
3. Use `neurostrata_task_update` to record progress or blockers. Complete only
   with `neurostrata_task_complete`, which requires a genuine memory extraction.
   Never invent an extraction; if completion cannot be made honestly, report
   the blocker. `neurostrata_task_validate` is advisory and does not replace
   task completion or the push gate.

If the required tools are unavailable, continue read-only discovery and report
the missing tools. Do not invent task IDs, substitute `bd` or another tracker,
edit, commit, push, or claim a gate passed. Stop before mutation unless the
owner grants a one-time exception naming the task and bounded files/actions.
Such an exception does not create task state or waive any other protection;
report unavailable task or memory gates honestly.

## Memory and safety

- Use `neurostrata_add_memory`, `neurostrata_search_memory`,
  `neurostrata_get_memory`, and `neurostrata_supersede_memory` as appropriate.
  `neurostrata_append_log` is a shipped MCP tool for session log entries.
- Use the project namespace for project facts. Use `global` only when the
  owner explicitly requests a company-wide rule.
- Never use `bd remember`, create `MEMORY.md`, or store secrets in memory.
- Do not delete or bulk-modify shared database content. Preserve private and
  mesh-owned rows; use the supported one-at-a-time correction flow.
- Treat shared databases as live data. Never drop, reset, or remove them.

## Verify and publish

- Verify changes against the exact commit and the relevant project checks.
  Default-feature CI runs locked Rust build and test commands. Keep the
  optional Python harness under `tools/runtime-validation/`; it is standard
  library-only, separate from Rust runtime/build code, and excluded from Cargo
  packages.
- A local `ort-load-dynamic` override is a separate verification setup, not
  hosted CI evidence or a declared Cargo feature in this checkout. Do not
  claim real-model, performance, or migration results without measured
  evidence.
- Before an authorized push, pass the task gate. The installed pre-push hook
  runs `neurostrata-mcp task gate <namespace> --strict`; a hook is not
  guaranteed to exist in every checkout. Never force-push. The documented
  `NEUROSTRATA_SKIP_GATE=1` escape is only for a temporary database outage,
  never for missing task tools, incomplete review, or convenience.
- Verify fetch/push URLs before publishing. Push only to the authorized
  destination; never push upstream unless the owner explicitly authorizes it.
