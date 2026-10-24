# Agent Instructions Alignment Plan

**Status:** Documentation-only alignment prepared on `develop`
`6f032d70ca2b8ab9fa2dbf85870b021264730915`.

The checked-in CI commands are `cargo build --release --locked` and
`cargo test --locked -- --test-threads=1`; both use Cargo's default features.

## Scope

Align the repository's root agent policy and its two companion instructions.
Keep this change separate from runtime code, dependencies, task/memory data,
hook installation, and the concurrent checkpoint dirty-flag fix.

### Files

- `AGENTS.md` — clarify task/push scope, add a missing-tool stop condition, and
  state the optional Python harness and evidence boundaries.
- `.agents/agents/NeuroStrata-Task.md` — remove its contradictory Beads CLI
  workflow and align task/memory, validation, and push guidance with the root.
- `.agents/skills/neurostrata/SKILL.md` — remove legacy BeadBoard/`todowrite`
  and stale append-log claims; document current MCP names and safety behavior.
- `docs/architecture/agent-instructions-alignment-plan.md` — preserve scope,
  acceptance cases, validation, risks, and the bounded exception record.

## Rationale and constraints

The root policy forbids `bd` and requires NeuroStrata task and memory tools,
but the task-agent instructions direct agents to `bd`. The skill contains
conflicting BeadBoard/`todowrite` advice and says append-log is unavailable,
while `src/server.rs` registers `neurostrata_append_log`. The root also lacks a
usable read-only path when required tools are not exposed.

Missing tools permit read-only discovery only. Do not invent task IDs, use an
alternate tracker, claim gates passed, or treat missing tools as mutation
permission. A one-time owner exception must name the bounded task and files or
actions; it does not create task/memory state, waive other controls, or become
a standing bypass. `NEUROSTRATA_SKIP_GATE=1` remains only the documented
temporary database-outage escape; it is not for missing tools or review.

This documentation task proceeded under the owner's one-time approval for the
absent task/memory **pre-edit prerequisites only**. No task ID was created and
no task, memory, validation, or pre-push gate is claimed to have run.

Keep the Python harness, tests, and fixtures within
`tools/runtime-validation/`, separate from Rust source/runtime/build and using
only the Python standard library. Cargo excludes that directory from source
packages. Add no Python runtime/build dependency.

State evidence precisely: checked-in CI uses locked Rust build/test commands at
the reviewed commit with default features. A local `ort-load-dynamic` override
is a separate verification setup, not hosted CI evidence and not a declared
Cargo feature in this checkout. Real-model, performance, and migration
outcomes remain unverified unless a measured run supports them.

## Acceptance scenarios

1. With task and memory tools available, agents read project rules, find or
   create and claim a real task, record progress, and complete it through the
   supported completion path.
2. With required tools unavailable, agents can continue read-only inspection,
   identify the missing tools, and stop before mutation unless a bounded,
   explicit owner exception exists. They never create fake IDs or use `bd`.
3. Append-log availability matches the tool registration in `src/server.rs`;
   task names, hook command, and completion behavior match current source.
4. No instruction authorizes force-push, a general gate bypass, destructive
   shared-database operations, or an implied mutation exception.
5. Python remains an optional, standard-library-only harness under
   `tools/runtime-validation/` and outside Cargo source packages.
6. CI claims describe the exact reviewed commit and default features;
   alternative local setup and unverified real-world results remain distinct.

## Validation

- Independently review the plan and final diff for scope creep, stale tool
  names, permissive fallback language, and conflicts with root protections.
- Check MCP and hook names against `src/server.rs` and `src/main.rs`.
- Check CI commands and Cargo packaging against `.github/workflows/ci.yml` and
  `Cargo.toml`.
- Search the changed instructions for legacy `bd`, BeadBoard, `todowrite`,
  stale append-log availability claims, and blanket `NEUROSTRATA_SKIP_GATE`
  usage; inspect each remaining mention for negation/context.
- Run `git diff --check` and inspect the exact documentation-only diff. No Rust
  build or runtime test is needed for prose-only changes; do not claim those
  checks ran.
- Commit with a Conventional Commit, push only to the user's fork, and open a
  draft PR targeting `develop`. Never push to Cognilogical upstream.

## Risks and mitigations

- **Conflicting instructions:** Keep root `AGENTS.md` authoritative and make
  companion files point to it.
- **Missing-tool loophole:** Allow read-only work only by default; require a
  bounded owner exception for mutations and preserve all other gates.
- **Tool drift:** Anchor names and hook behavior to current source rather than
  repeating obsolete workflows.
- **Overstated validation:** Distinguish hosted default-feature CI from local
  overrides and leave unmeasured real-model/performance/migration outcomes
  unverified.
- **Python/Rust commingling:** Preserve the existing separate directory and
  Cargo exclusion; do not change packages or runtime code.
