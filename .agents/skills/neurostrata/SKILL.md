---
name: neurostrata
description: "Use NeuroStrata task, memory, and session-log MCP tools safely."
---
# NeuroStrata (The Tri-Strata Model)

## Overview
NeuroStrata is the standard operating protocol for persisting and retrieving knowledge across sessions. It uses a native Rust Model Context Protocol (MCP) server connected directly to an embedded LadybugDB vector database partitioned into three distinct tiers (The Tri-Strata Model).

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
* `neurostrata_append_log`: Append a timestamped entry to the project's Episodic Buffer. This tool is registered in `src/server.rs`.

The MCP memory tools do not delete records. Memory correction uses
`neurostrata_supersede_memory`. To delete one, the user runs
`neurostrata-mcp delete <namespace> <id>` with the daemon stopped, or the GUI posts to the
daemon's `/delete`; such destructive operations require explicit human
authorization and must not target shared or live data. To merge two spellings of a project, the user runs
`neurostrata-mcp move <from> <id> <to>`, which is what `doctor` prints.

**Ids and namespaces.** A node id is a repository-relative path (`src/parser/ingest.rs`),
so that is the form to use in `locations` when a memory governs a file; the absolute path
lives in metadata as `absolute_path`. A namespace is the project name, never the checkout
folder, and names differing only by case resolve to the one already stored.

## SynapticGraph Integrity Rules
To prevent broken graphs and dead links, all Domain Narratives (markdown files) must adhere to these rigid constraints:
1. **Strict Namespacing**: All narrative markdown files must be placed in exactly one protected directory: `docs/architecture/domains/`. Do not pollute the root or random folders.
2. **Mandatory YAML Frontmatter**: Every narrative file must contain strict frontmatter declaring the domain, description, and the exact code paths it governs. This creates indestructible, hard-coded edges in the AST graph.
   ```yaml
   ---
   domain: "domain_name"
   description: "Concise summary of narrative"
   governs_paths:
     - "path/to/code/"
     - "path/to/another/file.go"
   ---
   ```
3. **Read-Before-Write Validation Lock**: When querying NeuroStrata and receiving a pointer to a narrative, you MUST use the `Read` tool to load it before writing code. If the file is missing or renamed, you must HALT, repair the mesh (locate the file or rewrite it), and update NeuroStrata before continuing.

## Smart Routing Interface & CRUD Autonomy
You (the Agent) are responsible for the bookkeeping. The user should not have to use rigid syntax. 

## Task Availability and Delegation

Use NeuroStrata task tools to find or create and claim work before editing.
Delegate only through an available, authorized agent mechanism; do not invent
an agent or substitute another tracker. If required tools are unavailable,
continue read-only discovery and report the missing tools. Stop before mutation
unless the owner grants a one-time exception naming the bounded task and
files/actions. That exception does not create task or memory state, waive other
controls, or establish a standing bypass.

## 🧩 SEQUENTIAL THINKING MANDATE
Record progress and blockers with `neurostrata_task_update`. Verify changes
against applicable checks and summarize evidence before task completion; never
claim checks that did not run.



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

## Task Completion & Compaction Defense

1. **Pre-push hook:** When installed and `neurostrata-mcp` is available, the
   hook runs `neurostrata-mcp task gate <namespace> --strict`. Resolve reported
   task-gate violations before an authorized push. A hook is not guaranteed
   to exist in every checkout.
2. **Task completion:** Use `neurostrata_task_complete` to mark a task done.
   It requires a genuine memory extraction, supplied inline or linked to an
   existing memory; do not invent an extraction. If completion is blocked,
   report the blocker.
3. **Context checks:** For long tasks, summarize the state and record progress
   with `neurostrata_task_update` before proceeding to the next major phase.
4. Verify fetch/push URLs before publishing. Push only to the authorized
   destination; never push upstream unless the owner explicitly authorizes it.


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

1. **Startup:** Begin with `neurostrata_get_snapshot` or
   `neurostrata_search_memory` before edits. `neurostrata_append_log` is shipped
   and appends to `.NeuroStrata/sessions/`; use it for concise project context
   where appropriate. Never log secrets or unrelated personal data.
2. Episodic Buffer entries preserve useful project context so durable rules
   can be reviewed and explicitly extracted into memory later.
3. Monitor topic shifts and use tagged append-log entries when appropriate.
4. After large architectural changes, refresh the graph only when the
   applicable project workflow requires it.


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
*   **CONTEXT RETENTION:** When a long task risks losing important retrieved rules, keep a concise working summary or re-query `neurostrata_search_memory`. Do not substitute an unapproved tracker.
*   **CRITICAL SAFETY CONSTRAINT:** The embedded LadybugDB vector database is a SHARED, global memory architecture containing Engrams for ALL of the user's projects. You DO NOT own the entire database.
*   **NEVER** attempt to delete the LadybugDB database directory, drop the table, or run destructive operations against the LadybugDB files. 
*   **NEVER** bulk delete Engrams. Deletion is not reachable from MCP at all: a single id can be removed with `neurostrata-mcp delete <namespace> <id>` (daemon stopped) or the daemon's `POST /delete`, and only to correct a hallucination in your own scope.
*   **NEVER** use `bd` commands as task or memory tracking substitutes.
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

## Project Validation Boundaries

- Keep the optional Python harness, tests, and fixtures under
  `tools/runtime-validation/`, separate from Rust source/runtime/build. It
  uses only the standard library; Cargo excludes it from source packages.
  Do not add a Python runtime/build dependency.
- Report locked default-feature CI evidence at the exact commit reviewed. A
  local `ort-load-dynamic` override is a separate setup, not hosted CI evidence
  or a declared Cargo feature in this checkout.
- Real-model, performance, and migration outcomes remain unverified unless a
  measured run and its evidence support the claim.

## Harvesting Hallucinations & Expensive Computations
You must aggressively harvest the results of your own labor:
- **Negative Knowledge (The Hallucination Trap):** When you or a sub-agent makes a mistake, hallucinates an API, or falls into a compiler trap, you MUST record the failed path and the correct resolution. Future agents will have similar semantic thoughts; the vector DB must intercept them before they write bad code.
- **The Token Tax (Expensive Computations):** If you spend multiple turns using `grep`, `read`, or `bash` to trace a complex variable path, reverse-engineer a system, or map out an undocumented module, the final synthesized conclusion MUST be saved immediately. Do not force the next agent to pay the same token tax to relearn it.

## 🚨 ENGRAM INSCRIPTION (MANDATORY)
IMMEDIATELY upon learning a new fact, execute `neurostrata_add_memory`.
Do not wait for the user to tell you to save a memory. You are structurally required to treat Engram Extraction as part of your core workflow. 
Before task completion, consider whether the work produced durable project knowledge.
1. Do not invent memories or store secrets.
2. When durable knowledge was learned, use `neurostrata_add_memory` with the
task's extracted-from metadata, or provide a genuine extraction through
`neurostrata_task_complete`.
3. Do not claim task completion or memory persistence unless the tool confirms it.

## ⛔ STRICT BOUNDARIES
FATAL ERROR: Guessing file paths will result in immediate termination. Verify via bash.
NEVER hallucinate file paths.
