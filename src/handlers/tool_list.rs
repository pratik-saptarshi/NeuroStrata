//! The `tools/list` catalogue (Task 20): the static tool definitions,
//! the SynapticMesh module's appended definitions, and the vocabulary
//! overlay that rewrites descriptions from the shipped schema. Moved
//! out of `process_mcp_request` verbatim; the content and the ordering
//! are the wire contract the tools/list test pins
//! (`tools_list_carries_all_thirty_tools_in_a_stable_order`).

use serde_json::Value;

/// Build the `tools/list` result value: the static catalogue, the mesh
/// definitions appended, the vocabulary summaries overlaid.
pub fn tool_list() -> Value {
        let mut result = serde_json::json!({
            "tools": [
                {
                    "name": "neurostrata_add_memory",
                    "description": "Store an architectural rule, project pattern, or task insight. Task recognition: if the content contains a task-shaped statement (TODO:, 'we should also', 'later we need to', 'remaining work:', 'defer'/'deferred'), consider neurostrata_task_create instead of or in addition to storing it as a memory.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string", "description": "The text of the memory to save." },
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'NeuroStrata') or 'global'. Do not use folder paths." },
                            "project_root": { "type": "string", "description": "The absolute path to the project root directory where the agent is currently working." },
                            "memory_type": { "type": "string", "description": "Use rule or fact; task for tracked work; procedure for a knowing-how routine that fires on a trigger or a repetition budget (perform it with neurostrata_procedure_perform); a fact may carry metadata.topology (component, target_system, channel, credential_boundary, reason), the access path the fact describes; a fact may also carry metadata.attempt (approach, outcome, failure_mode, context) with outcome \"failed\", evidence of one failed episode and not a rule; the namespace's registry row carries metadata.domain_registry.domains, the sorted list of the domain slugs the namespace uses; structural types are directory/file/markdown/symbol; custom types remain valid; default context. (This description is overlaid at runtime from src/schemas/memory-vocabulary.v3.json tool_summaries.memory_type; if it has drifted from the shipped vocab, the overlay wins, but the static value should stay in sync so the source is not misleading.)" },
                            "create_new_namespace": { "type": "boolean", "description": "Set to true ONLY if you are absolutely certain this is a brand new project namespace that doesn't exist yet." },
                            "user_id": { "type": "string", "description": "The user making the request." },
                            "agent_name": { "type": "string", "description": "The name of the agent storing the memory." },
                            "locations": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string", "description": "File path (e.g. docs/architecture.md)" },
                                        "lines": { "type": "string", "description": "Line numbers (e.g. 42-49)" },
                                        "symbol": { "type": "string", "description": "Code symbol (e.g. startSync())" },
                                        "source_context": { "type": "object", "description": "Optional opaque provenance from a configured source provider (for example, revision/worktree identity). Core stores and exports it without assuming Git or interpreting provider fields." }
                                    }
                                },
                                "description": "An array of file paths, line numbers, and symbols this memory governs. Memories MUST reference the specific documents they belong to."
                            },
                            "domain": { "type": "string", "description": "Optional category or domain this rule belongs to (e.g., 'frontend', 'database', 'devops', 'api'). It is stored normalized to a lowercase-kebab slug ('Database Access' becomes 'database-access'); a value that normalizes to nothing is stored exactly as given, and a non-string value is rejected." },
                            "register_domain": { "type": "boolean", "description": "Set to true ONLY if the domain is genuinely new for this namespace; the registry rejects any other unregistered domain." },
                            "related_to": { "type": "array", "items": { "type": "string" }, "description": "Optional list of memory IDs this rule connects to, forming a knowledge graph edge." },
                            "metadata": { "type": "object", "description": "Optional dictionary with Bi-Directional Anchors" }
                        },
                        "required": ["content", "namespace", "project_root"]
                    }
                },
                {
                    "name": "neurostrata_get_memory",
                    "description": "Fetch a single memory by id. Use it to read a memory before editing it, and to follow a Related Nodes or Governs pointer to the exact record instead of guessing at it with a search.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "The id of the memory to fetch." },
                            "namespace": { "type": "string", "description": "The namespace the memory lives in." }
                        },
                        "required": ["id", "namespace"]
                    }
                },
                {
                    "name": "neurostrata_get_snapshot",
                    "description": "Get a pre-computed cognitive snapshot of the most important active architectural rules for a project. Use this immediately upon starting a new task to ground yourself in the project's core architecture before searching.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'NeuroStrata') or 'global'." },
                            "domain": { "type": ["string", "array"], "items": { "type": "string" }, "description": "Optional domain slug(s) to surface in the domain strap, regardless of access_count." }
                        },
                        "required": ["namespace"]
                    }
                },
                {
                    "name": "neurostrata_ingest_directory",
                    "description": "Batch ingest and parse the Abstract Syntax Tree (AST) of the current project directory to build the Software Graph.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "dir_path": { "type": "string", "description": "Absolute path to the project directory to ingest. Usually the current working directory." },
                            "namespace": { "type": "string", "description": "The project namespace." }
                        },
                        "required": ["dir_path", "namespace"]
                    }
                },
                {
                    "name": "neurostrata_list_namespaces",
                    "description": "List all existing project namespaces in the database. Use this to prevent hallucinating namespace names.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "neurostrata_search_memory",
                    "description": "Search the project's long-term memory for architectural rules.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "What to search for." },
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'NeuroStrata') or 'global'. Do not use folder paths." },
                            "include_tasks": { "type": "boolean", "description": "Include memory_type 'task' rows in the results. They are excluded by default so task titles do not pollute rule recall." },
                            "domain": { "type": ["string", "array"], "items": { "type": "string" }, "description": "Optional domain filter. A slug, or a list of slugs. Normalized to lowercase-kebab. A filter that matches nothing is skipped rather than emptying the results, and the report says which happened." }
                        },
                        "required": ["query", "namespace"]
                    }
                },
                {
                    "name": "neurostrata_supersede_memory",
                    "description": "Correct a memory that is wrong or out of date. Stores your new text as a new memory and retires the old one, which keeps its original wording as history and stops being returned by search. Re-embeds, so search matches the new wording and not the words you replaced. This is the only tool on this surface that changes an existing memory.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "The id of the memory being corrected. Get it from a search result or neurostrata_get_memory." },
                            "namespace": { "type": "string", "description": "The namespace the memory lives in." },
                            "content": { "type": "string", "description": "The corrected text, written in full. It replaces the old wording rather than being appended to it." },
                            "governs": { "type": "array", "items": { "type": "string" }, "description": "Replacement structural pointers. Passed = replaces the inherited value wholesale; absent = the old value carries over and the reply names it." },
                            "related_to": { "type": "array", "items": { "type": "string" }, "description": "Replacement related_to ids. Passed = replaces; absent = carries over and is reported." },
                            "contained_by": { "type": "array", "items": { "type": "string" }, "description": "Replacement contained_by ids. Passed = replaces; absent = carries over and is reported." },
                            "locations": { "type": "array", "items": { "type": "object" }, "description": "Replacement anchors: [{path, lines?, symbol?, source_context?}]. source_context is opaque provenance from a configured provider; Core does not assume Git. Replaces refs wholesale and re-derives governs from the paths, exactly like add_memory." },
                            "reason": { "type": "string", "description": "WHY this memory is being corrected, in one clause. Recorded as superseded_reason on the retired memory, so the history says what was wrong, not merely that something changed. Omit it and the reply says the history will not carry a why." },
                            "source": { "type": ["string", "object"], "description": "Provenance of the CORRECTED text, with the same semantics add_memory's metadata.source accepts: a dated string, or {kind: owner-quote|doc|transcript|commit|derived, ref, captured_at}. Written to metadata.source on the replacement. Passed = replaces the old provenance; absent = the old one carries over and the reply names it." },
                            "allow_global": { "type": "boolean", "description": "Required to supersede anything in the machine-wide 'global' namespace, whose rules apply to every project on this machine." }
                        },
                        "required": ["id", "namespace", "content"]
                    }
                },
                {
                    "name": "neurostrata_procedure_perform",
                    "description": "Acknowledge one firing of a 'procedure' memory: stamp last_performed_at, increment performance_count, decrement remaining_fires (its iteration budget), and write the Episodic Buffer entry that last_episodic_pointer names. A procedure past its valid_to answers lapsed:true, one with no fires left answers spent:true, and neither mutates the row. CREATE A PROCEDURE FIRST via `neurostrata_add_memory` with `memory_type: \"procedure\"` (fields: trigger, remaining_fires, valid_to, performance_count, last_performed_at, last_episodic_pointer) — this tool only stamps; it does not insert.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Memory id of the procedure." },
                            "note": { "type": "string", "description": "Optional free-text note for the Episodic Buffer entry." },
                            "project_root": { "type": "string", "description": "Absolute path to the project root; the Episodic Buffer lives at <project_root>/.NeuroStrata/sessions/. Defaults to the working directory." }
                        },
                        "required": ["id"]
                    },
                    "outputSchema": {
                        "type": "object",
                        "properties": {
                            "performed": { "type": "boolean" },
                            "lapsed": { "type": "boolean" },
                            "spent": { "type": "boolean" },
                            "remaining_fires": { "type": ["integer", "null"] },
                            "performance_count": { "type": "integer" },
                            "episodic_pointer": { "type": ["string", "null"] },
                            "reason": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "neurostrata_task_create",
                    "description": "Create a tracked unit of work. Zero-Action Start: no file edits before a task exists and is claimed by this session. Task recognition: a statement of future/deferred work (TODO:, FIXME:, 'we should also', 'later we need to', 'remaining work:', 'defer'/'deferred') is a task, not a note — capture it here instead of letting it evaporate.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'MyProj'). New projects must call neurostrata_bootstrap first." },
                            "title": { "type": "string", "description": "One line: the outcome this task delivers." },
                            "description": { "type": "string", "description": "Optional longer context for the work." },
                            "task_type": { "type": "string", "enum": ["task", "bug", "feature", "epic"], "description": "What kind of work this is. Defaults to 'task'." },
                            "priority": { "type": "integer", "enum": [0, 1, 2, 3, 4], "description": "0 is highest. Defaults to 2." },
                            "labels": { "type": "array", "items": { "type": "string" }, "description": "Free-form tags carried on the record." },
                            "parent_id": { "type": "string", "description": "Id of an existing task this one contains (epic/subtask)." },
                            "blocked_by": { "type": "array", "items": { "type": "string" }, "description": "Ids of tasks this one waits on; a ready task has none unfinished." }
                        },
                        "required": ["namespace", "title"]
                    }
                },
                {
                    "name": "neurostrata_task_claim",
                    "description": "Claim a task for this session (open -> in_progress). Refused while another live session holds it; a stale claim (no update for 60 minutes) is released and taken over.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "The task id, e.g. 'myproj-x7q2'." },
                            "namespace": { "type": "string", "description": "The exact project name." },
                            "assignee": { "type": "string", "description": "Who is doing the work (the agent or operator name)." },
                            "session_id": { "type": "string", "description": "Unique id of this session; the exclusivity check compares it against the holder's." }
                        },
                        "required": ["id", "namespace", "assignee", "session_id"]
                    }
                },
                {
                    "name": "neurostrata_task_update",
                    "description": "Update a task: title (the rename surface: rewrites the task's content and re-embeds it, with the old and new wording appended to history), status (open/in_progress/blocked), a note appended to history (the Breath prompt lands here), priority, assignee, labels, or blockers. 'done' is rejected here on purpose: finish with neurostrata_task_complete.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "The task id." },
                            "namespace": { "type": "string", "description": "The exact project name." },
                            "title": { "type": "string", "maxLength": 200, "description": "New title; replaces the old one and re-embeds so search matches the new wording. Non-empty, 200 characters max." },
                            "status": { "type": "string", "enum": ["open", "in_progress", "blocked"], "description": "New state. 'done' is deliberately absent: it is reachable only through neurostrata_task_complete." },
                            "note": { "type": "string", "description": "Why, in one line. Required for blocked; appended to the task's history in every case." },
                            "priority": { "type": "integer", "enum": [0, 1, 2, 3, 4], "description": "0 is highest." },
                            "assignee": { "type": "string", "description": "Who is working this task." },
                            "add_labels": { "type": "array", "items": { "type": "string" }, "description": "Labels to add without replacing the existing ones." },
                            "blocked_by": { "type": "array", "items": { "type": "string" }, "description": "Replaces the blocker list outright (an empty array clears it)." }
                        },
                        "required": ["id", "namespace"]
                    }
                },
                {
                    "name": "neurostrata_task_list",
                    "description": "List tasks in a namespace. ready: true is the actionable backlog (open and every blocker done); done tasks are hidden unless include_done; retired rows (superseded or expired) are history and hidden unless include_retired.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name." },
                            "status": { "type": "string", "enum": ["open", "in_progress", "blocked", "done"], "description": "Only tasks in this state." },
                            "assignee": { "type": "string", "description": "Only tasks claimed by this assignee." },
                            "ready": { "type": "boolean", "description": "Only actionable work: open (not retired), with every blocker done. A retired blocker counts as done -- it is history, unless the row at the end of its supersession chain still carries the work." },
                            "include_done": { "type": "boolean", "description": "Include finished tasks in the unfiltered listing." },
                            "include_retired": { "type": "boolean", "description": "Include retired rows (superseded or expired) in the listing. They come back labelled retired: true -- history, not work." }
                        },
                        "required": ["namespace"]
                    }
                },
                {
                    "name": "neurostrata_task_complete",
                    "description": "Finish a task (-> done). Lock 2: succeeds only when at least one memory is extracted from it first -- pass memory.content (what did this task teach?), link_memory_id, or write the extraction yourself with neurostrata_add_memory and metadata extracted_from: [id]. Otherwise it fails with -32603 naming both ways to comply.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "The task id to complete." },
                            "namespace": { "type": "string", "description": "The exact project name." },
                            "reason": { "type": "string", "description": "What closed it; recorded on the done transition." },
                            "register_domain": { "type": "boolean", "description": "Set to true ONLY if memory.metadata.domain is genuinely new for this namespace; the registry rejects any other unregistered domain. It applies to memory.metadata.domain and is a no-op when the memory carries no domain." },
                            "memory": {
                                "type": "object",
                                "description": "Write the extraction inline; it lands with the completion, through the same pipeline as neurostrata_add_memory.",
                                "properties": {
                                    "content": { "type": "string", "description": "What did you learn? What rule does this task prove?" },
                                    "memory_type": { "type": "string", "description": "Defaults to 'fact'." },
                                    "metadata": { "type": "object", "description": "Optional key-value map persisted exactly as neurostrata_add_memory persists one (e.g. source {kind, ref, captured_at} for a Source-necessary rule); extracted_from is stamped with this task if absent." },
                                    "locations": { "type": "array", "items": { "type": "object" }, "description": "The same location anchors neurostrata_add_memory accepts." }
                                },
                                "required": ["content"]
                            },
                            "follow_up_tasks": { "type": "array", "items": { "type": "object", "properties": { "title": { "type": "string" }, "description": { "type": "string" }, "task_type": { "type": "string", "enum": ["task", "bug", "feature", "epic"] }, "priority": { "type": "integer", "enum": [0,1,2,3,4] }, "labels": { "type": "array", "items": { "type": "string" } } }, "required": ["title"] }, "description": "Optional list of follow-up tasks to create after this task is done. Each becomes an open task with spawned_from=parent and a related_to edge." },
                            "link_memory_id": { "type": "string", "description": "Bind an already-written memory to this task via EXTRACTED_FROM. Its vector is reused as-is; linking never re-embeds." }
                        },
                        "required": ["id", "namespace"]
                    }
                },
                {
                    "name": "neurostrata_task_validate",
                    "description": "Advisory gate report: {ok, violations, stale, unextracted_done, counts}. The same engine the pre-push hook runs, but it never blocks anything.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name." }
                        },
                        "required": ["namespace"]
                    }
                },
                {
                    "name": "neurostrata_bootstrap",
                    "description": "New project setup: creates the project's first task server-side and returns files to write (AGENTS.md, .NeuroStrata/docs), the hook install, and claim/ingest instructions as JSON for you to execute in order.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'MyProj'). Not a folder path." },
                            "project_root": { "type": "string", "description": "Absolute path to the project root." },
                            "project_description": { "type": "string", "description": "One line on what this project is; recorded on the first task." }
                        },
                        "required": ["namespace", "project_root"]
                    }
                },
                {
                    "name": "neurostrata_task_setup",
                    "description": "Existing project setup: reads the repo (git remote, languages, CI, .beads, hooks, AGENTS.md) and returns suggested rules, migration tasks, conflicts, and instructions. Writes nothing itself -- execute the returned instructions.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name." },
                            "project_root": { "type": "string", "description": "Absolute path to the project root." }
                        },
                        "required": ["namespace", "project_root"]
                    }
                },
                {
                    "name": "neurostrata_append_log",
                    "description": "Episodic Buffer writer: append a timestamped entry to the project's rolling session log, with a Topic Switch marker on tagged turns. Run it silently in the background as you work (the Continuous Backup Protocol).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string", "description": "The log entry: what just happened, what changed, what was decided." },
                            "project_root": { "type": "string", "description": "Absolute path to the project root; the buffer lives at <project_root>/.NeuroStrata/sessions/." },
                            "tags": { "type": "array", "items": { "type": "string" }, "description": "Topic tags for this turn (e.g. ['auth', 'database']). When present, a Topic Switch marker is injected." },
                            "namespace": { "type": "string", "description": "The exact project name. Optional: defaults to the project root's directory name, resolved against the known namespaces." }
                        },
                        "required": ["content", "project_root"]
                    }
                },
                {
                    "name": "neurostrata_guard_validate",
                    "description": "Validate an action against stored behavioral rules and (optionally) execute it in a sandboxed Podman exec. Returns a ValidateResponse carrying a ValidateVerdict: DeterministicReject, SandboxReject, SandboxPassHighFidelity, SandboxPassLowFidelity, or ApprovedFailOpen. Wraps src/guard/validator::validate.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'NeuroStrata') or 'global'. Defaults to 'global'." },
                            "action_type": { "type": "string", "description": "Type of action: 'bash', 'write_file', etc." },
                            "payload": { "type": "string", "description": "The actual command or file content to validate." },
                            "cwd": { "type": "string", "description": "Current working directory for sandbox execution. Defaults to empty." },
                            "use_sandbox": { "type": "boolean", "description": "Whether to execute the action in a sandbox after the semantic check. Defaults to false." }
                        },
                        "required": ["action_type", "payload"]
                    }
                },
                {
                    "name": "neurostrata_guard_learn",
                    "description": "Teach a new behavioral rule to the guard namespace. The rule's trigger_pattern is matched against future validated actions; on a match, the action is rejected with the rule's constraint_text. Wraps src/guard/validator::add_rule.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name or 'global'. Defaults to 'global'." },
                            "rule": {
                                "type": "object",
                                "description": "The BehavioralRule to store.",
                                "properties": {
                                    "id": { "type": "string", "description": "Stable id for this rule." },
                                    "version": { "type": "integer", "description": "Rule version (monotonic)." },
                                    "rule_class": { "type": "string", "description": "Class: 'secret-reject', 'churn-reject', 'constraint-injection', 'safety', etc." },
                                    "trigger_pattern": { "type": "string", "description": "Lowercase substring or phrase matched against action_type/payload." },
                                    "constraint_text": { "type": "string", "description": "Human-readable rule statement shown on violation." },
                                    "hit_count": { "type": "integer", "description": "How many times the rule has triggered. Usually 0 for new rules." },
                                    "status": { "type": "string", "description": "'active' or 'deprecated'. Defaults to 'active'." }
                                },
                                "required": ["id", "rule_class", "trigger_pattern", "constraint_text"]
                            }
                        },
                        "required": ["rule"]
                    }
                },
                {
                    "name": "neurostrata_guard_list_rules",
                    "description": "List all behavioral rules stored in the guard namespace. Wraps src/guard/validator::list_rules.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name or 'global'. Defaults to 'global'." }
                        }
                    }
                },
                {
                    "name": "neurostrata_archive_memory",
                    "description": "Mark a memory as archived by writing a tombstone beside it. The row keeps its content, id and metadata; archived is merged in rather than replacing what it was there. Metadata-only: no retrieval path filters on it yet, so an archived memory still appears in search_memory in 1.8.0.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "The id of the memory to archive." },
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'NeuroStrata') the memory lives in." }
                        },
                        "required": ["id", "namespace"]
                    }
                },
                {
                    "name": "neurostrata_task_intake",
                    "description": "Submit text for durable task intake. Runs M2 lexical detection to find task-shaped statements, creates an immutable receipt (source_kind, source_revision, source_digest), and spawns candidate(s) with review_state=pending. Returns receipt and candidates. Explicit zeros when no candidates.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name (e.g., 'NeuroStrata')." },
                            "source_kind": { "type": "string", "enum": ["conversation", "manual", "imported", "tool"], "description": "Origin of the intake. Defaults to conversation." },
                            "source_revision": { "type": "string", "description": "Revision identifier (git commit, timestamp, etc.) for traceability." },
                            "source_digest": { "type": "string", "description": "Content hash for idempotency; same digest = same receipt." },
                            "text": { "type": "string", "description": "Raw text to analyze for task intents." },
                            "title": { "type": "string", "description": "Optional explicit title (for manual intake)." },
                            "description": { "type": "string", "description": "Optional explicit description (for manual intake)." }
                        },
                        "required": ["namespace", "source_revision", "source_digest", "text"]
                    }
                },
                {
                    "name": "neurostrata_task_intake_list",
                    "description": "List intake receipts or candidates with filters and pagination. Always returns explicit zeros for count/total/offset/limit. Candidates are NOT in task_list, memory recall, graph export, or mesh.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name." },
                            "mode": { "type": "string", "enum": ["receipts", "candidates"], "description": "What to list. Defaults to candidates." },
                            "review_state": { "type": "string", "enum": ["pending", "in_review", "accepted", "dismissed"], "description": "Filter candidates by review state." },
                            "disposition": { "type": "string", "enum": ["pending", "created", "linked", "dismissed"], "description": "Filter candidates by disposition." },
                            "limit": { "type": "integer", "description": "Max items to return. Default 50." },
                            "offset": { "type": "integer", "description": "Pagination offset. Default 0." }
                        },
                        "required": ["namespace"]
                    }
                },
                {
                    "name": "neurostrata_task_intake_resolve",
                    "description": "Resolve a candidate with CAS (compare-and-swap). Disposition: created (spawns task), linked (attaches to existing task), dismissed. Requires cas_version for optimistic concurrency. Returns updated candidate and task_created flag.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "The exact project name." },
                            "candidate_id": { "type": "string", "description": "The candidate id to resolve." },
                            "disposition": { "type": "string", "enum": ["created", "linked", "dismissed"], "description": "Resolution outcome." },
                            "task_id": { "type": "string", "description": "Task id when disposition is created or linked." },
                            "cas_version": { "type": "integer", "description": "Expected cas_version for optimistic concurrency." }
                        },
                        "required": ["namespace", "candidate_id", "disposition", "cas_version"]
                    }
                }
            ]
        });
        // SynapticMesh surface: the schemas live with the module so the
        // CLI, the adapter, and this list cannot drift into three
        // different shapes of the same promise.
        if let Some(tools) = result.get_mut("tools").and_then(|t| t.as_array_mut()) {
            tools.extend(crate::synaptic_mesh::synaptic_mesh_tool_definitions());
        }
        // Append vocabulary summaries to tool descriptions from the shipped schema.
        {
            let vocab = crate::traits::memory_vocabulary();
            if let Some(tools) = result.get_mut("tools").and_then(|t| t.as_array_mut()) {
                if let Some(summaries) = vocab.get("tool_summaries") {
                    for tool in tools.iter_mut() {
                        if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                            let key = match name {
                                "neurostrata_add_memory" => Some("add"),
                                "neurostrata_search_memory" => Some("search"),
                                "neurostrata_ingest_directory" => Some("ingest"),
                                _ => None,
                            };
                            if let Some(key) = key {
                                if let Some(summary) =
                                    summaries.get(key).and_then(|s| s.as_str())
                                {
                                    if let Some(old_desc) =
                                        tool.get("description").and_then(|d| d.as_str())
                                    {
                                        let new_desc = format!("{} {}", old_desc, summary);
                                        tool["description"] = serde_json::json!(new_desc);
                                    }
                                }
                            }
                        }
                    }
                }
                // Replace memory_type property description in add_memory.
                if let Some(mem_type_desc) = vocab
                    .get("tool_summaries")
                    .and_then(|s| s.get("memory_type"))
                    .and_then(|s| s.as_str())
                {
                    for tool in tools.iter_mut() {
                        if tool.get("name").and_then(|n| n.as_str())
                            == Some("neurostrata_add_memory")
                        {
                            if let Some(d) = tool
                                .get_mut("inputSchema")
                                .and_then(|s| s.get_mut("properties"))
                                .and_then(|p| p.get_mut("memory_type"))
                                .and_then(|mt| mt.get_mut("description"))
                            {
                                *d = serde_json::json!(mem_type_desc);
                            }
                            break;
                        }
                    }
                }
            }
        }
    result
}
