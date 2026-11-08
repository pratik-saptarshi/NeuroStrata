# Sync Execution Plan — Forward-Merge of the Elected Target into Fork `develop`

Anthropic-Memory Parity program. This plan executes the D2 ruling
(sync path) under the M0 hard gate. It is a **plan**: it authorizes
nothing. Every outward action in it is gated on Pratik's separate
per-action word, as itemized in §2.

- **Authorization of record:** Pratik, 2026-11-08 — "D2: Authorize
  the sync path (staged, per-action gates stay mine)". Stage 1 of
  that staging is this validated plan; execution (Stage 2) begins
  only on his approval of it.
- **Vehicle:** a true forward-merge of the elected target commit
  into a sync branch off current `develop` — not a rebase, not a
  cherry-pick series (sync-resolution §2). The fork's 542
  develop-only commits are preserved.
- **Normative sources:** plan v2 Task 0.2 + M0 hard gate;
  `docs/plans/2026-11-07-memory-parity/sync-resolution.md` (§2 path,
  §4 preservation semantics, §7 fallbacks — see the §0 erratum on
  §7's pricing); `baseline-census.md` (preservation inventory
  source); `test-venue.md` (test venue standard, §2 hard/report-only
  legs, §2.1 abort classification, §8 merge-ready census).
- **Venue rules (whole plan):** fork only; ghg for all GitHub
  calls (`ghg check` pre-flight before every write; exit 75 or 403
  = stop and report the stage); **no contact with the live
  NeuroStrata daemon or its store** — the repo's in-tree AGENTS.md
  task-claiming/task-gate instructions conflict with program rules
  and are not complied with (standing recorded conflict); the sync
  does **not** upgrade, restart, or rebuild the live daemon (it
  stays on its separate v1.9.0 track); WIP limit ≤3 program PRs
  awaiting merge.

## GATE 0 — Target election (decided at plan approval, before any S1 work)

The draft of this plan treated the target as foreclosed ("the pin
is the pin"). The wringer panel disproved that framing with
measured numbers (§1, both rows). **Approving this plan therefore
includes an explicit target election** — the plan is parameterized
by TARGET and runs identically for either choice:

| Option | Target | Conflict surface (measured) | What it buys | What it costs extra |
|---|---|---|---|---|
| **PIN** | `8fdae4b745269b4097ecbf02a6375f0a0c3b792c` (v1.16.1; PRs #56/#57/#59/#60) | 164 conflicted / 95 src; result tree `511d352b` | The exact base M0 censused and priced; zero new census surface | Lands one release behind upstream on arrival; the +5 delta (below) is owed as a follow-up sync immediately |
| **RE-PIN** | `51be592335b4b447252af198b375836bc7246f8d` (v1.17.0; pin + PRs #61/#62/#63 + release + golden-rules) | 166 conflicted / 96 src; result tree `8608a3b4` | Current upstream tip; no immediate follow-up sync owed | The +5 commits (50 files, 13,540 ins, mostly additive new files; +628 ladybug.rs / +706 server.rs lines; does NOT touch judgment/import.rs/bus.rs) are uncensused — they are censused by the S8 re-run, and the two extra conflicted files (`src/graph_read/memories_for_file.rs` + its test, both new in +5) join G10 |

Branch name follows the election: `sync/main-8fdae4b-into-develop`
(PIN) or `sync/main-51be592-into-develop` (RE-PIN). Merged version
fact (§3 G8 / P13) is the target's version: 1.16.1 (PIN) or 1.17.0
(RE-PIN). If upstream advances past `51be592` before Gate 0 is
decided, the RE-PIN row is re-measured at S0 and the election is
re-presented if the surface moved by more than ±5 files.

## 0. Errata and honest scope (read before approving)

Three corrections the panel forced into the record. They change
what approval means, so they sit at the front.

**E1 — The sync-resolution §7 fallback pricing contains a phantom
cost, and D2 was elected against it.** §7b names its "single
largest line" of deferred cost: that upstream *deleted*
`src/events/outbox.rs`, so deferring the sync would force a
substrate redesign of Tasks 2.1–2.2. **Verified false in Stage 1:**
`outbox.rs` is absent at the merge-base `7d836c7` **and absent at
the target** — upstream never had it to delete; it entered develop
via fork PR #19 (`3f476dc`) and the merge keeps it conflict-free.
The fallback (M0 gate condition (b)) is therefore cheaper than §7
priced it. This does not by itself invalidate D2 — the sync also
buys divergence hygiene and Phase 2 readiness (E2) — but the
election was made against a price sheet with a phantom largest
line on one side, so **Gate 0's re-affirmation at approval is the
informed election**, and this erratum is recorded in the execution
log as the correction of record for §7.

**E2 — What the full sync actually buys, stated plainly.** Phase 1
(Tasks 2.1–2.12) strictly consumes only the judgment subtree —
`provider.rs`, shadow machinery in `mod.rs`, `canonical.rs` — plus
optionally a slice of `server.rs` (sync-resolution §7a's own
table). A hand-port of that subtree is the cheap alternative the
full sync over-buys against. The full sync's real purchase is:
(a) Phase 2 readiness (fts/lexical/rdf/community/rebac/did
surfaces, P17), (b) divergence hygiene — 773 (PIN) / 778 (RE-PIN) commits of drift
resolved once, under one audit, instead of compounding, and
(c) one configuration that has passed one full gate loop, rather
than a third judgment configuration existing on neither base.
Whether that is worth 3–6 days and 8 owner asks across 6 decision points (§8) is
the owner's call at Gate 0 — the plan no longer presents it as a
Phase 1 necessity.

**E3 — Election is not foreclosure.** D2 elected gate condition
(a) over (b); it did not rescind (b). If execution aborts under
§6, the outcome returns to the owner and (b) — correctly priced
per E1 — is live again. Likewise Phase 0 (D3) is independent, and
its premise check can refute or shrink Phase 1: **the owner may
elect D3 and await Phase 0's premise results before Gate A
without invalidating this plan** (§9). Sequencing the expensive
step before the premise check is a choice, and it is the owner's.

## 1. Verified ground truth (2026-11-08, Stage 1; panel-reproduced)

| Fact | Value | How verified |
|---|---|---|
| Fork `develop` tip | `1f27318e7d1735d1603d48e696755231ed31faa6` (PR #34 merge) | ghg ref read, GHG_ACTOR=parity-sync-plan |
| Fork `main` | `51be592335b4b447252af198b375836bc7246f8d` — **past the pin** | ghg ref read |
| Upstream `main` | `51be592…` (fork main == upstream main; mirror intact, advanced) | ghg ref read |
| Merge-base (develop ↔ either target) | `7d836c72e81552523b86abbb0b80d71709551c4e` (unchanged) | git merge-base |
| Divergence | PIN: 773 target-only / 542 develop-only commits. RE-PIN: 778 target-only | git rev-list |
| Differing files (develop ↔ PIN target) | 434 total / 154 under `src/` (was 430 at pricing; +4 = M0 `docs/plans/` files, add-only on develop) | git diff --name-only |
| **Conflict surface vs PIN** | **164 conflicted files, 95 under `src/`** — identical counts to the priced surface against old develop `5b8f9a7` | `git merge-tree --write-tree --name-only origin/develop 8fdae4b…` (git 2.43.0, exit 1), result tree `511d352b` |
| **Conflict surface vs RE-PIN** | **166 conflicted files, 96 under `src/`** — the entire mechanical delta of re-pinning is +2 files | same command vs `51be592…`, result tree `8608a3b4` (Devil's Advocate measurement, judge-verified in the log) |
| Conflict types (PIN) | 146 add/add + 18 content | merge-tree messages |
| Top churn (two-tip, PIN) | ladybug.rs 18,076 · server.rs 6,674 · import.rs 3,890 · main.rs 3,156 · embed.rs 2,726 · ingest.rs 2,115 · task/mod.rs 1,998 · worker.rs 1,563 · bus.rs 1,432 (total two-tip churn +2.1% vs the priced figure) | git diff --numstat |
| `binaries/` in the PIN auto-merge result tree | 0 entries — measured on the **auto-merge** tree; P15 re-verifies on the resolved tree, and this number is not quoted at Gate A as resolved-tree evidence | git ls-tree on `511d352b` |

**Deltas vs the priced surface, explained.** The M0 merges were
docs/scripts-only and added **zero** new conflicted files — the
164/95 counts are unchanged. Two composition deltas matter:
(a) server.rs churn rose 6,243 → 6,674 and task/mod.rs 1,811 →
1,998 — the develop side grew (PR #30's token fix; the #31
scaffold P1 fix), so the same conflicts carry more develop-side
content to preserve; (b) the differing-file total rose by exactly
the 4 M0 plan docs.

**Fork `main` moved — merge the TARGET by SHA, never the branch.**
At sync-resolution time the mirror sat exactly at the pin and the
path read "merge `origin/main`". The mirror has since been synced
forward to `51be592`. Therefore the merge step merges the elected
TARGET commit by SHA — `git merge <TARGET>` — never `origin/main`.
Under PIN, merging the branch tip would absorb five uncensused
commits and silently re-pin; under RE-PIN the tip *is* the target
today, and merging by SHA keeps that true even if the mirror moves
again mid-execution.

**`outbox.rs` correction (see also E1).** Absent at the merge-base
and at both targets; develop-only via fork PR #19; kept with no
conflict. The semantic risk sits in `bus.rs` (conflicted): the
merged bus must keep develop's outbox seq-qualification wired
(§3 G5; §4 P7).

**PR #29 — the competing sync (panel finding, now plan input).**
PR #29 (`merge(main): merge upstream main 8fdae4b into develop`,
branch `merge/main-into-develop-8fdae4b7`, head `ee556749`,
CONFLICTING/DIRTY vs current develop) is a prior lane's attempt at
this exact merge: merge commit `a05c68b` (pin as second parent) +
~46 fix commits. Tree-level panel checks establish: its head
contains **neither** `2675a33` (P5 scaffold fix) **nor** `1766850`
(P10 token fix) — it is based on pre-M0 develop `5b8f9a7`; its fix
series is dominated by repairs of its own splice method, with ≥4
commits parking tests on deferred contract forks; its tree differs
from the auto-merge result in 169 files. **It is not a vehicle.**
It is, however, a *rulings record*: its fix commits encode per-file
triage rulings from a lane that already fought this merge, and S2
mines them (§2). Its disposition is an owner decision at S0 —
this plan does not presume it.

## 2. Staged steps and owner gates

Gates are named **0 / A / B / C**. Everything not behind a gate is
local or read-only work, except where a step explicitly says
otherwise (S6's PR-comment writes, S8's sub-gates). `develop`
itself is never touched by any step except the merge at Gate C.
**Re-verification rule (panel P1):** the §1 ref reads — develop
tip, TARGET ancestry, PR #29 status, upstream tip — are re-run
**immediately before Gate A, before Gate B, and immediately
before the Gate C request**, and the results recorded in the
execution log. §6.4's abort is checked at those points, not
assumed.

- **S0 — Pre-flight + PR #29 disposition (read-only; one owner
  decision).** Re-run the §1 reads for the elected TARGET.
  **Hard stops:** a PR (other than #29) whose head already merges
  the TARGET; TARGET no longer an ancestor of fork main; conflict
  count moved by more than ±5 files vs §1. **PR #29 disposition —
  owner's word, required before S1:** close it as superseded by
  this plan (a `gh` write: `ghg check` + `ghg run -- pr close 29`
  with a comment naming this plan), or leave it open with the
  collision recorded (it then counts against the WIP limit and
  §6.4 watches it: if #29 merges first, the sync aborts). The
  plan's recommendation, from the panel evidence in §1: close as
  superseded and mine it in S2.
- **S1 — Local sync worktree + merge (no gate; local only).**
  Parent clone: `~/workspace/repos/NeuroStrata-parity-sync`
  (named, because S4's hook install lands in the parent's shared
  git-common-dir). `git worktree add
  ~/workspace/repos/NeuroStrata-sync -b <elected branch name>`
  off the verified develop tip. Commit this plan's .md source
  into the branch at
  `docs/plans/2026-11-07-memory-parity/sync-execution-plan.md`
  (planning documents ship in the PR they govern). Then
  `git merge --no-ff <TARGET>` (by SHA, per §1). Record the raw
  conflict list; it must match §1 within the S0 tolerance.
- **S2 — Conflict resolution (no gate; local only).** Resolve per
  §3's group strategies, one group per commit (or per file for
  G1/G2), conventional-commit subjects naming the group and the
  direction taken. **Two standing inputs:** (i) **PR #29 mining**
  — before resolving any file #29's fix series touched, read its
  ruling for that file and record *adopt* or *overturn-with-reason*
  in the execution log; silent contradiction of a prior ruling is
  a review finding. (ii) **Loss protection** — per-group commits
  plus a **daily `git bundle` of the sync branch into the goal's
  `hidden_files/`** (local, no gate): multi-day unpushed work on
  this VM is never single-copy. A fresh reviewer reviews each of
  G1, G2, G4, G5, **G7 (incl. gate.rs)**, G8 resolutions against
  §4's inventory before the branch is called resolved (SDD
  discipline; reviewers verify in the tree, not from the
  resolver's summary).
- **S3 — Local verification battery (no gate; local only).** Run
  every §4 check executable locally (greps, counts, guard
  direct-invocation battery per sync-resolution §6 — shell, no
  cargo). All local checks pass or are dispositioned in the log
  before Gate A is requested. (No sanctioned local Rust toolchain;
  compilation and tests are CI's job — S5.)
- **S4 — Guard re-verification (no gate; local only).** Per
  sync-resolution §2 step 5c: the merge touches
  `scripts/hooks/pre-push` and `scripts/install-hooks.sh` (both
  conflicted, resolved develop-wins per G9). Run
  `bash scripts/install-hooks.sh` in the sync worktree; with the
  git-common-dir fix this installs into the **parent clone's**
  (`NeuroStrata-parity-sync`) shared hooks — that target is the
  expected one; if the installer reports any other common dir,
  STOP and record. Re-run the §6 fabricated-URL battery against
  the **installed** hook (7 upstream forms rejected incl. bogus
  host/lowercase/:443; fork + unrelated pass). This precedes the
  first push: the first push is the first moment the merged
  tree's hook governs a real push.
- **GATE A — Push (owner's word).** The ask, in full: *push the
  sync branch to the fork — the initial push **and** resolution-fix
  pushes to this branch only, capped at the §5 budget (8–15
  test-(ubuntu) iterations); any push beyond budget, or to any
  other branch, returns to the owner.* Command:
  `git push -u origin <elected branch name>` from the sync
  worktree. Expected push output: Guard 1 passes the fork URL;
  Guard 2 prints `warning: neurostrata-mcp not found — gate NOT
  run for this push` (fail-soft). **If that warning is absent,
  STOP and report before proceeding** (runbook §3.3 — the task
  gate must not execute against the live store under program
  rules). Push is via git credential helper (ghg gates `gh`
  calls, not git transport — disclosed per lane practice); no
  `gh` write happens at this gate.
- **GATE B — Open the PR (owner's word).** `ghg check` then
  `ghg run -- pr create --base develop --head <elected branch
  name>`. The ask covers PR creation only — it grants no push
  scope (that was Gate A) and no comment scope beyond the PR body.
  PR body carries: this plan's summary incl. the Gate 0 election,
  the §1 surface table, the §4 preservation checklist with S3
  results, the merge-by-SHA rationale, the PR #29 disposition and
  mining note, and (under PIN) the explicit statement that the +5
  upstream delta is NOT in this PR. Title:
  `merge(sync): forward-merge upstream <short TARGET> into develop`.
  PR mechanics, stated for reviewers: ~773+ commits and a
  ~1,702-file merge-base diff exceed GitHub's 250-commit display
  cap — **review is tree-based** (per-group diffs against the
  merge-base and the §4 checks), not PR-UI-based.
- **S5 — CI iteration (within Gate A's stated scope).** Iterate
  on the `test (ubuntu-latest)` signal per the runbook: classify
  every failure (own-test FAILED line / §2.1 abort signature /
  other-verbatim). Fix commits are resolution fixes only — no new
  scope. **Push-cadence floor:** no new push until the prior
  head's `test (ubuntu)` is terminal, or the prior run is
  explicitly abandoned in the log with reason — an executor must
  not starve itself of terminal verdicts by cancelling runs
  faster than the slow-day leg time. Every attempt is recorded.
- **S6 — Review (scoped `gh` writes, pre-flighted).** Fresh
  whole-branch reviewer against §3/§4 (the S2 reviewers re-verify
  their groups' final state on the PR head); Codex review awaited
  and censused — a Codex no-show is recorded as *pending*, never a
  pass. **Write scope, stated plainly:** per-thread "Fixed in
  <sha>" dispositions, reasoned rebuttals, and thread resolutions
  on this PR only, each pre-flighted with `ghg check`; any other
  `gh` write returns to the owner. Threads are resolved only on
  in-tree-verified fixes.
- **GATE C — Merge (owner's word).** Only when the §7 acceptance
  criteria all hold on the exact head, censused at report time
  per runbook §8 — including the **executed-preservation-tests
  clause (§7.2)** and a **refreshed upstream-delta report**
  (current upstream tip + delta-to-TARGET at report time, so the
  owner rules knowing what the next sync already owes). Command:
  `ghg check` then `ghg run -- pr merge --merge
  --match-head-commit <exact head sha>`. The merge is a true
  merge (no squash — the sync branch's resolution history is the
  audit trail); never bypass branch protection.
- **S8 — Post-merge close-out (contains two gated sub-actions —
  not "no gate").** (a) Verify develop tip == the PR merge commit
  (ghg ref read; read-only). (b) **Re-run the Task 0.1 census on
  the new base** (M0 gate condition (a), second half; under RE-PIN
  the +5's Control Plane / NeuroMesh surface gets its first census
  rows here): recorded as
  `docs/plans/2026-11-07-memory-parity/baseline-census-postsync.md`
  via a small docs PR — **its own Gate-B and Gate-C words at the
  time** (read-only until then). (c) Re-derive the remaining delta
  to upstream tip as the next re-census seed (read-only). (d)
  **Guard activation in the main clone** (`~/workspace/repos/NeuroStrata`:
  post-merge `install-hooks.sh` run + battery re-verification) —
  a mutation outside the sync worktree, **owner's word at the
  time**; note it is distinct from S4's install into the parity
  clone's common dir. A daemon binary rebuild is NOT part of S8d
  and remains unauthorized. (e) **Store-compatibility open item,
  recorded not resolved:** nothing in this sync proves a
  post-sync binary reads a store written by pre-sync code. The
  sync never runs a post-sync binary against any real store
  (venue rules). S8 records this as a named prerequisite for
  Phase 1's first run — proposed test: a fixture store written by
  pre-sync code, opened by the post-sync binary, in CI — to be
  built in Phase 1 planning, not improvised here. (f)
  **Sibling-lane notice:** record in the execution log an
  inventory of lanes/clones based on pre-sync develop (and the
  lbug-0.22.3 lane, based on `main`, which meets this sync at its
  own next step) so the base change does not orphan them silently;
  notifying other lanes is the parent agent's call. (g) Gate
  report to the owner: **the M0 hard gate is then satisfied**;
  Phase 1 may be proposed for election. It does not start —
  Phase 1 election and the Milestone 1 gate questions are
  separate owner decisions.

## 3. Conflict-resolution strategy by group

**Universal method, applied before any group default:** for each
conflicted file, produce the develop-only change set
(`git diff <merge-base>..develop -- <file>`) and the target-only
change set, and adjudicate hunks against §4's inventory. A group
default ("target-wins") means: start from target text, then
re-apply every develop-only hunk the inventory names — it never
means "accept target wholesale without the audit". Every group
resolution records, in its commit body or the execution log,
which develop-only hunks were dropped and why — and, per §2 S2,
its disposition of any PR #29 ruling for the same file.

- **G1 — `src/store/ladybug.rs` (18,076 churn, add/add).**
  Hand-merge; target structure is the base text (the target's
  store carries the three-surface search world Phase 2 needs).
  Develop-only hunks audited one by one; the known
  census-sensitive loci are `search_inner`'s shape (census 5a —
  it *changes*, deliberately: post-sync there are three surfaces,
  rows 5c–5e flip ABSENT → PRESENT) and `is_symbol_of` span-hash
  handling (census 11e — the by-span-hash lookup stays ABSENT;
  Task 2.4 adds it later; the merge must not accidentally create
  or destroy it). The related keying file,
  `src/parser/generation.rs`, is itself conflicted (small churn,
  34/7): its `symbol_id` format AND its hash input are
  **data-compatible load-bearing** — stored symbol ids are
  written as `path#kind:name@hash` where the hash is the SHA-256
  of the symbol's full untruncated definition text — so keying
  resolves develop-wins (see P19). Any develop-only ladybug hunk
  not traceable to the inventory needs a named reason to survive.
- **G2 — `src/server.rs` (6,674 churn, content).** Hand-merge,
  the most adjudication-dense file. **Develop-wins** for:
  PR #30's daemon bearer-token resolution at forward time
  (post-census fix; its tests in `src/test_support.rs` + server
  tests travel with it); the snapshot's task-head / Zero-Action
  Start prefix (census row 7 item 1 — fork behavior).
  **Target-wins** for: `include_ast_edges` (census row 6 flips
  ABSENT → PRESENT — a sync purpose), shadow-verdict consumption
  on the write path, hook-hygiene call sites (server.rs:2050-2054
  region — whichever side's text survives, the §4 P5 check
  re-proves the scaffold invariant against it). Everything else:
  hunk-by-hunk against the inventory.
- **G3 — `src/synaptic_mesh/import.rs` (3,890, add/add) + signing
  paths (incl. `src/synaptic_mesh/worker.rs`).** **Target-wins**
  for the bundle path: bundle-only records routed through
  `validate_bundle_record` and the epoch gate (upstream PR #53,
  issue #43) and sign-after-lifecycle-preservation + RDF rebuild
  (PR #51, issue #46 — loci in worker.rs, P2) — F2 direction:
  develop has no competing behavior (`validate_bundle_record`
  0× develop / 6× target, re-verified in Stage 1). Develop-only
  hunks elsewhere in these files are audited per the universal
  method; none are inventoried.
- **G4 — Judgment subtree — ALL SEVEN conflicted files:
  `mod.rs`, `provider.rs`, `canonical.rs`, `laya_onnx.rs`,
  `circuit_breaker.rs`, `policy.rs`, `typesafe_jev.rs`.**
  **Target-wins** for provider internals across all seven:
  `HeuristicJudgmentProvider` becomes the active provider for
  Auto/Heuristic, shadow-verdict machinery (`last_shadow`,
  recording in `check_duplicates`) is adopted, `canonical.rs`
  comes with it, and the circuit-breaker / policy / typesafe_jev
  files arrive as the target's coherent set (they are the
  heuristic provider's own machinery — resolving them
  develop-wins or file-by-file would create the third
  configuration §7a of the resolution warns against).
  **Carve-outs (develop semantics that must survive):** Laya
  honest semantics (sync-resolution §4.1) — failure surfacing
  stays honest: `LayaOnnxProvider` surfaces `Unavailable`/errors;
  no silent mock-as-Laya substitution is reintroduced by
  resolution (the target's own `DeduplicationChecker::new`
  attempts a real Laya load and continues with `None`,
  warn-logged, on failure — census row 2j — which is compatible;
  P4's check includes a negative grep to prove it); upstream's
  `fetch_model` adoption from the 2026-11-01 precedent stands.
  Trait surface stays exactly duplicate-only (§4.4 of the
  resolution): if the merged `mod.rs` exposes any method beyond
  `judge_duplicates` / `health_check` / `name`, that is a
  resolution defect — Task 2.7's contradiction work is not
  smuggled in by a merge.
- **G5 — Events (`src/events/bus.rs` add/add; `outbox.rs` not
  conflicted; `event.rs`, `mod.rs`, subscribers — incl. the
  conflicted mesh-outbox pair `src/synaptic_mesh/outbox.rs` and
  `src/events/subscribers/mesh_outbox.rs`).**
  **Develop-wins for the durable-sequencing wiring:** the merged
  tree must keep `EventClass::{Audit, Mesh}`, the single
  cross-class `seq`, and the qualification path (census rows
  3b–3c: `outbox.rs:73-84`, `bus.rs:415-429` qualification,
  append-before-dispatch) functioning with develop's `outbox.rs`
  (which the merge keeps untouched), **including the call sites**
  that invoke `append_qualified` in the emit paths (develop
  bus.rs:531 and :562 — P7 checks call sites, not just the
  function definitions). The mesh-outbox pair resolves so the
  mesh subscriber keeps consuming the qualified stream (P20).
  Target bus changes are adopted where they do not sever that
  wiring; where a target hunk removes a call the outbox path
  depends on, the develop behavior is restored and the hunk is
  recorded as dropped with reason. This group carries the plan's
  highest semantic risk (§6) precisely because the failure mode
  is silent: the tree compiles and most tests pass while
  sequencing quietly stops qualifying pulses.
- **G6 — `src/main.rs` (3,156) + `src/daemon.rs`.** Hand-merge.
  **Develop-wins** for: T19's auto-spawn stderr → daemon.log
  block (#27, +217 lines in main.rs) and PR #30's main.rs/daemon.rs
  hunks (token resolution at forward time). Target-wins
  elsewhere, audited per hunk.
- **G7 — `src/task/*` — ALL EIGHT conflicted files: `mod.rs`
  (1,998), `gate.rs`, `intake.rs`, `intent.rs`, `machine.rs`,
  `precheck.rs`, `receipt.rs`, `wiring.rs`.** Two named
  develop-wins items, then target-wins audited:
  (i) **the scaffold invariant in `mod.rs`**: the merged
  `SCAFFOLDED_PRE_PUSH_HOOK` must still embed Guard 1 ahead of
  the gate/skip checks with its recomputed hygiene hash marker
  (PR #31 fix `2675a33`), so hook hygiene can never strip the
  upstream guard (the defect Codex caught in M0); the two
  discriminating tests from that fix
  (`scaffolded_pre_push_hook_carries_upstream_guard_before_the_gate`,
  `scaffolded_pre_push_hook_rejects_upstream_and_allows_fork`)
  must survive textually and pass in CI.
  (ii) **`gate.rs` — the Guard-2 task-gate implementation
  (~1,630-line develop add vs ~1,735-line target add):
  develop-wins for the gate's semantics** — the shipped hook's
  Guard 2 section stays byte-identical to develop's (P18), so the
  gate implementation it invokes must remain the one that section
  was built against; target gate.rs hunks are adopted only where
  they do not change the gate's CLI contract with the hook, and
  every adopted hunk is recorded. The remaining six files:
  target-wins, inventory-audited per the universal method.
- **G8 — Workflows + manifests (`.github/workflows/ci.yml`
  add/add, `nightly.yml` add/add, `release.yml` content;
  `Cargo.toml`/`Cargo.lock` content; `src-tauri/*` add/add).**
  Hand-merge with explicit rules, because target-wins here
  silently changes the venue:
  - `ci.yml`: **develop's gate structure wins** — the 5 hard /
    7 report-only legs and `.github/quality-baseline.md` regime
    (T15a, PR #24) are the venue the runbook documents; the
    target's ci.yml has **zero** report-only legs (Stage 1
    measured — in fact only build+test jobs at all), so its text
    is adopted only where it changes job content without removing
    a leg or a baseline. T16's version-sync check (core + GUI
    manifests) stays. P12 verifies job *wiring*, not just job
    presence (a spliced file whose jobs are mis-wired is a known
    failure — PR #29's history contains exactly that repair).
  - Version: **target wins the number** — merged `Cargo.toml`
    version is the TARGET's (1.16.1 PIN / 1.17.0 RE-PIN), and
    `src-tauri` manifests resolve to the same fact so T16's
    one-version-fact CI check passes on the merged tree.
  - `release.yml`: **develop-wins** for #26's attestation shape
    (no manual `workflow_dispatch`; provenance names the built
    tag); the `release governance` hard leg must pass on the
    merged file.
  - `Cargo.lock`: **target-wins** (it carries PR #56's advisory
    refresh — 9 RUSTSEC advisories cleared); develop-only
    dependency additions are audited via the base..develop diff;
    any develop-only crate the audit surfaces gets a named
    disposition instead of silent loss.
- **G9 — Guard scripts (`scripts/hooks/pre-push`,
  `scripts/install-hooks.sh`, both add/add).** **Develop-wins
  wholesale:** develop's versions are strict supersets (Guard 1
  prepended to the preserved task gate; installer with the
  git-common-dir fix). Resolution = take develop's text, then
  S4's battery proves the installed result.
- **G10 — Everything else (~120–130 files: docs, web-ui, plugins,
  examples, remaining src, conflicted test files; under RE-PIN
  also `src/graph_read/memories_for_file.rs` + its test).**
  Defaults: upstream-authored content (docs/**, web-ui, plugins)
  — target-wins after the develop-only audit (fork-authored docs
  under `docs/plans/` are develop-only and not conflicted);
  remaining src files (embed.rs 2,726, ingest.rs 2,115,
  worker.rs 1,563 — except its G3 signing loci, registry.rs 872,
  …) — hand-merge leaning target, inventory-audited. **Tests
  rule:** where both sides changed the same test file, the
  surviving test must match the behavior the merged code
  implements; no test present on either side is deleted without
  a recorded reason in the log. Upstream-new files
  (rdf/ontology.rs, graphql/mod.rs, fts/mod.rs, time_series.rs,
  kge.rs — add-only) arrive untouched.

## 4. Preservation inventory — post-merge verification checks

Each item names its check. "Local" checks run at S3 (and again on
the PR head's tree at S6 where marked †); "CI" checks are read
from the gate loop at S5/S6. A check that cannot be executed is
reported as such — never asserted. **Executed-evidence rule:**
for every item whose check names CI tests, §7.2 requires those
tests to have actually executed on the exact merge head at least
once — a leg-level classification never substitutes for missing
test evidence.

| # | Behavior (source) | Direction | Verification check |
|---|---|---|---|
| P1 | Bundle-only records validated: `validate_bundle_record` routing + epoch gate in import.rs (upstream #53/#43) | target wins | Local: `git grep -c validate_bundle_record <merge-head> -- src/synaptic_mesh/import.rs` ≥ 6 (target count). CI: mesh import tests pass in test (ubuntu) — note `synaptic_mesh::` tests sort after `parser::` and are equally abort-vulnerable (§5); their execution on the head is required by §7.2, not assumed. |
| P2 | Sign-after-lifecycle-preservation + RDF rebuild (upstream #51/#46, commit `e2beea0`) | target wins | Loci, identified in Stage 1 (no longer deferred): all in `src/synaptic_mesh/worker.rs` — `sign_record_with_keypair` (added by `e2beea0`); the seal-ordering block (target worker.rs ~274-278 and ~349-350: signing happens AFTER lifecycle preservation + RDF rebuild, never before); lifecycle preservation of `created_at`/`created_by` (~332-336); `rebuild_rdf_block` (~619). Local: all four loci present in the merged worker.rs with the ordering intact. CI: the commit's discriminating tests pass at test level — `a_preserved_lifecycle_record_verifies_against_its_stored_signature`, `a_record_with_a_rebuilt_rdf_block_verifies_against_its_stored_signature`, `a_corrupt_signing_key_still_fails_the_drain_after_the_reorder`. |
| P3 | HeuristicJudgmentProvider active for Auto/Heuristic + shadow machinery (`last_shadow` recorded) | target wins | Local: `git grep HeuristicJudgmentProvider` ≥ target's file set (2 files); `last_shadow` present in judgment/mod.rs. CI: judgment tests pass. |
| P4 | Laya honest semantics: Unavailable/errors surface; no silent mock-as-Laya (resolution §4.1) | develop semantics | Local, now mechanical: (i) **negative grep** — no `MockJudgmentProvider` (or any Mock provider) in the Laya construction path of the merged tree (`git grep MockJudgmentProvider <merge-head> -- src/judgment/` returns only test modules, matching the target's own placement); (ii) merged `DeduplicationChecker::new` still attempts a real Laya load and continues with `None`, warn-logged, on failure (census row 2j shape). Any deliberate wiring change is recorded in the log with reason. CI: judgment tests pass. |
| P5 | Hook-hygiene cannot strip Guard 1: scaffold embeds Guard 1 + valid hash marker (PR #31 P1 fix) | develop wins | Local†: scaffold text contains Guard 1 block + marker; **CI: the two `scaffolded_pre_push_hook_*` tests pass at test level in test (ubuntu)** — the discriminating check, and §7.2 requires their execution on the exact head. |
| P6 | Upstream-push guard functional post-merge (scripts + installer) | develop wins | Local (S4): installed-hook battery — 7 upstream URL forms exit 1 (incl. bogus host, lowercase, :443); fork https/ssh + unrelated exit 0; installer idempotent + git-common-dir resolution (parent = `NeuroStrata-parity-sync`). |
| P7 | Durable outbox sequencing survives: EventClass::{Audit, Mesh}, single cross-class seq, qualification + append-before-dispatch (census 3b–3c) | develop wins | Local†: merged bus.rs/outbox.rs contain the qualification path (`qualified_classes` bus.rs:415, `append_qualified` :440) **and the call sites that invoke it in the emit paths (develop :531 and :562, or their merged equivalents)** wired to develop's outbox.rs (blob unchanged or changes dispositioned) — definitions alone do not pass this check. CI: events/bus tests pass; **plus sign-off by TWO named reviewers on the G5 diff** (the failure mode is silent; one sign-off is a single point of failure). |
| P8 | Every-mutation-sequencing state unchanged from census row 3d (conditional seq; procedure_perform emits no pulse — pre-registered ABSENT stays the recorded truth) | neither (record) | Post-merge census (S8) re-derives row 3d on the new base; any change is recorded as a behavior change with a reason, not smoothed over. |
| P9 | T19: auto-spawned daemon stderr → daemon.log (#27) | develop wins | Local: the #27 block (from commit 5b8f9a7's diff) present in merged main.rs. CI: cli/daemon tests pass. |
| P10 | PR #30: daemon bearer token resolved at forward time | develop wins | Local: **all** of #30's hunks from commit 1766850's diff present post-merge — server.rs, daemon.rs, **main.rs and test_support.rs** (the draft's check omitted the latter two). CI: the #30 tests pass at test level. |
| P11 | Snapshot task-head / Zero-Action Start prefix (census row 7.1) | develop wins | Local: `snapshot_prefix` call present in merged server.rs snapshot handler. CI: snapshot tests pass. |
| P12 | Quality-gate venue intact: 5 hard / 7 report-only legs + quality-baseline.md regime (T15a) | develop wins | Local, wiring-level: parse the merged ci.yml — all 12 jobs present **with develop's job keys, `needs` graph, and report-only (`continue-on-error`) flags on the same 7 jobs**; a job present but mis-wired fails this check. quality-baseline.md present. CI: the gate loop *is* the check — leg set on the PR must match the runbook §2 table. |
| P13 | One version fact: core + GUI manifests agree; version = TARGET's post-sync (T16 check; target number) | target number, develop check | Local: version strings equal across Cargo.toml + src-tauri manifests and equal to the elected TARGET's version (1.16.1 / 1.17.0). CI: T16's sync check + `release governance` pass. |
| P14 | Release attestation: no manual dispatch; provenance names the built tag (#26) | develop wins | Local: merged release.yml has no `workflow_dispatch` trigger added by resolution. CI: `release governance` passes. |
| P15 | `binaries/` absent from the **resolved** tree (binaries-only-for-releases) | policy | Local: `git ls-tree -r --name-only <resolved-merge-head> -- binaries` = 0 entries. (The §1 auto-merge measurement does not discharge this.) |
| P16 | Judgment trait surface duplicate-only (resolution §4.4) | neither (guardrail) | Local: merged `src/judgment/mod.rs` trait exposes exactly `judge_duplicates` / `health_check` / `name`. |
| P17 | Phase 2 surfaces present post-sync: src/fts, src/lexical.rs, src/rdf, src/community.rs, src/rebac.rs, src/did.rs, include_ast_edges | target wins | Local: paths exist in merged tree; `git grep include_ast_edges` ≥ target count in server.rs. (Presence only — Phase 2 builds on them later.) |
| P18 | Guard 2 task gate preserved verbatim below Guard 1 in the shipped hook | develop wins | Local: diff of the merged pre-push's Guard-2 section vs develop's — byte-identical (the M0 review standard). |
| P19 | Span-hash keying unchanged — BOTH the format and the hash input: `symbol_id` = `path#kind:name@hash` where the hash segment is SHA-256 of the symbol's **full untruncated definition text** (census 11e keying; stored-id compatibility) | develop wins | Local: merged `src/parser/generation.rs` — `symbol_id` body matches develop's format string AND `sha256_hex`'s call path still hashes the full definition text (a surviving format string with a changed hash input fails this check — it would silently orphan stored symbol ids). CI: parser/ingest tests pass. |
| P20 | Mesh-outbox pair survives: `src/synaptic_mesh/outbox.rs` + `src/events/subscribers/mesh_outbox.rs` (both add/add conflicted) keep the mesh subscriber consuming the qualified event stream | develop wins (wiring) | Local†: merged pair present; the subscriber's subscription/qualification calls match develop's wiring (read-through against develop's versions, differences dispositioned in the log). CI: mesh/events tests pass. |
| P21 | Store compatibility (post-sync binary vs pre-sync store) | **open item — no check exists in this sync** | None executed here, by design (venue rules bar running a post-sync binary against any real store). Recorded at S8e as a Phase 1 prerequisite with a proposed CI fixture test. The plan does not claim this safety; it names its absence. |

## 5. Test strategy (per the test-venue runbook)

- **Venue:** fork CI only. No local cargo (runbook §1). The
  iteration signal is `test (ubuntu-latest)`; the gate is the
  full 12-check loop on the PR's exact head.
- **What gates:** the 5 hard legs — build (ubuntu/macos/windows),
  test (ubuntu), release governance — must conclude **success**
  on the exact head proposed for merge (subject to the §2.1
  classification rule below, reported — not absorbed — at
  Gate C, **and** subject to §7.2's executed-tests clause, which
  a classification cannot waive). The 7 report-only legs must
  match the runbook §2 baseline: same legs failing with the same
  failure *modes*; a new report-only failure mode is a finding
  to disposition, not noise (runbook §3.7). Report-only baselines
  are expected to *move* where the target legitimately changes
  counts (e.g. fmt hunks, audit advisories — PR #56 already
  cleared 9); movement is recorded with its cause, and
  `.github/quality-baseline.md` is updated in the sync PR only
  where a count demonstrably changed because of the merge, with
  the evidence in the PR body. Baseline *promotion* (report-only
  → hard) is out of scope.
- **§2.1 abort classification:** if test (ubuntu) dies with the
  LadybugDB `parser::ingest` SIGABRT signature
  (`unique_ptr<ValueVector>` assertion, suite stops at/just
  before `a_failed_relink_rolls_back_to_the_previous_generation`),
  classify per runbook §2.1 from the job log: read per-test lines
  for the preservation-relevant tests (P1/P2/P3/P5/P7/P10 test
  names) — tests alphabetically after `parser::` do not execute
  in an aborted run, and that includes both the P5 scaffold tests
  (`task::`) **and** P1's mesh tests (`synaptic_mesh::`); in that
  case the classification is "verdict indeterminate for the
  unexecuted tests", and one rerun is legitimate (M0 precedent:
  PR #31's rerun unmasked exactly these tests, 1051 passed / 0
  failed). Every attempt is recorded; rerunning until green and
  quoting only the green run is prohibited. **A classification
  may excuse the leg; it never substitutes for the missing test
  evidence (§7.2).**
- **Discriminating checks:** §4's table is the sync's
  discrimination suite — each row fails on the unmerged base in
  a stated direction (P1: 0× on develop; P3: absent on develop;
  P12: leg set differs at target; P15/P17 flip by construction)
  or fails if its resolution direction is botched (P5's two
  tests fail on the pre-#31 scaffold by construction; P2's named
  tests fail if the seal ordering is inverted; P7's call-site
  check + dual sign-off exist because no test fully discriminates
  a silently severed outbox). Where a preservation behavior has
  an existing test, the test name is read at test level in the
  job log — a leg-level "success" alone is not the evidence for
  P5/P10.
- **Suite size expectation:** the main-binary suite was 1,050
  tests on the M0 base (1,051 with M0's additions); the merged
  tree's count will be the target's count ± develop-only tests —
  the S5 first full run records the merged count and the delta
  is explained in the PR body (a large unexplained drop is an
  abort trigger, §6).
- **Budget (arithmetic corrected by the panel):** resolution
  pushes are budgeted at **8–15 test-(ubuntu) iterations + 3–5
  full gate-loop evaluations**. Wall-clock (runbook §5/§6):
  iteration leg ~5 min good day / 24–30 min slow day; full gate
  loop ~21 min good day / ~68 min slow day. The honest band is
  therefore **~1.7 h (all good days) to ~13.2 h (worst case:
  15 × 30 min + 5 × 68 min) of CI waiting** — plan on **5–14 h**
  spread across the execution window. CI waiting, not GitHub
  minutes, is the binding cost (runbook §10). Pushes obey the
  S5 cadence floor.

## 6. Abort / rollback criteria

`develop` is untouched until Gate C by construction (all work
lives on the sync branch), so "rollback" before Gate C is: stop,
disposition the branch/PR as below, report. **Abort the sync and
return to the owner if any of these is hit:**

1. A §4 preservation check fails and the failure cannot be
   corrected by re-resolution within its group strategy — in
   particular P7: if the merged bus cannot keep develop's outbox
   sequencing wired without redesigning either side's event
   architecture, that is a substrate redesign decision (it
   changes what Task 2.1 builds on), not a conflict resolution.
   Stop; do not improvise the redesign inside the sync.
2. A hard CI leg fails on the resolved tree with a cause that is
   neither (a) the change's own defect, fixable by re-resolution,
   nor (b) classifiable under §2.1 after two attempts on the
   same head — i.e. an unexplained hard-leg failure. Stop and
   report with the job-log evidence.
3. The merged suite count drops materially vs the explained
   delta (§5) — evidence that resolution silently deleted tests
   or a subsystem. Stop; the deletion must be found and
   dispositioned before any further push.
4. A re-verification point (§2) finds the base moved: develop
   moved under the sync branch in a way that changes the
   conflict surface materially, **PR #29 (if left open) merged
   first**, another program PR merged first, or fork main / the
   TARGET's ancestry changed. Stop; re-derive §1; the owner
   re-rules if the pricing changed.
5. Any step would require touching the live daemon/store,
   pushing anywhere but the fork sync branch, or filing
   anything upstream. These are outside the authorization by
   construction — the step is wrong, not the rule.
6. A ghg denial (exit 75 / 403) lands mid-S5/S6 and does not
   clear on the owner's word — stop that line and report the
   exact stage; the PR's state at that moment is recorded in the
   log (head, last terminal verdicts, open threads) so the loop
   can resume from evidence, not memory.

**Post-abort disposition (named in advance):** if abort happens
before Gate B, the sync branch stays local + bundled (§2 S2) and
nothing exists remotely. If abort happens after Gate B, the PR
is **left open and labelled in its body as aborted-with-reason**
— closing it is a `gh` write and is the owner's word, requested
in the abort report alongside the branch's disposition. Nothing
is deleted on abort; branch deletion always follows the
deletion rule (named target, confirmation, 1-hour delay,
reconfirmation).

**Post-merge rollback (named, not executed by this plan):** if
Gate C lands and S8's census or a later discovery shows a §4
behavior was lost, the remedy is a revert of the merge commit
via a new PR — itself an owner-gated action — or a forward fix
PR. The sync branch is retained after merge until S8 closes;
its deletion follows the deletion rule.

## 7. Acceptance criteria for the sync execution (mechanical)

The sync is done when **all** of these hold, each with evidence
in the execution log or the PR:

1. **The TARGET was merged, mechanically stated:** the sync
   branch's internal merge commit has the elected TARGET as its
   **second parent**; the TARGET is an **ancestor of post-merge
   develop**; and (under PIN) the +5 line's tip `51be592` is
   **not** an ancestor of post-merge develop. (A GitHub PR merge
   commit's own parents are (develop tip, branch tip) — the
   criterion is on the branch's merge commit + ancestry, as
   stated.)
2. **Executed preservation evidence on the exact head:** at
   least one terminal `test (ubuntu)` run **on the exact head
   proposed for merge** shows the §4-named preservation tests
   (P1, P2, P3, P5, P7, P10) **executed with verdicts** — pass,
   or a named failure dispositioned in the log. A §2.1
   classification may excuse the *leg's* conclusion; it never
   substitutes for this clause.
3. Gate loop terminal on the merged PR's exact head: 5 hard
   legs success (or §2.1-classified with the classification
   reported to and accepted by the owner at Gate C — with 7.2
   already satisfied); report-only legs match baseline or their
   movement is recorded with cause.
4. Every §4 check (P1–P21) executed with its result recorded —
   P21's result is the recorded open item, which counts as
   executed-honestly, not as passed; zero unresolved review
   threads on the PR; Codex state on the exact head stated
   (clean / findings-dispositioned / pending — pending is
   reported as pending).
5. Post-merge census re-run exists (S8b) and its provider rows
   show the target's internals (P3's world) — the M0 hard gate's
   condition (a) is discharged by evidence, not by the merge
   alone.
6. The guard is installed and battery-verified in the main
   clone post-merge (S8d, under its own owner word).
7. No upstream filing, push, or PR occurred at any point
   (ghg/git logs in the execution log show fork-only writes,
   each behind its gate).

## 8. Effort (a floor, not a tightening — panel-corrected)

The sync resolution priced the full sync at "several focused
engineer-days of conflict resolution + ~5–15 test (ubuntu) runs
+ 2–4 full matrix runs" against `5b8f9a7`. Re-derived against
`1f27318`: the conflict **count is identical (164/95; 166/96
under RE-PIN)**, total churn grew **+2.1%**, and the growth is
concentrated develop-side content in server.rs (+431) and
task/mod.rs (+187) — both fully inventoried in §4. The panel's
correction stands: **these figures are a floor.** The resolution's
own matrix-run price (2–4) and this plan's gate-loop budget
(3–5) are the same quantity at different optimism; the plan
budgets the higher number and says so.

- **Resolution:** ≥2–4 focused engineer-days of agent sessions,
  dominated by G1 (ladybug.rs) and G2 (server.rs); G3/G4/G9 are
  direction-clear and fast; G5 is small in lines but carries the
  highest per-line care; G7's gate.rs adjudication is new scope
  the draft had left in G10's default and may add time; G8 is
  mechanical-with-rules; G10 is long-tail adjudication,
  parallelizable across fresh resolvers per file group under the
  S2 review discipline. PR #29 mining (§2 S2) may save
  re-derivation on some files and cost reading time on others —
  net effect unpriced, honestly.
- **Verification:** S3/S4 battery ≈ half a day including the G5
  dual sign-off; S8 census re-run ≈ half a day (a Task 0.1
  re-execution on a new base; under RE-PIN it also censuses the
  +5 surface for the first time).
- **CI waiting:** §5's budget — 8–15 iterations + 3–5 gate
  loops ≈ **5–14 h of wall-clock waiting** (worst case 13.2 h),
  spread by owner-gate latency.
- **Elapsed:** realistically **3–6 calendar days** end-to-end.
- **Owner load, counted honestly: 6 decision points (8 individual owner asks, counting S8's three separately)** — Gate 0
  (target election, at approval), the S0 PR #29 disposition,
  Gate A, Gate B, Gate C (which may include a §2.1 classification
  acceptance — a substantive ruling inside the gate), and S8's
  docs-PR Gate-B/C words + the S8d guard-activation word. These
  compete in the same window with any other lanes the owner is
  directing; §8 does not pretend otherwise.
- **Risk shape:** high but bounded and one-time. Landing the
  merge resets divergence **to the elected TARGET** — it does
  not reset divergence to upstream, which keeps moving; the
  Gate C report (§2) states the remaining delta at decision
  time so the owner sees what the *next* sync already owes.

## 9. Sequencing vs D3 / Phase 0 / Phase 1

- **D3 (Phase 0 election) is independent of this plan** and
  remains the owner's separate election. Phase 0 is tooling
  (Python harness, SHA-pinned CI binary) and nothing in this
  plan waits for it. The ordering choice is stated, not hidden
  (E3): Phase 0's premise check can refute or shrink Phase 1,
  and this sync's full price (E2) is justified by Phase 2
  readiness + divergence hygiene rather than Phase 1 necessity —
  so **electing D3 and awaiting its premise results before
  Gate A is a legitimate sequencing the owner may choose at
  approval**; the plan remains valid if execution pauses there.
  One interaction to record if both run: Phase 0 baselines are
  base-specific — runs measured pre-sync describe the pre-sync
  base; if the premise check is to speak about the engine
  Phase 1 will build, the M1 report should either run baselines
  on the post-sync base or say plainly which base its numbers
  describe.
- **Phase 1 is blocked** until S8 completes: the M0 hard gate
  opens on (a) sync landed **and** census re-run on the new base
  — both halves. Phase 1's election additionally waits on the
  Milestone 1 gate (registration + baseline report + premise
  check) per plan v2; this plan discharges only the M0 half.
  S8e's store-compatibility item is a further named prerequisite
  for Phase 1's first run.
- **Upstream drift handling:** if upstream advances past the
  elected TARGET during execution, nothing changes mechanically
  (the merge is by SHA); the Gate C report carries the refreshed
  delta, and the *next* sync decision is its own ruling.

## Appendix A — Stage 1 evidence index

- Live refs: ghg reads, GHG_ACTOR=parity-sync-plan, 2026-11-08
  (fork main 51be592 / develop 1f27318 / upstream main 51be592).
- Dry runs (git 2.43.0, `~/workspace/repos/NeuroStrata-parity-sync`):
  vs PIN `8fdae4b…` → exit 1, result tree `511d352b`, 164 named
  conflicted files (95 src/), 146 add/add + 18 content;
  vs RE-PIN `51be592…` → result tree `8608a3b4`, 166 conflicted
  (96 src/) — Devil's Advocate measurement.
- Spot counts: validate_bundle_record 0×/6× (develop/target);
  HeuristicJudgmentProvider 0 files / 2 files; versions
  1.14.3 (develop) / 1.16.1 (pin) / 1.17.0 (tip); report-only
  hits in ci.yml: develop 12 / target 0; outbox.rs absent at
  merge-base `7d836c7` **and at both targets**, added by develop
  commit `3f476dc` (fork PR #19) — the E1 erratum's evidence.
- P2 loci: commit `e2beea0` touches only
  `src/synaptic_mesh/worker.rs` (+557/−10): `sign_record_with_keypair`,
  seal-ordering comments (target ~274-278, ~349-350), lifecycle
  preservation (~332-336), `rebuild_rdf_block` (~619), named
  discriminating tests as listed in P2.
- P7 call sites (develop bus.rs): definitions :415/:440; call
  sites :531, :562.
- P19 hash input: develop `generation.rs` — `symbol_id` =
  `format!("{}#{}:{}@{}", path, kind, name, span_sha256)`;
  `span_sha256` is the SHA-256 (`sha256_hex`) of the symbol's
  full untruncated definition text (doc comment at the function).
- PR #29: head `ee556749`; ancestry checks —
  `git merge-base --is-ancestor 2675a33 ee556749` = NO;
  `1766850` = NO; `8fdae4b` = YES; tree differs from the PIN
  auto-merge result in 169 files.
- Preservation loci: PR #24 `48bf626`, PR #25 `d2f87a1`,
  PR #26 `c7d8f20`, PR #27 `5b8f9a7`, PR #30 `1766850`,
  PR #31 fix `2675a33` (via merge `5550963`).
- Panel record: Feasibility FEASIBLE-WITH-CHANGES; Risk
  UNACCEPTABLE-as-drafted; Devil's Advocate KILL-THE-PLAN-AS-FRAMED
  — judge synthesis + full findings in
  `hidden_files/sync-execution-log.md` (2026-11-08). This refined
  draft is the panel's required revision. Phase 4 independent
  fresh review (2026-11-08): **APPROVED-WITH-CHANGES** — all
  panel P1s judged CURED, every load-bearing number independently
  reproduced; its four P3 wording changes (E2 drift count, G10
  residual count, §8 owner-load count, §5 budget floor) are
  applied in this text.
