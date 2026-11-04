# Keywest guinea-pig report: `repair_namespace_hygiene` overwrote the content of every `fact` node

**Observed:** 2026-11-01 (damage), diagnosed 2026-11-02
**Reporter task:** `keywest-health-ns4v` (Keywest); upstream issue not filed by Keywest
**Scope:** Observed data loss in the `keywest.health` and `NeuroStrata` namespaces and the
repair that was written to undo it. **No NeuroStrata source-code change was made** — the
defect was already fixed upstream in `6978e9c`; this report documents the blast radius and
the residual hazard.

## Summary

Upstream commit `837ef08` (2026-11-01 12:14, "Wave 10: repair 15 SHACL shape_warnings in
keywest.health (metadata.task backfill + content backfill)") added a content backfill to
`LadybugStore::repair_namespace_hygiene`. The condition it shipped was:

```rust
if mtype == "fact" || (content_empty && !mtype.is_empty() && mtype != "rule" && mtype != "procedure" && mtype != "task") {
```

The `mtype == "fact"` arm is **not** guarded by `content_empty`. For every `fact` node in the
namespace, the repair therefore overwrote `m.content` with `metadata.title`, else
`metadata.location`, else the literal string `"legacy"` — regardless of what the content
already was.

`repair_namespace_hygiene` is called from `dangling_edges` and `shape_warnings`
(`src/store/ladybug.rs:2274` and `:2503`), both of which the export path calls. **A routine
`neurostrata-mcp export-graph` was therefore sufficient to destroy the content of every fact
node in the store.**

## Expected / triage question

A hygiene repair should repair what is broken. Overwriting the content of a row that already
has content is not a repair — it is data loss, and it should not be reachable from a read
path.

Two questions for upstream:

1. **Should the content backfill be reachable from a read path at all?** `dangling_edges` and
   `shape_warnings` read; the repair writes. A repair invoked as a side effect of a read
   means an export can mutate the store, which contradicts the expectation that exporting is
   non-destructive.
2. **Can the write-while-reading pattern be guarded generally?** The guard in `6978e9c` fixes
   this one condition. A structural guard — "a repair never overwrites a non-empty content
   field" — would have caught the whole class, and would not depend on getting each
   `mtype` arm right.

## Actual

### The defect and the fix

| Commit | When | Change |
|---|---|---|
| `837ef08` | 2026-11-01 12:14 | Introduced the backfill with the unguarded `mtype == "fact"` arm. |
| `46b579e` | 2026-11-01 12:27 | Fixed only the string escaping. The unguarded arm remained. |
| `6978e9c` | 2026-11-02 06:26 | Guarded the arm: `if (content_empty && mtype == "fact") \|\| (content_empty && …)`. |

`6978e9c` is an ancestor of upstream `main` at the time of writing. The live binary is
1.14.3, hash `017a81ad7be9259a`, in sync with source. **The fix stops new damage; it cannot
undo what was already written.**

### Blast radius

762 nodes were damaged across two namespaces:

| Namespace | Damaged |
|---|---|
| `NeuroStrata` | 447 |
| `keywest.health` | 315 |

By type: 741 `fact`, 20 `file`, 1 `rule`.

Of the 762:

- **727 were recoverable** — their original content was present in the committed Keywest
  graph export `docs/neurostrata/graph.json` (commit `005b2c38`, 2026-11-01 10:26, which
  predates the damage), and their ids carry no `::` namespace separator so `edit_memory`
  accepts them.
- **35 are not recoverable.** 21 are `file` markers whose content was *already* empty before
  the damage (the committed export shows `''` for them, so nothing was lost); 14 are
  `NeuroStrata`-namespace facts created after the damage, whose content existed in no export.
- 20 of the damaged ids contain `::` and are refused by `edit_memory` ("written by directory
  ingestion"). All 20 are among the 21 already-empty `file` markers above, so no real content
  was lost to that restriction.

### Evidence timeline

- `docs/neurostrata/graph.json` at commit `005b2c38` (2026-11-01 10:26): 14,767 nodes,
  **0** nodes with content `"legacy"`.
- The 2026-11-01 10:24 backup and the live DB: 21,213 nodes, **762** nodes with content
  `"legacy"`.
- Every prior committed export has 0 `"legacy"` nodes.

The damage window is therefore 2026-11-01 10:26 → 12:14 (the defect commit), narrowed by
the 10:24 backup and the 10:26 export both being clean.

### The repair

The repair was performed from the committed post-redaction export (the recovery source must
be the *redacted* export, so that restoring cannot reintroduce a reviewer name that had been
scrubbed from the artifacts). Each candidate content was checked against the live reviewer
name set before any write; a content containing a name would have aborted the whole run.

The repair tool lives in the Keywest repository at
`scripts/neurostrata/restore_legacy_content.py`. It writes through `neurostrata-mcp edit`
(never `set-metadata`, which replaces `payload.metadata` wholesale and leaves content
alone), re-verifies the reviewer-name check on every pass, and never prints a memory's
content.

### A residual hazard this report should record

`neurostrata-mcp export-graph` **starts a daemon if none is running**, and a running daemon
holds the store lock. A repair that needs to write to the store must therefore contend with
the daemon that the very export path spawns. The operational consequence for consumers:

- `neurostrata-mcp status` is **not** a safe read-only probe for "is the daemon up" — it is
  itself a client, and it starts a daemon to answer.
- `neurostrata-mcp shutdown` is the only safe way to release the lock (it checkpoints; never
  kill the daemon).
- A process that repeatedly probes with `status` will repeatedly resurrect the daemon it was
  checking for, producing a write/lock contention loop that looks like a hung repair.

This is worth an upstream note because the read-path-spawns-a-daemon behavior is what makes
the export path unsafe to reason about, independently of this specific defect.

## Reproduction outline

1. Check out `837ef08`.
2. Create a namespace with a `fact` node whose `content` is a distinctive string and whose
   `metadata.title` is a different string. Set `mtype` to `fact`.
3. Run `neurostrata-mcp export-graph <out>` for that namespace.
4. Read the node back. Its content is now `metadata.title` (or `metadata.location`, or the
   literal `"legacy"`) — the original string is gone.
5. Repeat against `6978e9c`. The content survives.

## Why it matters

A memory store is the only copy of what a project knows. This defect let a **read** operation
destroy it, silently, at the scale of the entire namespace, with no error, no warning, and
no non-`"legacy"` trace — the rows still looked healthy, and the export succeeded. The
recovery path existed only because Keywest keeps a committed graph export; a consumer that
does not keep one would have lost everything, permanently.

The upstream fix in `6978e9c` is correct for the reported condition. The recommendation in
this report is structural rather than specific: **a hygiene repair should never overwrite a
non-empty content field, and a read path should not mutate the store.** Both would have
prevented this incident outright.
