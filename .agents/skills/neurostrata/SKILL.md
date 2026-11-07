---
name: neurostrata
description: "Manage the 3-Tier Memory Architecture (Global, Domain, Task), Central Executive Task Machine, Automatic Graph-First Exploration, and Subagent Context Propagation. Replaces legacy MEMORY.md and bd remember."
---
# NeuroStrata (v1.16.1 — Tri-Strata Memory, Central Executive & Graph-First Architecture)

## Overview
NeuroStrata is the standard operating protocol for persisting and retrieving knowledge across sessions. It completely replaces `MEMORY.md`, local markdown trackers, and `bd remember` by utilizing a native Rust Model Context Protocol (MCP) server connected directly to an embedded LadybugDB vector database partitioned into three distinct tiers (The Tri-Strata Model).

## The Tri-Strata Model
1. **Global Stratum (`namespace="global"`)**: Company-wide constraints, infrastructure mandates (e.g., LadybugDB only, no REST, Clojure for parsing), and universal tool usage rules. **CRITICAL: NEVER use 'global' unless the user EXPLICITLY instructs you to make a company-wide machine rule. Assume all architecture rules are local to the current project and DEFAULT TO THE PROJECT NAMESPACE.**
2. **Domain Stratum (SynapticGraph, `namespace="<project_name>"`)**: Hidden business rules, API contracts, and spatial code layouts specific to the project's domains.
   * **SynapticGraph Linking**: To prevent context bloat, NeuroStrata stores *pointers* (e.g., "See `docs/architecture/domains/sync.md` for full narrative") instead of dumping entire narratives into an Engram.
3. **Task Stratum (`namespace="<task_id>"`)**: Specific insights, decisions, and context scoped to a single active task.

## Available MCP Tools  
NeuroStrata provides the following native MCP tools that you MUST use to manage the system's memory:
* `neurostrata_add_memory`: Add a new architectural rule, project pattern, or task insight (an Engram). **FORMATTING:** If the Engram is a strict, non-negotiable constraint that must NEVER be ignored, prefix it with "RULE: " (e.g., "RULE: Never use Python"). If it is general context, domain logic, or a pointer to documentation, just save it as normal text without the prefix. If scoped `namespace=global`, file paths in metadata must point to `~/.config/neurostrata/global/`.
* `neurostrata_search_memory`: Search for existing Engrams before writing code or making architectural decisions.
* `neurostrata_get_snapshot`: Fetch the project's active architectural rules in one call. Use it as your first action on a new task.
* `neurostrata_list_namespaces`: List the namespaces the shared database holds.
* `neurostrata_ingest_directory`: Batch ingest an entire directory of markdown files (e.g., `docs/architecture/`) into NeuroStrata. The server will automatically chunk and embed the files.
* `neurostrata_get_memory`: Fetch one Engram by id. Use it to read a rule before correcting it, and to follow a Governs or Related Nodes pointer to the exact record.
* `neurostrata_supersede_memory`: Correct an Engram that is wrong or out of date. Your new text is stored as a new Engram and the old one is retired: it keeps its original wording as history and stops being returned by search. Use it to fix a rule you got wrong; it is the only tool here that changes an existing Engram.

**These seven are the whole surface, and every one of them is additive.** Nothing you can
reach destroys a memory. That is deliberate: operations that lose bytes -- editing a rule in
place, deleting one, moving one between namespaces, restoring a backup -- are CLI and GUI
commands, so a human is at the keyboard when they run. There is no MCP tool for them. To
correct a rule use
`neurostrata_supersede_memory`. To delete one, the user runs
`neurostrata-mcp delete <namespace> <id>` with the daemon stopped, or the GUI posts to the
daemon's `/delete`. To merge two spellings of a project, the user runs
`neurostrata-mcp move <from> <id> <to>`, which is what `doctor` prints.

**Ids and namespaces.** A node id is a repository-relative path (`src/parser/ingest.rs`),
so that is the form to use in `locations` when a memory governs a file; the absolute path
lives in metadata as `absolute_path`. A namespace is the project name, never the checkout
folder, and names differing only by case resolve to the one already stored.
NeuroStrata (`neurostrata-mcp`, `neurostrata-lf3p`) is a single compiled Rust binary housing an embedded vector, property graph, full-text, and W3C RDF/SPARQL 1.1/SHACL engine (LadybugDB + Tantivy + `oxrdf`/`oxttl` + `spargebra`/`spareval` + `shacl` + Laya). It completely replaces `MEMORY.md`, local markdown TODO lists, `TodoWrite`, and deprecated `bd` commands with binary-enforced task and memory gates.

---

## 0. The Prime Directive: Never Defer Anything
- **Highest-priority mandate:** Never leave any task, feature, or implementation in a `"deferred"`, `"todo"`, `"stub"`, `"phase-2"`, or `"later"` state.
- Every open item must be either:
  1. Completed, verified, and committed to `git` in the current session, OR
  2. Filed as a concrete tracked task via `neurostrata_task_create` with an assignee, description, and blockers documented.

---

## 1. Zero-Touch Onboarding & Self-Upgrading Lifecycle
- **Zero-Touch Harness Auto-Install (`src/task/harness_plugins.rs`):** On init (`neurostrata_bootstrap`, `neurostrata_task_setup`, and `neurostrata_get_snapshot`), NeuroStrata automatically detects the active/installed CLI agent harness (`opencode`, `openclaw`, `claude`, `hermes`, `codex`, `github-cli`) from environment and project markers, extracts the compiled zero-dependency plugin into `.NeuroStrata/plugins/<harness>/`, and wires the harness config (`opencode.json`, `.claude/settings.local.json`, `.hermes/`, `.codex/`, `.openclaw/`, Copilot CLI) with zero user involvement.
- **Self-Healing Dual Git Hooks (`pre-push` + `pre-commit`):** `neurostrata_get_snapshot` (`hook_hygiene`) and `neurostrata-mcp hooks install` automatically install and refresh both `.git/hooks/pre-push` (strict task gate) and `.git/hooks/pre-commit` (graph-query ledger), backing up any foreign hooks safely.
- **Upgrade & Lifecycle Delta Engine (`src/task/wiring.rs`):** When a new binary version ships, the next `neurostrata_get_snapshot` automatically backfills schema defaults, upgrades stale hooks/plugins, and reports `LIFECYCLE & HOOK DELTA` if new wires or triggers need registration.

## 🛑 COST MANAGEMENT & ASYNC DELEGATION
**Your Role:** You are the Knowledge Manager, Architect, and Orchestrator. You are running on an expensive frontier model. 
**The Mandate:** Offload heavy work (coding, refactoring, file creation) to the cheaper `NeuroStrata-Task` agent where that agent is defined, or capture it asynchronously in BeadBoard to avoid blocking the chat. Not every project ships that agent -- when it is absent, do the work inline rather than inventing a subagent.
**The Tooling & Workflow:** 
1. **Async Backlogging (Preferred):** The `Task` tool blocks the chat synchronously. If the user wants to keep chatting and brainstorming, DO NOT use the `Task` tool. Instead, use the `bash` tool to create a BeadBoard bead to capture the requirements in the backlog.
2. **Synchronous Execution:** ONLY use the `Task` tool (`subagent_type: "NeuroStrata-Task"`) if the user explicitly asks for the work to be completed right now.
**Exceptions:** You may only make direct file edits yourself for trivial, one-off changes (e.g., fixing a single typo, renaming a variable).
---

## 2. Central Executive — Mandatory Task Workflow & The 3 Soft Locks
- **Pre-Flight First Action (Zero-Trust Policy):** Your VERY FIRST action on any new session or task MUST be `neurostrata_get_snapshot` (`namespace`, `project_root` — `project_root` is inferred automatically if omitted) or `neurostrata_search_memory`.
- **Zero-Action Start:** You are strictly blocked from editing files (`write`, `edit`) or running mutating shell commands until a task exists and is claimed by your session:
  1. Check existing work: `neurostrata_task_list` (`namespace`, `ready: true`).
  2. Create new work if needed: `neurostrata_task_create` (`namespace`, `title`, `description`, `priority`, `task_type`: `"task" | "bug" | "feature" | "epic"` — never `"recurring"`).
  3. Claim exclusively: `neurostrata_task_claim` (`id`, `namespace`, `assignee`, `session_id`). Claims older than 60 minutes are taken over automatically.
- **Lock 3 — The Breath Working-Memory Prompt:** Every 3–5 mutation steps, pause and append a state summary to the task using `neurostrata_task_update` (`id`, `namespace`, `session_id`, `note`). Note that `status: "done"` is refused by `neurostrata_task_update`.
- **Lock 2 — Done-Funnel Mandatory Extraction:** Close a task ONLY via `neurostrata_task_complete` (`id`, `namespace`, `session_id`, `note`, and `memory: { content, memory_type: "rule" | "fact", metadata: { source, governs } }` or `link_memory_id`). With zero extracted memories, `neurostrata_task_complete` fails with `BLOCKED (Lock 2)`.
- **Lock 1 — Pre-Push Git Gate:** `git push` runs `neurostrata-mcp task gate <namespace> --strict`. Any `in_progress` task, `done` task without an extracted memory, open P0 > 24h, or untracked work-item debt blocks the push.
- **Task vs Procedure:**
  - A **task** finishes once (`neurostrata_task_complete`).
  - A **procedure** is a recurring habit with a finite rehearsal budget (`remaining_fires`) created via `neurostrata_add_memory` (`memory_type: "procedure"`, `metadata.trigger`: `"session-start" | "before-edit" | "after-mutation" | "every-n-sessions:N" | "every-n-days:N"`) and fired via `neurostrata_procedure_perform`.
- **Lexical Task-Intent Recognition:** Deferral phrases (`TODO:`, `FIXME:`, `follow-up:`, `later we need to`, `we should also`) in speech, logs, or memories are surfaced as `task_intent_signals` and `TASK NUDGE` — capture them immediately with `neurostrata_task_create`.

1. **Analyze Scope**:
   * *Global*: Is this a universal tool preference, infrastructure mandate, or language constraint that applies to ALL projects? (Route to `namespace="global"`).
   * *Domain*: Is this a project-specific architecture rule, API contract, or data flow? (Route to Domain Stratum).
   * *Task*: Is this only relevant to the current bug/feature? (Route to `namespace="<task_id>"`).
2. **Auto-Detect Domain**: If it's a Domain insight, look at your current working directory (`pwd`) or the files you are editing to infer which domain it belongs to.
3. **Autonomously Prune & Update**: When inscribing a new Engram, first `neurostrata_search_memory` to see if a similar or contradictory rule already exists. If an old rule is outdated, say so explicitly in the new Engram ("supersedes the earlier rule that ...") so the correction wins on retrieval, and tell the user which id is now stale.

## 🌱 EIDETIC RECALL (Project Genesis)
Every active project MUST have a foundational "Bootstrap" Engram (a LadybugDB node with `memory_type="bootstrap"`). This acts as the supreme context anchor for the entire repository.

1. **The Initial Check:** When starting work on an unfamiliar project, use `neurostrata_get_snapshot` or search the namespace to verify a bootstrap Engram exists.
2. **The Autonomous Rummage:** If no bootstrap Engram exists, you MUST build one immediately. Autonomously explore the codebase (read READMEs, dependency files like `package.json` or `Cargo.toml`, and core structural folders).
   * **Crucial Dependency (The AST Graph):** If the bootstrap Engram does not exist, the codebase is completely unknown to the AI architecture. You MUST use the `neurostrata_ingest_directory` tool to parse the Abstract Syntax Tree (AST) into LadybugDB BEFORE generating the bootstrap Engram. This automatically maps the physical codebase into the vector database.
3. **The User Interrogation:** If the repository is entirely empty, completely opaque, or you cannot deduce its goal, you MUST stop and explicitly ask the user: "What is the core purpose and intended architecture of this project?"
4. **The Ingestion:** Once synthesized, use `neurostrata_add_memory` with `memory_type="bootstrap"` to save a dense, high-level summary of the project's purpose, tech stack, and primary domain logic. You MUST pass the `locations` array mapping this memory to the key architectural files (e.g. `README.md`, `src/main.rs`).
5. **The Evolution (Refinement):** The codebase lives and breathes. Every few major tasks or feature epics, proactively review the existing bootstrap Engram. If the project has expanded or pivoted, update the bootstrap Engram to reflect the new reality (by creating a new, more refined bootstrap Engram and deprecating the old one).
---

## 3. Automatic Graph-First Exploration & Subagent Context Propagation
- **Auto-Retrieved Task Context:** `neurostrata_task_create` and `neurostrata_task_claim` automatically execute a hybrid vector + 1-hop graph search on the task's title/description and return `auto_retrieved_context` plus a ready-to-paste `subagent_prompt_injection` markdown block.
- **Graph-First Lookup Reflex:** Before using blind `grep`/`glob` or editing a file, inspect governing rules (`GOVERNS`) and AST call/import edges (`DEPENDS_ON`, `calls`, `imports`) using:
  - `neurostrata_memories_for_file` (`namespace`, `path`, optional `symbol`, optional `include_ast_edges`) — deterministic, zero-embedding file governance lookup; with `include_ast_edges` (default `true`) it also returns `related_memories` and an `ast_summary` (`symbols`, `callers`, `callees`, `imports`, `importers`, `blast_radius_count`) — this is the call-chain and blast-radius lookup.
  - `neurostrata_related` (`namespace`, `id`, `depth`) — 1–2 hop graph neighborhood.
  - `neurostrata_search_memory` (`mode`: `"vector"` (default) | `"lexical"` | `"lexical-inhouse"`, optional `include_related: true`) — three alternative retrieval surfaces (LadybugDB HNSW vector, Tantivy full-text, in-house BM25) plus a bounded graph expansion. They are **not** fused: there is no rank fusion, no Personalized PageRank and no neural reranker in the search path.
  - `neurostrata_sparql_query` (`namespace`, `query`) — W3C SPARQL 1.1 (`SELECT`, `ASK`, `CONSTRUCT`) over the `oxrdf` RDF store.
  - Every lookup increments `task.graph_queries_count` and records into `.NeuroStrata/session_queries.json`.
- **Verifiable Graph-First predicate:** Before editing any file, check whether its path and governing memory IDs (`mem-...`) are already listed under `### NeuroStrata Task & Graph Context` in your prompt. For any file you touch that is NOT listed in that block, call `neurostrata_memories_for_file` first.
- **Subagent Prompt Propagation:** Subagents start with a blank context window. On session init, write the Graph-First rule into your active plan, and whenever you spawn ANY subagent (`subagent`, `Task`, `delegate_task`, `spawn_agent`), paste the `subagent_prompt_injection` block from `neurostrata_task_claim` into the subagent's prompt (the 6 harness plugins also prepend it automatically).

---

## 4. Tri-Strata Memory, Negative Exemplars & Non-Destructive Lifecycle
- **Tri-Strata Isolation:**
  1. `namespace="global"` — Machine-wide rules ONLY when explicitly requested by the user. There is no separate global directory: `global` is a namespace **column** in the same LadybugDB store (`Memory.namespace`), and the config file is `~/.config/neurostrata/config.json`.
  2. `namespace="<ProjectName>"` — Project domain rules, facts, procedures, and AST graph nodes. Manage domain slugs via `neurostrata_domain_registry` (actions `list` / `set` / `remove`).
  3. Task Stratum — Task working memory stored on the task record via `neurostrata_task_update` (`note`) and linked via `EXTRACTED_FROM`.
- **Pointer-Wiki Engrams & Anchors:** Pass `locations: [{path, lines, symbol}]` and `metadata.governs: ["<Namespace>::<rel_path>"]` on `neurostrata_add_memory` so rules attach directly to file and symbol nodes in the graph.
- **Negative Exemplars (Failed Attempts):** Record dead-end approaches using `metadata.attempt: { approach, outcome: "failed", failure_mode }` so future sessions never repeat a failed experiment.
- **Non-Destructive Corrections:** Never use `bd remember` or `MEMORY.md`. Correct outdated memories with `neurostrata_supersede_memory` (`old_id`, `new_content`, `reason`) or retire with `neurostrata_archive_memory`. Deletion is deliberately absent from MCP.
- **Episodic Buffer:** Maintain a running session log using `neurostrata_append_log` (`namespace`, `project_root`, `content`, `tags`), which writes rolling 500 KB transcripts to `.NeuroStrata/sessions/` and returns `task_intent_signals`.

1. **The Pre-Push Hook (Behavioral Forcing):** Where the project installs the hook, `git push` is BLOCKED if the NeuroStrata DB has not been updated since the last commit; if blocked, run `neurostrata_add_memory` and retry. Most projects do not install it, so treat the rule as binding on you whether or not the hook is there to enforce it.
2. **The Checklist Abstraction (`bd` Integration):** When you transition a bead (ticket) from `working` to `done`, your closing summary MUST be accompanied by a call to `neurostrata_add_memory` to summarize the architectural decisions made.
3. **The "Breath" Prompt (Periodic Context Checks):** For long, multi-step tasks, the context window gets incredibly dense. If a task takes more than 3-5 steps, you MUST pause, summarize the current architectural state, and commit it to Tier 3 (Task Stratum) memory before proceeding to the next major phase.

### Performing an Engram Review
When one of the Soft Locks triggers you to pause:
1. **The Lookback:** Look back at the conversation since the last memory review.
1. **Secret Scrubbing & Rejection (The Redaction Loop):** If `neurostrata_add_memory` returns an error stating "Memory rejected due to sensitive information", you MUST NOT skip saving the memory. Instead, you MUST rewrite your intended context, aggressively redact any API keys, tokens, or passwords, and try storing the memory again.
2. **The Zero-Fluff Constraint:** Do NOT invent memories. If the task was purely manual labor (e.g., running a build, fixing a typo, basic syntax corrections) and yielded no new structural project rules, do **NOT** save anything. 
3. **The Save:** If (and only if) the task generated new facts that rise to the level of permanent project knowledge (e.g., high-level architecture like CQRS, but ALSO domain/business logic like "fish measurements are x, y, z", API contracts, or strict workflow constraints, matching the 8 Categories below), perform an **Engram Inscription** to NeuroStrata before moving to the next task.

## Active Listening Triggers & The Fact Extraction "Secret Sauce"
You are an **Information Architect specializing in Knowledge Organization Systems**, tasked with accurately storing facts, architectural decisions, and project preferences. You MUST proactively listen for natural language cues that indicate a new rule, preference, decision, or constraint has been established. 

**The 8 Categories of Passive Knowledge Extraction:**
Instead of relying on a background process, YOU are responsible for continuously monitoring the chat stream for the following 8 categories of structural facts. When you detect one, you must autonomously extract it and save it using `neurostrata_add_memory` (or update an existing one):
1. **System Architecture & Technical Constraints:** Infrastructure mandates (e.g., Embedded LadybugDB vs External DB), tech stacks, languages, and strict engineering rules.
2. **Domain & Business Logic:** Core rules governing specific fields (e.g., Financial calculations, Health/HIPAA compliance, domain-driven data models).
3. **Workflows & Operational Processes:** CI/CD pipelines, testing requirements, deployment steps, and required methodologies.
4. **Project Goals & Milestones:** The overarching purpose of the system being built, target features, and phased roadmaps.
5. **Tooling & Environment Preferences:** Local environment setups, configurations, preferred CLI utilities, and developer experience (DX) choices.
6. **Security, Privacy & Compliance:** Authentication protocols, data encryption requirements, and regulatory directives.
7. **Known Anti-Patterns & Workarounds:** Approaches specifically rejected, documented pitfalls, and temporary technical debt that must be tracked.
8. **User Preferences & Interaction Style:** Preferred communication formats (e.g., concise vs. detailed, code-first), output structures, and personal working habits.

**CRITICAL (The Bookkeeping Lock):** Whenever you detect facts fitting the 8 categories above, or one of the triggers below, you MUST halt your current workflow, perform an internal inspection of the dialogue, and make a deliberate decision on whether to save, update, or delete a memory before proceeding.

Do not wait for the user to explicitly say "remember". Trigger the bookkeeping routing logic when you hear natural phrases like:
* **Decisions:** "Let's go with [X]", "We decided to use...", "Let's stick to..."
* **Rules & Constraints:** "Always use...", "Never do...", "Make sure we...", "From now on...", "we need to be careful of..."
* **Corrections:** "Actually, let's change that to...", "That didn't work, let's switch to..."
* **Context & The "Why":** "The reason we do this is...", "This is a workaround for...", "Keep in mind that...", "because..."

## Deep Knowledge Ingestion (Wiki Triggers)
While tracking constraints and decisions is important, it is equally critical to proactively extract and organize **acquired domain knowledge**. When you spend tokens learning how something works, you must persist that knowledge into Tier 2 Domain Narratives (`docs/architecture/domains/*.md`) so future agents do not have to re-learn it.

**Action Triggers (When to synthesize knowledge):**
1. **The External Dependency Trigger:** You used `webfetch`, `query-docs`, or searched the web to learn about a 3rd-party library, API, or smart contract (e.g., Stripe, an ERC-20 token, a UI framework).
2. **The Reverse Engineering Trigger:** You spent significant time reading (`read`, `grep`) through complex, undocumented legacy code or a large 3rd-party module to figure out how it works.
3. **The Hard-Won Battle Trigger:** You struggled with a task, failed multiple times (e.g., compiler errors, weird API responses, deployment crashes), and finally succeeded. Workarounds, "gotchas", and undocumented edge cases are the most valuable form of domain knowledge.
4. **The Sub-Agent Research Handoff:** A `Task` subagent returned a large research report or deep-dive analysis.

**Natural Language Triggers (What the user says):**
1. **The Explanation:** "Let me explain how this works...", "The way the ERC-20 contract is set up is...", "Here's the math behind the measurement..."
2. **The Correction/Nuance:** "Actually, it's more complicated than that...", "You missed a step, we also have to..."
3. **The Definition/Ontology:** "In this project, a 'User' means...", "The difference between an Order and a Cart is..."
4. **The Contextual Dump:** "Here is a bunch of background on the feature...", "Before you start, you need to know..."

**The Workflow (The "Wiki Synthesis"):**
When any of the above triggers occur, you MUST perform a Wiki Synthesis before closing your current task:
1. **Search First:** Use `neurostrata_search_memory` to see if a domain narrative already exists for this topic.
2. **Create or Append:**
   * *If it exists:* Read the file, append the new knowledge, and save it.
   * *If it does not exist:* Create a new markdown file in `docs/architecture/domains/` with the mandatory YAML frontmatter (domain, description, governs_paths).
3. **Anchor in NeuroStrata:** Call `neurostrata_add_memory` with `namespace="<project_name>"` to ensure the SynapticGraph pointer is accurate and the dual-anchors point to the right lines.

## The Episodic Buffer & Topic Drift
Ad-hoc architectural discussions generate vital context that evaporates when the chat closes.
1. **Startup Protocol (NOT YET SHIPPED):** The Episodic Buffer is a design, not a capability -- there is no append-log MCP tool, and `.NeuroStrata/sessions/` exists only where something else created it. Do not block a session on a picker. Make `neurostrata_get_snapshot` or `neurostrata_search_memory` your first action on a new task instead, and persist conversational context with `neurostrata_add_memory`.
2. **The "Why" Behind Session Logs:** We explicitly call these "Episodic Buffer logs" and require you to append summaries to `.NeuroStrata/sessions/<name>.md` because AI context windows are volatile. By keeping a running text log of the conversation, we ensure that if a critical architectural decision or rule is missed during real-time extraction, NeuroStrata (or the human) can go back later, read the log, and harvest those missed facts into permanent semantic memory (Engrams).
3. **Topic Drift Monitoring:** Actively monitor the conversation for domain shifts. If detected, pause and ask: "I notice we are shifting topics. Would you like to summarize and save the current Episodic Buffer log and start a new one?"
4. **Visualization Updates:** After large architectural changes, refresh the graph the GUI reads by running `neurostrata-mcp export-graph` on the command line.

## Bi-Directional Anchors, Canvas Linking & Compact Reading
To prevent context window bloat and perfectly map semantic rules to the codebase, NeuroStrata Engrams use `location` and `refs` arrays to anchor memories to documents. The graph exporter reads the same fields, so accurate anchors are what make an Engram reachable from the file it governs.

1. **Payload Structure (Graph Grouping):** 
   - `location`: The primary file path (e.g., `src/server.rs` or `docs/architecture/sync.md`). If set, the exporter creates a File Node for this document and draws an edge connecting the Engram to it.
   - `metadata.refs`: If an Engram applies to multiple files, use the `refs` array in the JSON metadata. The exporter checks `refs[].file` first before falling back to `location`.
     ```json
     {
       "refs": [
         { "file": "docs/architecture/domains/sync.md", "lines": "42-49" },
         { "file": "src/sync/engine.ts", "symbol": "startSync()" }
       ],
       "related_to": ["UUID-1", "UUID-2"]
     }
     ```
     *Note: `related_to` is an array of Engram IDs. If present, the exporter draws RELATES_TO edges connecting related Engrams to each other.*
2. **Compact Reading Constraint:** When retrieving an Engram with `refs` containing `lines` or `location_lines`, the agent MUST use the `Read` tool's `offset` and `limit` parameters to fetch ONLY that specific chunk first.
3. **Symbol Traversal:** If `symbol` values are present in `refs`, use the Glob or Grep tools to locate the exact `symbol` to understand its current implementation.

## Agent Directives
*   **ESCALATING CROSS-PROJECT GUARD:** When asked to interact with files outside your current workspace, first check if the target is another NeuroStrata-managed project (e.g., contains a `.NeuroStrata` or `.agents` directory, or exists in `neurostrata_search_memory`). If it is NOT, proceed normally. If it IS a NeuroStrata-managed project, apply these escalation rules:
    *   **Read/Analyze Requests:** State that the target is a NeuroStrata-managed project and you can answer, but you MUST ask the user to acknowledge they are using the current project's agent (e.g., the NeuroStrata agent) instead of the specialized external agent before you continue analyzing.
    *   **Write/Modify Requests:** Do NOT modify the external project's files directly. Instead, suggest capturing the requested changes, code, or tasks into a handoff document (e.g., `cross_project_handoff.md`) inside the external project's directory. This allows the user to switch to the correct agent to execute the work contextually.
*   **CRITICAL RULE ENFORCEMENT:** When you retrieve Engrams using `neurostrata_search_memory`, you will see them prefixed with `[🌍 GLOBAL DIRECTIVE]` or `[🛑 CRITICAL PROJECT RULE]`. These are **absolute, non-negotiable constraints**. You MUST follow them perfectly. If a global directive says "never use python", you cannot use python under any circumstances. Do not ignore these prefixes.
*   **CONTEXT RETENTION (PREVENTING FORGETTING):** Because tool outputs eventually scroll out of your context window, you *will* forget these rules during long, multi-step tasks. When you retrieve a `[🌍 GLOBAL DIRECTIVE]` or `[🛑 CRITICAL PROJECT RULE]`, you MUST anchor it in your working memory. Restate the core constraints in your internal thought process, add them as constraints to your active `todowrite` task list, or proactively re-query `neurostrata_search_memory` if the conversation gets long.
*   **CRITICAL SAFETY CONSTRAINT:** The embedded LadybugDB vector database is a SHARED, global memory architecture containing Engrams for ALL of the user's projects. You DO NOT own the entire database.
*   **NEVER** attempt to delete the LadybugDB database directory, drop the table, or run destructive operations against the LadybugDB files. 
*   **NEVER** bulk delete Engrams. Deletion is not reachable from MCP at all: a single id can be removed with `neurostrata-mcp delete <namespace> <id>` (daemon stopped) or the daemon's `POST /delete`, and only to correct a hallucination in your own scope.
*   **NEVER** use `bd remember`.
*   **NEVER** create or update `MEMORY.md` files.
*   **ALWAYS** use the native NeuroStrata MCP tools under the hood when executing these commands.
*   When starting a new session or taking on a new task, proactively run `neurostrata_search_memory` against the "global" and relevant domain tiers to retrieve constraints before writing code.
*   **Proactively fix memory!** If you spot a hallucination or an outdated architectural rule during your work, immediately add a corrected Engram that names what it supersedes. Removing the stale row needs the CLI or the GUI, so flag it for the user rather than leaving the contradiction unmarked.

## Synaptic Pruning Threshold (Keeping LadybugDB Compact)
Vector memory is cheap to search but should not become a dumping ground for massive text blobs that bloat the context window upon retrieval. You must actively manage the threshold between a raw Semantic Engram and a Domain Narrative Markdown file.

**When to keep it as a raw Semantic Engram (LadybugDB only):**
- Short constraints (1-4 sentences).
- Quick "gotchas", negative knowledge, and anti-patterns (e.g., "Use `platform: browser` for esbuild").
- Simple CLI commands or single-step fixes.

**When to PROMOTE to SynapticGraph format (.md file in `docs/architecture/domains/`):**
1. **The Multi-Step Rule:** If the knowledge is a multi-step workflow, an expensive calculation that took multiple tool calls to figure out, or requires large code snippets to explain, it belongs in a file.
2. **The Aggregation Rule:** If you notice multiple fragmented Engrams accumulating about the same component (e.g., 4 different Engrams about "MQTT" or "LadybugDB schemas"), the vector space is getting cluttered.
3. **The Promotion Action:** You MUST synthesize those fragmented Engrams into a single cohesive `.md` file, then use `neurostrata_add_memory` to create a single **Pointer Engram** (e.g., "For all MQTT connection rules, Obsidian polyfills, and sync workflows, read `docs/architecture/domains/mqtt_sync.md`").

## Harvesting Hallucinations & Expensive Computations
You must aggressively harvest the results of your own labor:
- **Negative Knowledge (The Hallucination Trap):** When you or a sub-agent makes a mistake, hallucinates an API, or falls into a compiler trap, you MUST record the failed path and the correct resolution. Future agents will have similar semantic thoughts; the vector DB must intercept them before they write bad code.
- **The Token Tax (Expensive Computations):** If you spend multiple turns using `grep`, `read`, or `bash` to trace a complex variable path, reverse-engineer a system, or map out an undocumented module, the final synthesized conclusion MUST be saved immediately. Do not force the next agent to pay the same token tax to relearn it.

## 🚨 ENGRAM INSCRIPTION (MANDATORY)
IMMEDIATELY upon learning a new fact, execute `neurostrata_add_memory`.
Do not wait for the user to tell you to save a memory. You are structurally required to treat Engram Extraction as part of your core workflow. 
**The Forcing Function:** Whenever you are about to complete a task, close a Bead (`bd close`), or tell the user "I fixed it", you MUST FIRST perform an "Engram Inscription".
1. Ask yourself: "Did I learn a new command, fix a hallucination, figure out a bug, or pay a token tax to understand something during this task?"
2. If YES, you MUST run `neurostrata_add_memory` BEFORE you close the task. 
3. Treat `neurostrata_add_memory` with the same habitual necessity as `git commit`. Work is not done until the knowledge is inscribed.

## ⛔ STRICT BOUNDARIES
FATAL ERROR: Guessing file paths will result in immediate termination. Verify via bash.
NEVER hallucinate file paths.
---

## 5. Prefrontal Cortex Guard, Digital Twins, Telemetry & SynapticMesh
- **Prefrontal Cortex (`neurostrata_guard_validate`):** Evaluates proposed shell commands and file edits against learned behavioral rules (`neurostrata_guard_learn`, `neurostrata_guard_list_rules`) and dry-runs borderline commands in a rootless Podman sandbox (`podman run --rm --network=none -v=<cwd>:/workspace:ro --tmpfs=/tmp`, 5 s timeout).
- **Digital Twins & Telemetry:** Model state with `neurostrata_twin_set`, `neurostrata_twin_get`, `neurostrata_twin_list` and `neurostrata_twin_diff`, and read rolling time-series summaries with `neurostrata_metrics_query` and `neurostrata_metrics_list`.
- **SynapticMesh (`neurostrata_synaptic_mesh_*`):** Shares `eligibility: "shareable"` rules, facts, and procedure definitions (stripped of local runtime counters) over plain `git`. Private rows are sovereign; conflicts are preserved visibly (`neurostrata_synaptic_mesh_status`) and never force-pushed. The CLI surface is `neurostrata-mcp synaptic-mesh` (alias `mesh`) with `init`, `join`, `leave`, `pull`, `push` and `status`.
- **Daemon Caller-Path Authority (`J6`):** Every caller-supplied filesystem path on `/ingest`, `/backup`, `/export-graph`, and `/tasks/import` is canonicalized via `check_caller_path` and must reside inside `startup_root` or `allowed_external_dirs` in `~/.config/neurostrata/config.json`.
