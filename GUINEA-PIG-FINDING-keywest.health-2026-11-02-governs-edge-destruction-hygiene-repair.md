# Keywest guinea-pig report: `repair_namespace_hygiene` deletes every `governs` edge whose target is a path

**Observed:** 2026-11-01 (damage), diagnosed 2026-11-02
**Reporter task:** `keywest-health-ns4v` (Keywest); upstream issue not filed by Keywest
**Related report:** `GUINEA-PIG-FINDING-keywest.health-2026-11-02-content-backfill-hygiene-repair.md`
(same function, same trigger path, a different arm)
**Scope:** Observed edge-layer destruction in the `keywest.health` and `NeuroStrata` namespaces
and the repair written to undo it. **No NeuroStrata source-code change was made.**
**Status:** fixed upstream in `127c7bf` (2026-11-02 17:59); present in the released 1.14.3
binary.

## Summary

`LadybugStore::repair_namespace_hygiene` (`src/store/ladybug.rs:2369`) contains a "dangling refs
in edge arrays" repair that filters the edge arrays

```rust
let keys = ["governs","relates_to","contained_by","extracted_from","depends_on"];
```

down to entries that resolve as **node ids**:

```rust
let tid = if t.contains(NAMESPACE_SEPARATOR) {
    t.split(NAMESPACE_SEPARATOR).last().unwrap_or(t).to_string()
} else { t.to_string() };
let exists = known_ids.contains(&tid)
    || (tid.len() >= 7 && known_ids.iter().any(|k| k.starts_with(&tid)));
if exists { kept.push(item.clone()); } else { unresolvable.push(t.to_string()); }
```

`governs` on a memory is, by the project convention, a list of **repo paths**
(`keywest.health::docs/rules.md`), not node ids. A path never resolves as an id, so every path
in `governs` fails the existence test and is **removed from the array** and moved into
`metadata.archived_refs`.

The repair runs inside `shape_warnings` (`src/store/ladybug.rs:2503`), which the export path
calls. **A routine `neurostrata-mcp export-graph` was therefore sufficient to destroy the
`governs` edge layer of every node in the namespace.**

This is a second, independent arm of the same defect family reported in the companion file
(2026-11-02). The content backfill destroyed `m.content`; this one destroys
`metadata.governs`. They share the trigger path and the function, and the content guard landed
in `6978e9c`.

**STATUS: FIXED UPSTREAM in `127c7bf` (2026-11-02 17:59), while this report was being written.**
That commit rewrote the existence test so a path-valued target is resolved with
`resolve_target_on_write` instead of being deleted:

```rust
let ns_prefix = format!("{}{}", namespace, NAMESPACE_SEPARATOR);
let exists = known_ids.contains(t)
    || exact_target_exists(conn, t).unwrap_or(false)
    || if let Some(stripped) = t.strip_prefix(&ns_prefix) {
        matches!(resolve_target_on_write(conn, &namespace, stripped), TargetResolution::Resolved(_))
    } else if !t.contains(NAMESPACE_SEPARATOR) {
        matches!(resolve_target_on_write(conn, &namespace, t), TargetResolution::Resolved(_))
    } else { false }
    || (t.len() >= 7 && known_ids.iter().filter(|k| k.starts_with(t)).count() == 1);
```

**The released 1.14.3 binary that Keywest runs still carries the defect**, so the exposure is
real for every consumer on that version. The report is kept because the blast radius and the
recovery procedure are not in the fix, and because the fix has no regression test yet (one is
proposed below).

## Blast radius

| Namespace | Nodes that lost `governs` | Edges lost |
|---|---|---|
| `keywest.health` | 535 | 1213 |
| `NeuroStrata` | 142 | 312 |
| **Total** | **677** | **1525** |

Of the 535 `keywest.health` nodes, **all 10 governance anchors** lost their entire array. The
anchor rows are the ones that carry the source graph: `docs/rules.md`, `AGENTS.md`,
`docs/agent-strategy.md`, `docs/standards.md` and the call-discipline spec. Every one of them
had `governs: []` and an `archived_refs` list that was byte-for-byte the `governs` list it used
to have.

Before the damage, `docs/neurostrata/graph.json` carried 0 thin anchors. After, all 10 were
thin, and the project's own `scripts/neurostrata-export-gate.sh` failed with
`governance anchor carries no edges`.

## Why the repair is wrong

Three separate problems, each independently sufficient:

1. **`governs` is not an id array.** It is a path array, by the convention the project and the
   downstream export gate both rely on. The repair treats a path as a dangling id reference.

2. **The repair is reachable from a read path.** `shape_warnings` reads; the repair writes. An
   export mutates the store. (Same objection as the content backfill, reported separately.)

3. **It is not idempotent in the safe direction.** The repair moves the paths to
   `archived_refs` — which is where the data survives — but a consumer that reads `governs`
   sees an empty array and concludes the memory governs nothing. The edge is silently gone
   while the data is still on the row, one field away.

A narrower formulation: the repair should only drop an edge target that *claims* to be an id.
A target that resolves to a repo path (or that does not resolve at all) is a declaration, not a
dangling reference, and belongs in `archived_refs` at most — never deleted from `governs`.

## Expected / triage question

The upstream project already carries `GUINEA-PIG-FINDING-keywest.health-2026-10-16-governs-edge-materialization.md`,
which concerns the *materialization* of `governs` edges. This report concerns the
*destruction* of the `governs` **declaration** by the hygiene repair. They are different code
paths and both matter.

Questions for upstream:

1. **Should `governs` be in the hygiene repair's dangling-ref key list at all?** If path-valued
   `governs` is the convention, the repair must not filter it.
2. **If it must stay, how should the repair distinguish a path from an id?** A target whose
   tail matches a repo path is a declaration; only a target that is shaped like an id
   (`ns::uuid`) and does not resolve is a dangling reference.
3. **Should a hygiene repair that runs from a read path be allowed to delete an edge array
   outright?** Moving to `archived_refs` is recoverable; deleting the array is not, from the
   consumer's point of view.

## Keywest's recovery (for reference; no upstream change was made)

The lost values were recoverable in both namespaces: the pre-damage export
(`docs/neurostrata/graph.json` at commit `005b2c38`) carried the original arrays, and every
node's `archived_refs` carried a copy of what the repair removed. 677 nodes / 1525 edges were
restored from those two sources and verified against the pre-damage export.

Recovery tooling (Keywest-side, not upstream):
`scripts/neurostrata/restore_governs_edges.py` (the repair), and
`scripts/neurostrata/with-daemon-down.sh` (the daemon-lock discipline the repair needs).

## Suggested regression test

```rust
// A governs declaration naming a repo path must survive the hygiene repair.
let before = memory_with_governs("keywest.health::docs/rules.md");
store.repair_namespace_hygiene("keywest.health").await.unwrap();
let after = store.get("keywest.health", &before.id).await.unwrap().unwrap();
assert_eq!(
    after.metadata["governs"],
    json!(["keywest.health::docs/rules.md"]),
    "the hygiene repair must not delete a path-valued governs declaration"
);
```

## Severity

High, and silent. The defect was reachable from an ordinary export and it destroyed the
source-graph edge layer — the thing the exported graph exists to preserve. Fixed upstream in
`127c7bf`; still present in the released 1.14.3 binary, so every consumer on that version is
exposed until they upgrade.

## Notes from the recovery that upstream may want

- **`set-metadata` replaces `payload.metadata` wholesale** (`src/server.rs:7010`). A repair
  that wants to change one key must send the whole current metadata object back. Keywest's
  repair tool does this; a caller that sends only `{"governs": [...]}` erases every other
  metadata field on the row.
- **The store canonicalizes `governs` on write.** `upsert` resolves an unqualified path to the
  node that owns it and rewrites the value: `docs/runbooks/gmail.md` became
  `keywest.health::docs/runbooks/gmail.md`, and in the generation-scoped `NeuroStrata`
  namespace `docs/designs/....md` became `NeuroStrata::4::docs/designs/....md` (generation 4
  is the active generation there). These rewrites are **semantically correct** — the path
  still names the same file, now through the node that owns it — but they mean a
  byte-comparison against a pre-upgrade export will report differences that are not damage.
  Keywest verified this by resolving every rewritten value back to the node it names.
- **A stale `db.wal.checkpoint` re-corrupts a restored store.** Separately from this defect:
  `lbug` replays the WAL beside the store on open, so restoring a good store while an old WAL
  is still in place re-applies the bad WAL to the good store. Keywest hit this during the
  recovery and it cost an hour of confusion; a restore procedure that does not move the WAL
  aside is not complete.
