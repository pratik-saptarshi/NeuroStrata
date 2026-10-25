use crate::traits::{Embedder, MemoryPayload, SearchResult, VectorStore};
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::events::{RecursionToken, ThalamicBus, ThalamicPulse};
use crate::guard::{BehavioralRule, GuardValidator, ValidateResponse};

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[allow(dead_code)]
    pub params: Option<Value>,
}

#[derive(Serialize)]
pub struct JsonRpcResponse<T> {
    jsonrpc: String,
    id: Option<Value>,
    // A response carries a result or an error, never both.
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

impl<T> JsonRpcResponse<T> {
    pub fn success(id: Option<Value>, result: T) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: Option<Value>, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(serde_json::json!({ "code": code, "message": message })),
        }
    }
}

/// The request was not JSON.
const PARSE_ERROR: i64 = -32700;
/// The request was JSON-RPC, and answering it failed here rather than at the
/// far end.
const INTERNAL_ERROR: i64 = -32603;

#[allow(dead_code)]
impl JsonRpcResponse<Value> {
    pub fn error(id: Option<Value>, error: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

pub async fn process_mcp_request(
    request: JsonRpcRequest,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
    ingests: Arc<crate::ingest_jobs::IngestJobs>,
    deduplication_checker: Option<Arc<crate::judgment::DeduplicationChecker>>,
    bus: Arc<crate::events::ThalamicBus>,
) -> Value {
    let id = request.id.clone();
    match request.method.as_str() {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "neurostrata-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": {}
                }
            });
            serde_json::to_value(JsonRpcResponse::success(id.clone(), result)).unwrap_or_else(|e| {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32603,
                        "message": format!("Internal serialization error: {}", e)
                    }
                })
            })
        }
        "notifications/initialized" => {
            serde_json::json!({})
        }
        "tools/list" => {
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
            serde_json::to_value(JsonRpcResponse::success(id.clone(), result)).unwrap_or_else(|e| {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32603,
                        "message": format!("Internal serialization error: {}", e)
                    }
                })
            })
        }
        "tools/call" => {
            let mut result_text = "Tool execution failed".to_string();
            // Set this to a JSON-RPC error response to short-circuit the
            // normal success path. Used when a handler fails at encoding
            // and the error must surface as a -32603, not as a success
            // payload containing an error string.
            let mut error_response: Option<Value> = None;
            if let Some(params) = &request.params {
                if let Some(name) = params.get("name").and_then(|n| n.as_str()) {
                    let arguments = params
                        .get("arguments")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));

                    let ctx = crate::handlers::ToolContext {
                        emb: emb.clone(),
                        store: store.clone(),
                        ingests: ingests.clone(),
                        deduplication_checker: deduplication_checker.clone(),
                        bus: bus.clone(),
                    };
                    // The dispatch table: each family claims its tool
                    // names in turn; a name no family claims is the
                    // unknown-tool fallthrough. (Before the Task 20
                    // split this was one match over every tool name,
                    // with the handlers defined below it in this file.)
                    let mut outcome = crate::handlers::memory::dispatch_memory_tool(
                        name,
                        &arguments,
                        &ctx,
                    )
                    .await;
                    if outcome.is_none() {
                        outcome = crate::handlers::tasks::dispatch_task_tool(
                            name,
                            &arguments,
                            &ctx,
                        )
                        .await;
                    }
                    if outcome.is_none() {
                        outcome = crate::handlers::guard::dispatch_guard_tool(
                            name,
                            &arguments,
                            &ctx,
                        )
                        .await;
                    }
                    if outcome.is_none() {
                        outcome = crate::handlers::system::dispatch_system_tool(
                            name,
                            &arguments,
                            &ctx,
                        )
                        .await;
                    }
                    if outcome.is_none() {
                        outcome = crate::handlers::mesh::dispatch_mesh_tool(
                            name,
                            &arguments,
                            &ctx,
                        )
                        .await;
                    }
                    match outcome {
                        Some(Ok(text)) => result_text = text,
                        Some(Err(msg)) => {
                            // A handler failure that must surface as a
                            // JSON-RPC error, never a successful result
                            // containing an error string (the channel
                            // get_snapshot encoding failures and refused
                            // Lock 2 task completions use).
                            error_response = Some(serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {
                                    "code": INTERNAL_ERROR,
                                    "message": msg
                                }
                            }));
                        }
                        None => {
                            result_text = format!("Unknown tool: {}", name);
                        }
                    }
                }
            }

            if let Some(err) = error_response {
                return err;
            }
            let result = serde_json::json!({
                "content": [
                    { "type": "text", "text": result_text }
                ]
            });
            serde_json::to_value(JsonRpcResponse::success(id.clone(), result)).unwrap_or_else(|e| {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32603,
                        "message": format!("Internal serialization error: {}", e)
                    }
                })
            })
        }
        _ => serde_json::json!({}),
    }
}

// Optional proxy helper to keep backwards compat with the stdio loop
/// How long a proxied call may run before the caller is told it failed.
///
/// A blanket 60 seconds used to sit here, which is shorter than the operation
/// that most needs the proxy: a full directory ingest of this repository takes
/// 58 to 86 seconds, so it was cut off every time. The bound is now generous
/// enough for real work and overridable, and -- unlike before -- reaching it
/// produces an answer rather than silence.
fn proxy_timeout() -> std::time::Duration {
    let secs = std::env::var("NEUROSTRATA_PROXY_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(1800);
    std::time::Duration::from_secs(secs)
}

/// How long the proxy will keep waiting for a daemon THIS PROCESS started before
/// it reports the daemon as unreachable. Generous because the first run of a new
/// install downloads the embedding model, which is minutes on a slow link.
/// Irrelevant when the daemon was already up: that case never waits.
fn daemon_startup_budget() -> std::time::Duration {
    let secs = std::env::var("NEUROSTRATA_DAEMON_STARTUP_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(600);
    std::time::Duration::from_secs(secs)
}

/// Whether to keep waiting rather than report the daemon unreachable.
///
/// Pure so it can be tested: the loop it drives needs a live socket, but the
/// decision is four facts and no I/O. Only a CONNECT failure is worth waiting
/// on -- a timeout means something answered and then took too long, and any
/// other error is not going to fix itself by being asked again.
fn should_wait_for_startup(
    daemon_has_answered: bool,
    is_connect_error: bool,
    waited: std::time::Duration,
    budget: std::time::Duration,
) -> bool {
    !daemon_has_answered && is_connect_error && waited < budget
}

/// Which of the two ways this proxy came to exist.
///
/// `neurostrata-mcp` with no arguments is a proxy either way, but not the same
/// proxy: one is talking to a daemon that was already answering, the other to a
/// daemon it has just spawned and that may still be loading its model. A connect
/// failure means opposite things in the two cases, and saying "it is not running"
/// about a daemon this process started thirty seconds ago is simply wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DaemonOrigin {
    /// A daemon answered /health before the proxy started.
    AlreadyRunning,
    /// This process spawned the daemon and did not wait for it to finish.
    SpawnedByUs,
}

/// Writes one JSON-RPC message and flushes it, so the caller sees it now rather
/// than whenever the buffer happens to fill.
async fn write_message(writer: &mut io::Stdout, message: &str) -> io::Result<()> {
    writer.write_all(message.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

/// The reply a failed request gets, or None when it must not get one.
///
/// JSON-RPC notifications carry no id and must never be answered, so a failure
/// on one is reported to stderr and goes no further. Everything else gets an
/// error response: a request read and then dropped leaves the client waiting on
/// an id that never comes back, which is a hang with no error reported
/// anywhere (bead neurostrata-oty).
fn failure_line(id: Option<Value>, code: i64, message: &str) -> Option<String> {
    // Notifications (no id) must never receive a response — that is the
    // intended suppression, not an error.
    id.as_ref()?;
    let response = JsonRpcResponse::<Value>::failure(id, code, message.to_string());
    match serde_json::to_string(&response) {
        Ok(line) => Some(line),
        Err(e) => {
            // Encoding failure must surface, not vanish via .ok().
            eprintln!(
                "CRITICAL: could not encode error response ({}): {}",
                message, e
            );
            None
        }
    }
}

async fn answer_with_error(
    writer: &mut io::Stdout,
    id: Option<Value>,
    code: i64,
    message: String,
) -> io::Result<()> {
    eprintln!("{}", message);
    match failure_line(id, code, &message) {
        Some(line) => write_message(writer, &line).await,
        None => Ok(()),
    }
}

/// The proxy's HTTP client. It carries the daemon bearer token (read
/// from the token file the daemon maintains) in its default headers:
/// without it, every forwarded /mcp call is a 401 against the daemon's
/// auth middleware -- the Task 1 F1 regression, in which the product's
/// primary client surface went dark the moment the daemon learned to
/// authenticate.
fn proxy_client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = crate::daemon::read_token_file("daemon.token") {
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }
    reqwest::Client::builder()
        // Separate from the overall bound on purpose. A port with nothing behind
        // it hangs rather than refusing on this machine -- the same behaviour
        // DaemonProbe in main.rs exists to describe -- so without this, a daemon
        // that is not running is indistinguishable from one that is merely slow,
        // and the wait is the whole timeout rather than a few seconds.
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(proxy_timeout())
        .build()
        .unwrap()
}

/// One forwarding attempt: POST the JSON-RPC request to the daemon's
/// /mcp endpoint with the proxy client. The startup-wait loop in
/// `start_mcp_proxy` retries this on connect errors while the daemon is
/// still starting.
async fn forward_mcp_request(
    client: &reqwest::Client,
    url: &str,
    request: &Value,
) -> reqwest::Result<reqwest::Response> {
    client.post(url).json(request).send().await
}

pub async fn start_mcp_proxy(origin: DaemonOrigin) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = stdout;

    let client = proxy_client();

    // Whether anything has ever answered on the port. Until something has, a
    // daemon we spawned ourselves is presumed to be still starting; after it
    // has, a connect failure means it went away, which is a different report.
    let mut daemon_has_answered = origin == DaemonOrigin::AlreadyRunning;

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        // Every path from here answers, or deliberately does not because the
        // message was a notification. Reading a request and then dropping it --
        // which is what this loop did whenever the daemon could not be reached,
        // or the line did not parse -- leaves the client waiting on an id that
        // never comes back: a hang with no error reported anywhere.
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) => {
                answer_with_error(
                    &mut writer,
                    Some(Value::Null),
                    PARSE_ERROR,
                    format!("The MCP request could not be parsed as JSON: {}", e),
                )
                .await?;
                continue;
            }
        };

        let id = request.get("id").cloned();

        // A daemon this process spawned is not ready the moment it is spawned,
        // and the first request usually arrives before it is. Waiting here is
        // the difference between a session that starts slowly and one that
        // reports every tool as broken.
        let waiting_since = std::time::Instant::now();
        let response = loop {
            let attempt =
                forward_mcp_request(&client, "http://127.0.0.1:34343/mcp", &request).await;

            let still_starting = should_wait_for_startup(
                daemon_has_answered,
                attempt
                    .as_ref()
                    .err()
                    .is_some_and(reqwest::Error::is_connect),
                waiting_since.elapsed(),
                daemon_startup_budget(),
            );

            if !still_starting {
                break attempt;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        };

        if response.is_ok() {
            daemon_has_answered = true;
        }

        match response {
            Ok(resp) => {
                let status = resp.status();
                match resp.text().await {
                    Ok(text) if status.is_success() => {
                        // A notification the daemon chose not to answer stays
                        // unanswered here too.
                        if !text.is_empty() {
                            write_message(&mut writer, &text).await?;
                        }
                    }
                    Ok(text) => {
                        answer_with_error(
                            &mut writer,
                            id,
                            INTERNAL_ERROR,
                            format!(
                                "The NeuroStrata daemon answered {}: {}",
                                status,
                                text.trim()
                            ),
                        )
                        .await?;
                    }
                    Err(e) => {
                        answer_with_error(
                            &mut writer,
                            id,
                            INTERNAL_ERROR,
                            format!("The NeuroStrata daemon's reply could not be read: {}", e),
                        )
                        .await?;
                    }
                }
            }
            Err(e) => {
                let cause = if e.is_connect() && !daemon_has_answered {
                    "this process started one, but it has not begun listening on                      127.0.0.1:34343 within the startup budget -- a first run                      downloads the embedding model, which can take several                      minutes; raise NEUROSTRATA_DAEMON_STARTUP_SECS, or run                      `neurostrata-mcp daemon` in a terminal to watch it start"
                } else if e.is_connect() {
                    "it is not running or is not listening on 127.0.0.1:34343"
                } else if e.is_timeout() {
                    "it did not answer in time; the work may still be running"
                } else {
                    "the request to it failed"
                };
                answer_with_error(
                    &mut writer,
                    id,
                    INTERNAL_ERROR,
                    format!(
                        "Could not reach the NeuroStrata daemon: {} ({}). FIRST: run `neurostrata-mcp status` -- do NOT start a daemon (exit 0 healthy, 1 down, 2 busy/finishing). One daemon serves every console; a forced second one risks the store.",
                        cause, e
                    ),
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// The path and line range a `locations` argument's first entry carries --
/// what the memory's `location`/`location_lines` columns hold (empty when the
/// list is empty).
pub(crate) fn first_location(locations: &[Value]) -> (String, String) {
    match locations.first() {
        Some(first) => (
            first
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string(),
            first
                .get("lines")
                .and_then(|l| l.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        None => (String::new(), String::new()),
    }
}

/// Adds the `refs` anchors and the GOVERNS declarations a `locations` argument
/// carries. Split out of handle_add_memory so task_complete's inline
/// extraction lands on the exact same anchoring path (section 3.2).
pub(crate) fn stamp_locations(metadata: &mut Value, locations: &[Value]) {
    if locations.is_empty() {
        return;
    }
    let obj = match metadata.as_object_mut() {
        Some(obj) => obj,
        None => return,
    };
    let refs: Vec<serde_json::Value> = locations
        .iter()
        .map(|loc| {
            let mut ref_obj = serde_json::Map::new();
            if let Some(path) = loc.get("path").and_then(|p| p.as_str()) {
                ref_obj.insert("file".to_string(), serde_json::json!(path));
            }
            if let Some(lines) = loc.get("lines").and_then(|l| l.as_str()) {
                ref_obj.insert("lines".to_string(), serde_json::json!(lines));
            }
            if let Some(sym) = loc.get("symbol").and_then(|s| s.as_str()) {
                ref_obj.insert("symbol".to_string(), serde_json::json!(sym));
            }
            if let Some(context) = loc.get("source_context").filter(|v| v.is_object()) {
                ref_obj.insert("source_context".to_string(), context.clone());
            }
            serde_json::Value::Object(ref_obj)
        })
        .collect();
    obj.insert("refs".to_string(), serde_json::Value::Array(refs));

    // A rule that names files governs them. This is the edge that makes an
    // architectural memory reachable from the code it constrains, rather
    // than only from a similar-sounding query.
    let governed: Vec<serde_json::Value> = locations
        .iter()
        .filter_map(|loc| loc.get("path").and_then(|p| p.as_str()))
        .filter(|p| !p.is_empty())
        // Same normalisation the ingester applies to its node ids, so a
        // path written by hand lands on the file node it names.
        .map(crate::parser::ingest::normalize_node_path)
        .fold(Vec::new(), |mut acc: Vec<String>, path| {
            // Several locations often sit in one file; one edge is enough.
            if !acc.contains(&path) {
                acc.push(path);
            }
            acc
        })
        .into_iter()
        .map(|p| serde_json::json!(p))
        .collect();
    if !governed.is_empty() {
        obj.insert("governs".to_string(), serde_json::Value::Array(governed));
    }
}


/// The lineage stamps every newly written row carries: born now, seen zero
/// times. Shared with task_complete's inline extraction so a fact written
/// there is indistinguishable from one written by add_memory.
pub(crate) fn stamp_new_memory(metadata: &mut Value) {
    if let Some(meta_obj) = metadata.as_object_mut() {
        meta_obj.insert(
            "valid_from".to_string(),
            serde_json::json!(chrono::Utc::now().timestamp()),
        );
        meta_obj.insert("access_count".to_string(), serde_json::json!(0));
    }
}

/// The write half of add_memory: init, embed, dedup check, upsert.
/// task_complete's inline extraction runs this same pipeline (section 3.2,
/// path 3) and the error strings stay identical across both callers.
pub(crate) async fn embed_dedup_and_upsert(
    store: &Arc<dyn VectorStore>,
    emb: &Arc<dyn Embedder>,
    namespace: &str,
    payload: &MemoryPayload,
    dedup: Option<&Arc<crate::judgment::DeduplicationChecker>>,
) -> Result<crate::judgment::policy::MemoryWriteReport, String> {
    use crate::judgment::canonical::canonical_duplicate_key;
    use crate::judgment::policy::{DedupDecision, DedupHint, MemoryWriteReport};
    use crate::traits::{DedupCandidateScope, RawDistanceQuery};

    store
        .init(namespace)
        .await
        .map_err(|_| "Failed to initialize table.".to_string())?;
    let vector = emb
        .embed(&payload.content)
        .await
        .map_err(|_| "Failed to generate embedding.".to_string())?;

    let incoming_key = canonical_duplicate_key(namespace, payload)
        .map_err(|e| format!("Failed to compute canonical key: {}", e))?;

    // Determine scope.
    let scope = if payload.memory_type == "procedure" {
        let trigger = payload
            .metadata
            .get("trigger")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        DedupCandidateScope::Procedure { trigger }
    } else {
        DedupCandidateScope::Domain
    };

    let incoming_is_attempt = crate::attempt::is_failed_attempt(&payload.metadata);

    let mut decision = DedupDecision::Allowed;
    let mut dedup_hint = None;
    let mut linked_existing_id = None;

    // Exact-content candidates (independent of vector rank).
    if let Ok(exact_candidates) = store
        .find_active_exact_content(namespace, &payload.content, scope.clone())
        .await
    {
        for ec in exact_candidates {
            if let Ok(cand_key) = canonical_duplicate_key(namespace, &ec.payload) {
                if cand_key == incoming_key {
                    let has_extracted = ec
                        .payload
                        .metadata
                        .get("extracted_from")
                        .and_then(|v| v.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false);
                    // An incoming memory that itself declares an extraction
                    // (e.g. task_complete's inline lesson, or an add_memory
                    // carrying extracted_from) is a link, not a duplicate: the
                    // lesson still needs to land on the existing row. Blocking
                    // it here would strand the task in Lock 2 (the task_complete
                    // dedup trap). Only a bare content duplicate with no
                    // extraction on either side is a true BlockedExact.
                    let incoming_ids: Vec<String> = payload
                        .metadata
                        .get("extracted_from")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let incoming_has_extracted = !incoming_ids.is_empty();
                    if has_extracted || incoming_has_extracted {
                        linked_existing_id = Some(ec.id.clone());
                        decision = DedupDecision::LinkedExisting;
                        // Update existing candidate's metadata idempotently:
                        // merge extracted_from ids, reuse its vector, no twin row.
                        if let Ok(Some((existing_vec, mut existing_payload))) =
                            store.get(namespace, &ec.id).await
                        {
                            let meta = existing_payload.metadata.as_object_mut();
                            if let Some(m) = meta {
                                let arr = m
                                    .entry("extracted_from".to_string())
                                    .or_insert_with(|| serde_json::json!([]));
                                if let Some(a) = arr.as_array_mut() {
                                    for tid in &incoming_ids {
                                        if !a.iter().any(|x| x.as_str() == Some(tid)) {
                                            a.push(serde_json::json!(tid));
                                        }
                                    }
                                }
                            }
                            // Also carry other metadata? Keep existing; idempotent.
                            let _ = store
                                .upsert(namespace, &ec.id, existing_vec, existing_payload)
                                .await;
                        }
                    } else {
                        decision = DedupDecision::BlockedExact;
                        // Surface the candidate so callers (e.g. task_complete)
                        // can fall back to link_memory_id instead of failing
                        // opaquely on Lock 2.
                        if linked_existing_id.is_none() {
                            linked_existing_id = Some(ec.id.clone());
                        }
                    }
                    // No new row for exact reuse / bare exact duplicate.
                    return Ok(MemoryWriteReport {
                        upsert: None,
                        decision,
                        dedup_hint,
                        linked_existing_id,
                    });
                }
            }
        }
    }

    // Raw-distance candidates (model judgment path).
    if let Some(checker) = dedup {
        let limit = checker.config.max_candidates.max(1);
        if let Ok(raw_candidates) = store
            .nearest_by_raw_distance(
                namespace,
                vector.clone(),
                RawDistanceQuery {
                    limit,
                    scope: scope.clone(),
                },
            )
            .await
        {
            // Attempt partition + registry filter (registry already at store seam).
            let filtered: Vec<_> = raw_candidates
                .into_iter()
                .filter(|c| {
                    crate::attempt::is_failed_attempt(&c.payload.metadata) == incoming_is_attempt
                })
                .collect();

            if !filtered.is_empty() {
                // Convert to SearchResult for the judgment provider.
                let search_results: Vec<crate::traits::SearchResult> = filtered
                    .iter()
                    .map(|c| crate::traits::SearchResult {
                        id: c.id.clone(),
                        score: c.raw_distance,
                        payload: c.payload.clone(),
                        evidence: None,
                    })
                    .collect();

                if let Some(duplicates) = checker.check_duplicates(payload, &search_results).await {
                    if !duplicates.is_empty() {
                        let dup = &duplicates[0];
                        let raw_dist = filtered
                            .iter()
                            .find(|c| c.id == dup.existing_memory_id)
                            .map(|c| c.raw_distance)
                            .unwrap_or(0.0);
                        decision = DedupDecision::AllowedWithHint;
                        dedup_hint = Some(DedupHint {
                            candidate_id: dup.existing_memory_id.clone(),
                            raw_distance: raw_dist,
                            confidence: Some(dup.confidence),
                        });
                    }
                }
            }
        }
    }

    // Write unless blocked.
    if matches!(decision, DedupDecision::BlockedExact | DedupDecision::BlockedModel) {
        return Ok(MemoryWriteReport {
            upsert: None,
            decision,
            dedup_hint,
            linked_existing_id,
        });
    }

    let new_id = uuid::Uuid::new_v4().to_string();
    let report = store
        .upsert(namespace, &new_id, vector, payload.clone())
        .await
        .map_err(|_| "Failed to store memory in database.".to_string())?;
    Ok(MemoryWriteReport {
        upsert: Some(report),
        decision,
        dedup_hint,
        linked_existing_id,
    })
}

// A memory is durable only once checkpointed -- WAL replay restores the catalog
// but not row insertions (bead neurostrata-kug) -- and this surface used to
// checkpoint after every write to close that window. It cost more than it
// bought: the engine waits for all transactions to drain before flushing, so
// under load the checkpoint blocked the writer for minutes and failed anyway
// (bead neurostrata-3fi.6.4). The store now marks itself dirty and the daemon's
// background task does the flushing, which is also what runs at shutdown.

/// Resolves a namespace against the ones that already exist, ignoring case.
///
/// A project's identity should not depend on how somebody spelled the folder
/// when they cloned it. This machine's checkout is .../neurostrata while the
/// repository is NeuroStrata; the GUI derives the namespace from the folder
/// name and agents pass the project name, so one project quietly became two
/// strata (bead neurostrata-fld). Windows and macOS being case-insensitive
/// means the same directory can be addressed several ways besides.
///
/// An existing namespace wins on a case-insensitive match and keeps the
/// spelling it was stored with; a genuinely new name is left exactly as the
/// caller wrote it. Listing namespaces costs about two milliseconds, so this
/// runs on every call rather than being cached and going stale.
pub(crate) async fn resolve_namespace(store: &Arc<dyn VectorStore>, requested: &str) -> String {
    match store.list_namespaces().await {
        Ok(existing) => {
            // An exact name always wins. Only when nothing matches exactly does
            // case decide, so a caller naming a namespace that really exists is
            // never redirected to a differently-cased neighbour.
            if existing.iter().any(|known| known == requested) {
                return requested.to_string();
            }
            let mut candidates: Vec<String> = existing
                .into_iter()
                .filter(|known| known.eq_ignore_ascii_case(requested))
                .collect();

            match candidates.len() {
                0 => requested.to_string(),
                1 => candidates.remove(0),
                _ => {
                    // Two spellings already exist. Row order is not a decision,
                    // so take the fuller namespace and say what was passed over.
                    candidates.sort();
                    let mut best = candidates[0].clone();
                    let mut best_len = 0usize;
                    for candidate in &candidates {
                        let held = store
                            .list(candidate, None, None)
                            .await
                            .map(|m| m.len())
                            .unwrap_or(0);
                        if held > best_len {
                            best_len = held;
                            best = candidate.clone();
                        }
                    }
                    eprintln!(
                        "WARNING: '{}' matches {:?}; using '{}', which holds the most memories. Merge them with: neurostrata-mcp move <from> <id> <to>",
                        requested, candidates, best
                    );
                    best
                }
            }
        }
        // A namespace that cannot be verified is used as given: refusing the
        // write would be worse than writing it where the caller asked.
        Err(_) => requested.to_string(),
    }
}

/// What a metadata update did.
#[derive(Debug, PartialEq)]
pub(crate) enum MetadataOutcome {
    Updated,
    NotFound,
    /// Written by directory ingestion, so the next ingest would overwrite the
    /// update. The file is what to change.
    Ingested,
}

/// Update metadata on an existing memory without changing its ID or content.
///
/// This is the operator's surgical path for fixing metadata issues (like missing
/// provenance) without creating a new memory ID or breaking existing references.
pub(crate) async fn set_metadata(
    store: &dyn VectorStore,
    namespace: &str,
    id: &str,
    new_metadata: serde_json::Value,
) -> anyhow::Result<MetadataOutcome> {
    let (vector, mut payload) = match store.get(namespace, id).await? {
        Some(found) => found,
        None => return Ok(MetadataOutcome::NotFound),
    };
    if payload.user_id == "auto-ingestor" || id.contains(crate::parser::ingest::NAMESPACE_SEPARATOR)
    {
        return Ok(MetadataOutcome::Ingested);
    }

    payload.metadata = new_metadata;
    store.upsert(namespace, id, vector, payload).await?;
    Ok(MetadataOutcome::Updated)
}

/// What an operator's edit did.
#[derive(Debug, PartialEq)]
pub(crate) enum EditOutcome {
    Edited,
    NotFound,
    /// Written by directory ingestion, so the next ingest would overwrite the
    /// edit. The file is what to change.
    Ingested,
}

/// The operator's in-place edit, shared by the CLI and the daemon's /edit.
///
/// It rewrites the text where the memory is, then moves the row with relocate
/// if the namespace changes, and never deletes anything. Both earlier orders
/// lost memories: deleting first lost one whenever the write that followed
/// failed, and writing to the new namespace first found the old row by id --
/// upsert never rewrites a namespace -- so the delete that followed removed
/// the only copy.
pub(crate) async fn edit_memory(
    store: &dyn VectorStore,
    embedder: &dyn Embedder,
    old_namespace: &str,
    id: &str,
    new_namespace: &str,
    content: &str,
    location: &str,
) -> anyhow::Result<EditOutcome> {
    // Secret scan on the manual edit path: an edit is as permanent as an add.
    if let Some(rejection) =
        crate::secrets::scan_entry_point(content, &serde_json::json!({}), "edit_memory")
    {
        return Err(anyhow::anyhow!("{}", rejection));
    }

    let (vector, mut payload) = match store.get(old_namespace, id).await? {
        Some(found) => found,
        None => return Ok(EditOutcome::NotFound),
    };
    if payload.user_id == "auto-ingestor" || id.contains(crate::parser::ingest::NAMESPACE_SEPARATOR)
    {
        return Ok(EditOutcome::Ingested);
    }

    // Re-embed whenever the text changes. Reusing the old vector left the row
    // ranked by wording it no longer contained, so correcting a wrong rule kept
    // the wrong rule findable and hid the correction (bead neurostrata-vbj).
    let vector = if payload.content == content {
        vector
    } else {
        embedder.embed(content).await?
    };
    payload.content = content.to_string();
    payload.location = location.to_string();

    store.upsert(old_namespace, id, vector, payload).await?;
    if old_namespace != new_namespace {
        store.relocate(id, old_namespace, new_namespace).await?;
    }
    Ok(EditOutcome::Edited)
}


#[cfg(test)]
mod startup_tests {
    use super::{should_wait_for_startup, DaemonOrigin};
    use std::time::Duration;

    const BUDGET: Duration = Duration::from_secs(600);

    /// The bug this exists for: `neurostrata-mcp` with no daemon spawns one and
    /// starts proxying without waiting for it. Reporting the first request as
    /// "the daemon is not running" describes a daemon this process just started.
    #[test]
    fn a_daemon_we_started_is_waited_for_rather_than_declared_absent() {
        assert!(should_wait_for_startup(
            false,
            true,
            Duration::from_secs(45),
            BUDGET
        ));
    }

    /// A daemon that was already answering is not starting up, so a refusal is
    /// the truth and the caller should hear it now.
    #[test]
    fn a_daemon_that_was_already_up_is_never_waited_for() {
        assert_eq!(DaemonOrigin::AlreadyRunning, DaemonOrigin::AlreadyRunning);
        assert!(!should_wait_for_startup(
            true,
            true,
            Duration::from_secs(1),
            BUDGET
        ));
    }

    /// Once something has answered, a later refusal means the daemon went away.
    /// Waiting on that would turn a real failure into a hang, which is the
    /// behaviour this whole file exists to remove.
    #[test]
    fn a_daemon_that_answered_and_then_vanished_is_reported_not_awaited() {
        assert!(!should_wait_for_startup(
            true,
            true,
            Duration::from_secs(0),
            BUDGET
        ));
    }

    /// The budget is a bound, not a suggestion: past it the caller gets an
    /// answer, because an unanswered request is the failure mode being fixed.
    #[test]
    fn waiting_stops_at_the_budget() {
        assert!(!should_wait_for_startup(
            false,
            true,
            Duration::from_secs(601),
            BUDGET
        ));
    }

    /// Only a connect failure is a daemon that has not opened its port yet. A
    /// timeout means something answered and then took too long; retrying it
    /// would re-run work that may still be running.
    #[test]
    fn only_a_connect_failure_is_worth_waiting_on() {
        assert!(!should_wait_for_startup(
            false,
            false,
            Duration::from_secs(1),
            BUDGET
        ));
    }
}

#[cfg(test)]
mod migration_notice_tests {
    use crate::handlers::memory::unmigrated_ids_notice;
    use crate::traits::{MemoryPayload, SearchResult};

    fn node(id: &str, user_id: &str) -> SearchResult {
        SearchResult {
            id: id.to_string(),
            score: 0.0,
            payload: MemoryPayload {
                content: String::new(),
                location: String::new(),
                location_lines: String::new(),
                memory_type: "file".to_string(),
                metadata: serde_json::json!({}),
                user_id: user_id.to_string(),
                agent_name: None,
            },
            evidence: None,
        }
    }

    #[test]
    fn a_migrated_namespace_says_nothing() {
        let memories = vec![
            node("NeuroStrata::src", "auto-ingestor"),
            node("NeuroStrata::src/main.rs", "auto-ingestor"),
        ];
        assert!(unmigrated_ids_notice("NeuroStrata", &memories).is_none());
    }

    /// Hand-written memories are keyed by UUID and always will be. Counting
    /// them as unmigrated would make the notice permanent and therefore
    /// ignorable, which is worse than not having it.
    #[test]
    fn hand_written_memories_are_not_counted() {
        let memories = vec![
            node("550e8400-e29b-41d4-a716-446655440000", "anthomus"),
            node("NeuroStrata::src", "auto-ingestor"),
        ];
        assert!(unmigrated_ids_notice("NeuroStrata", &memories).is_none());
    }

    #[test]
    fn an_unmigrated_namespace_says_how_many_and_what_to_run() {
        let memories = vec![
            node("src", "auto-ingestor"),
            node("README.md", "auto-ingestor"),
            node("NeuroStrata::src/main.rs", "auto-ingestor"),
        ];
        let notice = unmigrated_ids_notice("NeuroStrata", &memories)
            .expect("two of the three ids predate qualification");
        assert!(notice.contains("2 of 3"), "{}", notice);
        assert!(notice.contains("neurostrata-mcp backup"), "{}", notice);
        assert!(notice.contains("neurostrata-mcp ingest"), "{}", notice);
        // The agent must hand this to the user, not attempt it.
        assert!(notice.contains("MIGRATION NEEDED"), "{}", notice);
    }

    #[test]
    fn a_namespace_with_nothing_ingested_says_nothing() {
        assert!(unmigrated_ids_notice("NeuroStrata", &[]).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{RecursionToken, ThalamicBus, ThalamicPulse};
    use crate::handlers::guard::handle_guard_validate;
    use crate::handlers::memory::{
        format_memory, governance_conflict_note, handle_add_memory, handle_get_memory,
        handle_get_snapshot, handle_search_memory, handle_supersede_memory,
        render_evidence_suffix, render_path, render_search_results,
    };
    use crate::guard::testkit::MockStore;

    /// Task 1 (F1/FM-2 regression): the stdio proxy's client must carry
    /// the daemon bearer token, or every proxied MCP session is a 401
    /// against the token-enforcing daemon -- the proxy is the product's
    /// primary client surface. A recording stub stands in for the daemon
    /// and captures the Authorization header of the forwarded request.
    #[tokio::test(flavor = "multi_thread")]
    async fn proxy_forwards_the_daemon_bearer_token() {
        use std::io::{BufRead, Read, Write};

        // The token the daemon would have created and the proxy must read.
        let expected = crate::daemon::load_or_create_daemon_token();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stub = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let mut auth = String::new();
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                let lower = trimmed.to_ascii_lowercase();
                if lower.starts_with("authorization:") {
                    // The value is compared exactly, so take it from the
                    // original line -- only the name match is folded.
                    auth = trimmed["authorization:".len()..].trim().to_string();
                }
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
            if content_length > 0 {
                let mut body = vec![0u8; content_length];
                reader.read_exact(&mut body).unwrap();
            }
            let response_body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}";
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            reader.get_mut().write_all(response.as_bytes()).unwrap();
            auth
        });

        let client = proxy_client();
        let request = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        let url = format!("http://{addr}/mcp");
        let resp = forward_mcp_request(&client, &url, &request).await.unwrap();
        assert!(resp.status().is_success());
        let auth = stub.join().unwrap();
        assert_eq!(
            auth,
            format!("Bearer {expected}"),
            "the proxy must forward the daemon bearer token"
        );
    }

    #[test]
    fn locations_preserve_opaque_source_context_without_interpreting_it() {
        let mut metadata = serde_json::json!({});
        let context = serde_json::json!({
            "provider": "git",
            "revision_scope": "branch:feature-x",
            "worktree_id": "opaque-worktree"
        });
        stamp_locations(
            &mut metadata,
            &[serde_json::json!({
                "path": "src/lib.rs",
                "source_context": context
            })],
        );

        assert_eq!(metadata["refs"][0]["file"], "src/lib.rs");
        assert_eq!(metadata["refs"][0]["source_context"]["provider"], "git");
        assert_eq!(
            metadata["refs"][0]["source_context"]["revision_scope"],
            "branch:feature-x"
        );
        assert_eq!(metadata["governs"][0], "src/lib.rs");
    }

    /// A request the proxy could not forward is answered, not dropped. Dropping
    /// it left the client waiting on an id that never came back: a hang with no
    /// error anywhere.
    #[test]
    fn a_request_that_cannot_be_forwarded_is_answered() {
        let line = failure_line(
            Some(serde_json::json!(7)),
            INTERNAL_ERROR,
            "daemon unreachable",
        )
        .expect("a request carrying an id gets a reply");
        let parsed: Value = serde_json::from_str(&line).expect("valid JSON-RPC");

        assert_eq!(parsed["id"], serde_json::json!(7));
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["error"]["code"], INTERNAL_ERROR);
        assert_eq!(parsed["error"]["message"], "daemon unreachable");
    }

    /// A response carries a result or an error, never both, and never a null
    /// result beside an error.
    #[test]
    fn an_error_response_carries_no_result() {
        let line = failure_line(Some(serde_json::json!(1)), PARSE_ERROR, "not JSON").unwrap();
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert!(parsed.get("result").is_none(), "got {}", parsed);
    }

    /// Notifications have no id and must never be answered, however badly they
    /// fail.
    #[test]
    fn a_notification_is_never_answered() {
        assert!(failure_line(None, INTERNAL_ERROR, "daemon unreachable").is_none());
    }

    /// 60 seconds used to be the bound, and a directory ingest of this
    /// repository takes 58 to 86 -- so the call that most needs the proxy was
    /// the one it cut off.
    #[test]
    fn the_proxy_bound_outlasts_a_directory_ingest() {
        if std::env::var("NEUROSTRATA_PROXY_TIMEOUT_SECS").is_ok() {
            return;
        }
        assert!(proxy_timeout() > std::time::Duration::from_secs(300));
    }

    fn payload(content: &str) -> MemoryPayload {
        MemoryPayload {
            content: content.to_string(),
            user_id: "tester".to_string(),
            memory_type: "rule".to_string(),
            agent_name: None,
            location: String::new(),
            location_lines: String::new(),
            metadata: serde_json::json!({}),
        }
    }

    fn namespace_from(existing: &[&str], requested: &str) -> String {
        // The same rule resolve_namespace applies, over a list instead of the store.
        if existing.iter().any(|known| *known == requested) {
            return requested.to_string();
        }
        existing
            .iter()
            .find(|known| known.eq_ignore_ascii_case(requested))
            .map(|known| known.to_string())
            .unwrap_or_else(|| requested.to_string())
    }

    #[test]
    fn a_differently_cased_checkout_lands_in_the_existing_namespace() {
        let existing = ["NeuroStrata", "NeuroPlasticity"];
        assert_eq!(namespace_from(&existing, "neurostrata"), "NeuroStrata");
        assert_eq!(namespace_from(&existing, "NEUROSTRATA"), "NeuroStrata");
    }

    #[test]
    fn an_exact_name_is_never_redirected() {
        // Both spellings exist here, which is the mess this repairs. A caller
        // asking for the one that exists must still reach it.
        let existing = ["NeuroStrata", "neurostrata"];
        assert_eq!(namespace_from(&existing, "neurostrata"), "neurostrata");
        assert_eq!(namespace_from(&existing, "NeuroStrata"), "NeuroStrata");
    }

    #[test]
    fn a_genuinely_new_namespace_keeps_the_spelling_it_was_given() {
        let existing = ["NeuroStrata"];
        assert_eq!(namespace_from(&existing, "SomethingElse"), "SomethingElse");
    }

    #[test]
    fn a_formatted_memory_carries_its_anchors() {
        let mut p = payload("checkpoint every write");
        p.location = "src/store/ladybug.rs".to_string();
        p.location_lines = "428-440".to_string();
        p.metadata = serde_json::json!({ "governs": ["src/daemon.rs"] });

        let out = format_memory("abc-123", &p);

        assert!(out.contains("--- Memory ID: abc-123 ---"));
        assert!(out.contains("Content: checkpoint every write"));
        assert!(out.contains("File Location: src/store/ladybug.rs (Lines: 428-440)"));
        assert!(out.contains("Governs: [\"src/daemon.rs\"]"));
    }

    #[test]
    fn every_stored_anchor_is_rendered_with_its_lines_and_symbol() {
        let mut p = payload("parse before storing");
        p.metadata = serde_json::json!({ "refs": [
            { "file": "src/parser/ingest.rs", "lines": "133-160", "symbol": "ingest_directory" },
            { "file": "src/store/ladybug.rs" }
        ]});

        let out = format_memory("abc-123", &p);

        assert!(
            out.contains("src/parser/ingest.rs:133-160 (ingest_directory)"),
            "{}",
            out
        );
        assert!(out.contains("src/store/ladybug.rs"), "{}", out);
    }

    #[test]
    fn an_empty_anchor_list_is_not_rendered() {
        let mut p = payload("a rule with no edges");
        p.metadata = serde_json::json!({ "governs": [], "related_to": [] });

        let out = format_memory("abc-123", &p);

        assert!(!out.contains("Governs"));
        assert!(!out.contains("Related Nodes"));
    }

    #[test]
    fn a_failed_attempt_renders_the_attempt_line() {
        let mut p = payload("tried X");
        p.memory_type = "fact".to_string();
        p.metadata = serde_json::json!({
            "attempt": {
                "approach": "retry with backoff",
                "outcome": "failed",
                "failure_mode": "timeout",
                "context": "under load"
            }
        });

        let out = format_memory("abc-123", &p);

        assert!(
            out.contains("Attempt [FAILED in under load]: tried 'retry with backoff' -> failed with 'timeout' (single episode, not a global rule)"),
            "{}",
            out
        );
    }

    #[test]
    fn a_malformed_attempt_renders_no_attempt_line() {
        let mut p = payload("tried X");
        p.memory_type = "fact".to_string();
        p.metadata = serde_json::json!({
            "attempt": {
                "approach": "retry with backoff",
                "outcome": "failed",
                "failure_mode": "timeout"
            }
        });

        let out = format_memory("abc-123", &p);

        assert!(!out.contains("Attempt [FAILED in"), "{}", out);
    }

    struct RecordingProvider {
        calls: std::sync::Arc<std::sync::Mutex<usize>>,
        last_candidate_count: std::sync::Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl crate::judgment::JudgmentProvider for RecordingProvider {
        async fn judge_duplicates(
            &self,
            _new_memory: &crate::traits::MemoryPayload,
            candidates: &[crate::traits::SearchResult],
        ) -> anyhow::Result<Vec<crate::judgment::DuplicateJudgment>> {
            *self.calls.lock().unwrap() += 1;
            *self.last_candidate_count.lock().unwrap() = candidates.len();
            Ok(vec![])
        }
        async fn health_check(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        fn name(&self) -> &str {
            "recording"
        }
    }

    #[tokio::test]
    async fn a_negative_exemplar_never_blocks_a_positive_fact() {
        use crate::guard::testkit::MockStore;
        use std::sync::Arc;

        // Incoming: positive fact (no attempt). Candidate: negative exemplar (valid attempt).
        // Partition must drop the candidate, so the provider is never called.
        {
            let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
            let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
            store.init("probe").await.unwrap();

            // seed a negative exemplar candidate
            let neg_payload = payload("same subject");
            let mut neg_payload = neg_payload;
            neg_payload.memory_type = "fact".to_string();
            neg_payload.metadata = serde_json::json!({
                "attempt": {
                    "approach": "retry",
                    "outcome": "failed",
                    "failure_mode": "timeout",
                    "context": "ctx"
                }
            });
            store
                .upsert("probe", "neg-1", vec![0.0; 4], neg_payload)
                .await
                .unwrap();

            let calls = Arc::new(std::sync::Mutex::new(0usize));
            let last = Arc::new(std::sync::Mutex::new(0usize));
            let provider: Arc<dyn crate::judgment::JudgmentProvider> =
                Arc::new(RecordingProvider {
                    calls: calls.clone(),
                    last_candidate_count: last.clone(),
                });
            let mut cfg = crate::config::DeduplicationConfig {
                provider: crate::config::DedupProviderMode::Jev,
                judgment_api_key_env: None,
                judgment_api_key: Some("test".to_string()),
                timeout_seconds: 5,
                similarity_threshold: 0.85,
                confidence_threshold: 0.5,
                max_candidates: 5,
                cb_failure_threshold: 3,
                cb_backoff_initial_seconds: 1,
                cb_backoff_max_seconds: 60,
            };
            let _ = &mut cfg;
            let checker = Arc::new(crate::judgment::DeduplicationChecker::new_with_provider(cfg, provider));

            let incoming = payload("same subject");
            let mut incoming = incoming;
            incoming.memory_type = "fact".to_string();
            // no attempt

            let res = embed_dedup_and_upsert(
                &store,
                &emb,
                "probe",
                &incoming,
                Some(&checker),
            )
            .await;
            assert!(res.is_ok(), "write should succeed: {:?}", res);
            assert_eq!(
                *calls.lock().unwrap(),
                0,
                "provider must not be called when candidate is a negative exemplar and incoming is positive"
            );
        }

        // Incoming: negative exemplar. Candidate: positive fact. Partition drops it.
        {
            let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
            let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
            store.init("probe").await.unwrap();

            let pos_payload = payload("same subject");
            let mut pos_payload = pos_payload;
            pos_payload.memory_type = "fact".to_string();
            store
                .upsert("probe", "pos-1", vec![0.0; 4], pos_payload)
                .await
                .unwrap();

            let calls = Arc::new(std::sync::Mutex::new(0usize));
            let last = Arc::new(std::sync::Mutex::new(0usize));
            let provider: Arc<dyn crate::judgment::JudgmentProvider> =
                Arc::new(RecordingProvider {
                    calls: calls.clone(),
                    last_candidate_count: last.clone(),
                });
            let cfg = crate::config::DeduplicationConfig {
                provider: crate::config::DedupProviderMode::Auto,
                judgment_api_key_env: None,
                judgment_api_key: Some("test".to_string()),
                timeout_seconds: 5,
                similarity_threshold: 0.85,
                confidence_threshold: 0.5,
                max_candidates: 5,
                cb_failure_threshold: 3,
                cb_backoff_initial_seconds: 1,
                cb_backoff_max_seconds: 60,
            };
            let checker = Arc::new(crate::judgment::DeduplicationChecker::new_with_provider(cfg, provider));

            let mut incoming = payload("same subject");
            incoming.memory_type = "fact".to_string();
            incoming.metadata = serde_json::json!({
                "attempt": {
                    "approach": "retry",
                    "outcome": "failed",
                    "failure_mode": "timeout",
                    "context": "ctx"
                }
            });

            let res = embed_dedup_and_upsert(
                &store,
                &emb,
                "probe",
                &incoming,
                Some(&checker),
            )
            .await;
            assert!(res.is_ok(), "write should succeed: {:?}", res);
            assert_eq!(
                *calls.lock().unwrap(),
                0,
                "provider must not be called when incoming is a negative exemplar and candidate is positive"
            );
        }

        // Same-attempt candidates are still handed to the provider.
        {
            let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
            let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
            store.init("probe").await.unwrap();

            let mut cand = payload("same subject");
            cand.memory_type = "fact".to_string();
            cand.metadata = serde_json::json!({
                "attempt": {
                    "approach": "retry",
                    "outcome": "failed",
                    "failure_mode": "timeout",
                    "context": "ctx"
                }
            });
            store
                .upsert("probe", "att-1", vec![0.0; 4], cand)
                .await
                .unwrap();

            let calls = Arc::new(std::sync::Mutex::new(0usize));
            let last = Arc::new(std::sync::Mutex::new(0usize));
            let provider: Arc<dyn crate::judgment::JudgmentProvider> =
                Arc::new(RecordingProvider {
                    calls: calls.clone(),
                    last_candidate_count: last.clone(),
                });
            let cfg = crate::config::DeduplicationConfig {
                provider: crate::config::DedupProviderMode::Auto,
                judgment_api_key_env: None,
                judgment_api_key: Some("test".to_string()),
                timeout_seconds: 5,
                similarity_threshold: 0.85,
                confidence_threshold: 0.5,
                max_candidates: 5,
                cb_failure_threshold: 3,
                cb_backoff_initial_seconds: 1,
                cb_backoff_max_seconds: 60,
            };
            let checker = Arc::new(crate::judgment::DeduplicationChecker::new_with_provider(cfg, provider));

            let mut incoming = payload("same subject");
            incoming.memory_type = "fact".to_string();
            incoming.metadata = serde_json::json!({
                "attempt": {
                    "approach": "retry2",
                    "outcome": "failed",
                    "failure_mode": "timeout2",
                    "context": "ctx2"
                }
            });

            let res = embed_dedup_and_upsert(
                &store,
                &emb,
                "probe",
                &incoming,
                Some(&checker),
            )
            .await;
            assert!(res.is_ok(), "write should succeed: {:?}", res);
            assert_eq!(
                *calls.lock().unwrap(),
                1,
                "provider must be called for same-attempt candidates"
            );
            assert_eq!(*last.lock().unwrap(), 1, "the candidate must be handed over");
        }
    }

    /// Embeds deterministically from the text, so two different strings can
    /// never collide. That is the whole point of the assertions below.
    struct StubEmbedder;

    #[async_trait::async_trait]
    impl Embedder for StubEmbedder {
        async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
            let mut v = vec![0.0f32; 4];
            for (i, b) in text.bytes().enumerate() {
                v[i % 4] += b as f32;
            }
            // LadybugDB's `ARRAY_DISTANCE` rejects whole-number float arrays
            // as a binder error: the engine parses the literal as INTEGER[] when
            // every element is an integer, and the array-distance signature is
            // FLOAT[]/DOUBLE[]. Real embedders never produce integers, so
            // production is unaffected; the stub was a foot-gun only test
            // code could step on. A constant per-element fraction makes the vector
            // a non-integer FLOAT[] deterministically, and the offset is the
            // same for the row written and the query read so the round-trip
            // and the equality assertions still hold.
            for (i, x) in v.iter_mut().enumerate() {
                *x += (i as f32 + 1.0) * 0.001;
            }
            Ok(v)
        }
        fn dimensions(&self) -> usize {
            4
        }
    }

    /// The stub's vector is accepted by `ARRAY_DISTANCE`: at least one element
    /// carries a fractional part, so LadybugDB parses the literal as
    /// `FLOAT[]` rather than `INTEGER[]`. The pre-fix stub summed to integer
    /// values and broke the search path silently.
    #[test]
    fn stub_embedder_outputs_a_non_integer_float_vector() {
        let v = futures::executor::block_on(StubEmbedder.embed("podman")).unwrap();
        assert!(
            v.iter().any(|x| x.fract() != 0.0),
            "at least one element must be a non-integer float for ARRAY_DISTANCE: {v:?}"
        );
    }

    /// A round-trip through the store preserves the fractional part, so a
    /// vector-comparison assertion across the kept-prefix path keeps working
    /// after the offset is added.
    #[tokio::test]
    async fn stub_embedder_vector_round_trips_through_the_store() {
        use crate::traits::VectorStore;
        let dir = std::env::temp_dir().join(format!(
            "ns-stub-roundtrip-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store: Arc<dyn VectorStore> =
            Arc::new(crate::store::LadybugStore::for_testing(&dir, 4).expect("open temp"));
        store.init("probe").await.expect("init");
        let id = uuid::Uuid::new_v4().to_string();
        let in_vector = StubEmbedder.embed("always use podman").await.unwrap();
        store
            .upsert(
                "probe",
                &id,
                in_vector.clone(),
                payload("always use podman"),
            )
            .await
            .expect("seed");
        let (out_vector, _) = store
            .get("probe", &id)
            .await
            .unwrap()
            .expect("round-tripped");
        assert_eq!(
            in_vector, out_vector,
            "the fractional part survives the round-trip"
        );
    }

    /// `handle_search_memory` with `StubEmbedder` (no `NoisyEmbedder`
    /// workaround) succeeds: the search path is no longer a landmine that
    /// every test had to step around.
    #[tokio::test]
    async fn handle_search_memory_runs_with_stub_embedder() {
        let (store, _live_id) = store_with_one_rule("probe").await;
        let text = handle_search_memory(
            serde_json::json!({ "namespace": "probe", "query": "podman" }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;
        assert_ne!(
            text, "Failed to search database.",
            "the search path no longer hits ARRAY_DISTANCE's integer-array binder error: {text}"
        );
    }

    /// A store holding two rules in two different domains, so a domain filter
    /// has something to choose between.
    async fn store_with_two_domains(namespace: &str) -> (Arc<dyn VectorStore>, String, String) {
        let store: Arc<dyn VectorStore> = Arc::new(crate::guard::testkit::MockStore::new());
        store.init(namespace).await.expect("init the namespace");
        let mut auth = payload("the auth service terminates mTLS");
        auth.metadata = serde_json::json!({ "domain": "auth" });
        let mut data = payload("the database access layer runs on podman");
        data.metadata = serde_json::json!({ "domain": "database-access" });
        let auth_id = "auth-rule".to_string();
        let data_id = "data-rule".to_string();
        store
            .upsert(namespace, &auth_id, vec![0.1, 0.2, 0.3, 0.4], auth)
            .await
            .expect("seed the auth rule");
        store
            .upsert(namespace, &data_id, vec![0.5, 0.6, 0.7, 0.8], data)
            .await
            .expect("seed the database rule");
        (store, auth_id, data_id)
    }

    /// A search with no domain reads exactly as it did before the filter
    /// existed: the same rows, and no mode line, because no substitution
    /// happened.
    #[tokio::test]
    async fn search_memory_without_a_domain_behaves_exactly_as_today() {
        let (store, auth_id, data_id) = store_with_two_domains("probe").await;

        let report = handle_search_memory(
            serde_json::json!({ "namespace": "probe", "query": "rules" }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;

        assert!(report.contains(&auth_id), "the auth rule is back: {report}");
        assert!(
            report.contains(&data_id),
            "the database rule is back: {report}"
        );
        assert_eq!(
            report.matches("--- Memory ID:").count(),
            2,
            "exactly the two rules, and no others: {report}"
        );
        assert!(
            !report.contains("domain filter:"),
            "no mode line when no domain was asked for: {report}"
        );
        assert!(
            report.starts_with("--- Memory ID:"),
            "the report opens with the memories, unchanged: {report}"
        );
    }

    /// A domain filter narrows the rows, and the report says so. The mode line
    /// is the honest half of the filter: the caller is told whether the
    /// filter chose the rows.
    #[tokio::test]
    async fn search_memory_with_a_domain_filters_the_results_and_reports_the_mode() {
        let (store, auth_id, data_id) = store_with_two_domains("probe").await;

        let report = handle_search_memory(
            serde_json::json!({ "namespace": "probe", "query": "rules", "domain": "auth" }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;

        assert!(
            report.contains(&auth_id),
            "the matching rule is back: {report}"
        );
        assert!(
            !report.contains(&data_id),
            "the other domain's rule is gone: {report}"
        );
        assert!(
            report.starts_with("domain filter: applied (domain)"),
            "the report says the filter chose the rows: {report}"
        );
    }

    /// A domain that matches nothing is skipped rather than emptying the
    /// results -- a wrong domain must never blind the search -- and the report
    /// says the rows were not chosen by the filter.
    #[tokio::test]
    async fn search_memory_with_a_domain_that_matches_nothing_reports_the_skip() {
        let (store, auth_id, data_id) = store_with_two_domains("probe").await;

        let report = handle_search_memory(
            serde_json::json!({ "namespace": "probe", "query": "rules", "domain": "no-such-domain" }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;

        assert!(
            report.contains(&auth_id),
            "the rows come back unfiltered: {report}"
        );
        assert!(
            report.contains(&data_id),
            "the rows come back unfiltered: {report}"
        );
        assert!(
            report.starts_with(
                "domain filter: skipped — no row matched the domain; the results are unfiltered (vector)"
            ),
            "the report says the filter was skipped: {report}"
        );
    }

    /// A domain that normalizes to nothing is not a silent unfiltered search:
    /// the caller asked for a filter, so the report says the filter was
    /// skipped and why. Both argument shapes that carry it are covered.
    #[tokio::test]
    async fn search_memory_with_a_domain_that_does_not_normalize_reports_the_skip() {
        let (store, auth_id, data_id) = store_with_two_domains("probe").await;

        for domain in [serde_json::json!("!!!"), serde_json::json!(["!!!"])] {
            let report = handle_search_memory(
                serde_json::json!({ "namespace": "probe", "query": "rules", "domain": domain }),
                Arc::new(StubEmbedder),
                store.clone(),
            )
            .await;

            assert!(
                report.contains(&auth_id),
                "the rows come back unfiltered: {report}"
            );
            assert!(
                report.contains(&data_id),
                "the rows come back unfiltered: {report}"
            );
            assert!(
                report.starts_with(
                    "domain filter: skipped — the domain did not normalize; the results are unfiltered (vector)"
                ),
                "the report says the domain did not normalize: {report}"
            );
        }
    }

    /// An argument that names no domain at all is not a request for one: the
    /// report stays byte-identical to the one a search without a filter gives.
    #[tokio::test]
    async fn search_memory_with_an_empty_domain_argument_reports_no_mode() {
        let (store, auth_id, data_id) = store_with_two_domains("probe").await;

        for domain in [
            serde_json::json!([]),
            serde_json::json!(""),
            serde_json::json!([""]),
            serde_json::json!("   "),
        ] {
            let report = handle_search_memory(
                serde_json::json!({ "namespace": "probe", "query": "rules", "domain": domain }),
                Arc::new(StubEmbedder),
                store.clone(),
            )
            .await;

            assert!(
                report.contains(&auth_id) && report.contains(&data_id),
                "both rules are back: {report}"
            );
            assert!(
                !report.contains("domain filter:"),
                "no mode line when no domain was asked for: {report}"
            );
        }
    }

    /// A legacy row whose stored domain is free text still matches the slug:
    /// the comparison normalizes the stored value, so the filter works
    /// without a migration and without rewriting the row.
    #[tokio::test]
    async fn search_memory_with_a_free_text_domain_matches_the_legacy_row() {
        let store: Arc<dyn VectorStore> = Arc::new(crate::guard::testkit::MockStore::new());
        store.init("probe").await.expect("init the namespace");
        let mut legacy = payload("the legacy database rule");
        legacy.metadata = serde_json::json!({ "domain": "Database Access" });
        store
            .upsert("probe", "legacy-rule", vec![0.1, 0.2, 0.3, 0.4], legacy)
            .await
            .expect("seed the legacy rule");

        let report = handle_search_memory(
            serde_json::json!({ "namespace": "probe", "query": "rules", "domain": "Database Access" }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;

        assert!(
            report.contains("legacy-rule"),
            "the legacy row is back: {report}"
        );
        assert!(
            report.starts_with("domain filter: applied (domain)"),
            "the report says the filter chose the rows: {report}"
        );
    }

    // ── the bus, as the request handlers see it ────────────────────────────────

    /// Records every pulse the dispatcher hands it, in order. The bus has no
    /// "what did you carry" query by design, so a test that wants to assert on
    /// a pulse registers this and reads the log.
    struct PulseRecorder {
        seen: Arc<std::sync::Mutex<Vec<ThalamicPulse>>>,
    }

    #[async_trait::async_trait]
    impl crate::events::MemorySubscriber for PulseRecorder {
        fn name(&self) -> &'static str {
            "pulse_recorder"
        }

        async fn handle(
            &self,
            event: &ThalamicPulse,
            _token: &RecursionToken,
            _ctx: &crate::events::SubscriberContext<'_>,
        ) -> Result<(), crate::events::SubscriberError> {
            self.seen.lock().expect("recorder lock").push(event.clone());
            Ok(())
        }
    }

    /// A bus for a test that only needs the emit site to be callable -- the
    /// paths that are rejected before any write, where nothing should be
    /// published and nobody is asserting on it.
    fn quiet_bus() -> Arc<ThalamicBus> {
        recording_bus().0
    }

    /// A bus with one recorder on it, plus the recorder's log.
    ///
    /// No store is attached: the recorder never reads `ctx.store`, and an
    /// attach would only couple these tests to a temp database they do not
    /// need.
    fn recording_bus() -> (Arc<ThalamicBus>, Arc<std::sync::Mutex<Vec<ThalamicPulse>>>) {
        let bus = Arc::new(ThalamicBus::new(64));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        bus.register(Box::new(PulseRecorder { seen: seen.clone() }));
        (bus, seen)
    }

    /// Wait for the dispatcher to deliver `at_least` pulses, then hand back
    /// everything it recorded. Polls rather than sleeping a fixed interval: the
    /// dispatcher is a background task, so the only question is whether it has
    /// caught up yet.
    async fn pulses_seen(
        seen: &Arc<std::sync::Mutex<Vec<ThalamicPulse>>>,
        at_least: usize,
    ) -> Vec<ThalamicPulse> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if seen.lock().expect("recorder lock").len() >= at_least {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the dispatcher delivered the pulses");
        seen.lock().expect("recorder lock").clone()
    }

    async fn store_with_one_rule(namespace: &str) -> (Arc<dyn VectorStore>, String) {
        let dir = std::env::temp_dir().join(format!("ns-supersede-{}", uuid::Uuid::new_v4()));
        let store: Arc<dyn VectorStore> = Arc::new(
            crate::store::ladybug::LadybugStore::for_testing(&dir, 4).expect("open temp database"),
        );
        store.init(namespace).await.expect("create the schema");
        let id = uuid::Uuid::new_v4().to_string();
        let vector = StubEmbedder.embed("always use podman").await.unwrap();
        store
            .upsert(namespace, &id, vector, payload("always use podman"))
            .await
            .expect("seed the rule");
        (store, id)
    }

    #[tokio::test]
    async fn editing_a_memory_into_another_namespace_keeps_it() {
        let (store, id) = store_with_one_rule("probe").await;

        let outcome = edit_memory(
            &*store,
            &StubEmbedder,
            "probe",
            &id,
            "other",
            "always use podman, pinned",
            "docs/tools.md",
        )
        .await
        .expect("edit");

        assert_eq!(outcome, EditOutcome::Edited);
        let (_, edited) = store
            .get("other", &id)
            .await
            .unwrap()
            .expect("the memory is in its new namespace");
        assert_eq!(edited.content, "always use podman, pinned");
        assert_eq!(edited.location, "docs/tools.md");
        assert!(
            store.get("probe", &id).await.unwrap().is_none(),
            "and nowhere else"
        );
    }

    #[tokio::test]
    async fn an_ingested_node_is_not_edited() {
        let (store, _) = store_with_one_rule("probe").await;
        let mut node = payload("Path: src/a.rs");
        node.user_id = "auto-ingestor".to_string();
        store
            .upsert("probe", "probe::src/a.rs", vec![0.0; 4], node)
            .await
            .unwrap();

        let outcome = edit_memory(
            &*store,
            &StubEmbedder,
            "probe",
            "probe::src/a.rs",
            "probe",
            "hand-written",
            "",
        )
        .await
        .expect("edit");

        assert_eq!(outcome, EditOutcome::Ingested);
        let (_, kept) = store
            .get("probe", "probe::src/a.rs")
            .await
            .unwrap()
            .expect("still there");
        assert_eq!(kept.content, "Path: src/a.rs", "and unchanged");
    }

    fn new_id_from(message: &str) -> String {
        // "Superseded <old> with <new> in namespace <ns>."
        message
            .split(" with ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .expect("the reply names the replacement")
            .to_string()
    }

    /// The bi-temporal contract: the old wording survives, stamped with the
    /// moment it stopped being true, and the correction is a separate row.
    #[tokio::test]
    async fn superseding_retires_the_old_row_and_keeps_its_text() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let reply = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "probe", "content": "always use docker" }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.starts_with("Superseded"), "got {}", reply);

        let (_, old) = store
            .get("probe", &old_id)
            .await
            .unwrap()
            .expect("still readable by id");
        assert_eq!(
            old.content, "always use podman",
            "history keeps its own wording"
        );
        assert!(old.metadata["valid_to"].as_i64().is_some(), "retired");
        assert_eq!(
            old.metadata["superseded_by"],
            serde_json::json!(new_id_from(&reply))
        );

        let (_, new) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .expect("the replacement was stored");
        assert_eq!(new.content, "always use docker");
        assert_eq!(new.metadata["supersedes"], serde_json::json!(old_id));
        assert!(new.metadata["valid_from"].as_i64().is_some());
        assert!(
            new.metadata
                .get("valid_to")
                .map(|v| v.is_null())
                .unwrap_or(true),
            "the replacement is current, not history"
        );
    }

    #[tokio::test]
    async fn superseding_with_replacement_locations_drops_stale_absolute_path() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let (vector, mut old) = store.get("probe", &old_id).await.unwrap().unwrap();
        old.metadata["absolute_path"] = serde_json::json!("/old/root/missing.rs");
        store.upsert("probe", &old_id, vector, old).await.unwrap();

        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": old_id,
                "namespace": "probe",
                "content": "replacement source rule",
                "locations": [{ "path": "docs/new-source.md" }]
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.starts_with("Superseded"), "got {}", reply);

        let (_, replacement) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .expect("replacement exists");
        assert_eq!(replacement.location, "docs/new-source.md");
        assert!(replacement.metadata.get("absolute_path").is_none());
    }

    /// The defect that made editing useless: the row kept the vector of text it
    /// no longer contained, so search matched the wording we replaced.
    #[tokio::test]
    async fn the_replacement_is_embedded_from_its_own_text() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let reply = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "probe", "content": "always use docker" }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;

        let (old_vector, _) = store.get("probe", &old_id).await.unwrap().unwrap();
        let (new_vector, _) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            old_vector, new_vector,
            "the correction carries its own embedding"
        );
        assert_eq!(
            new_vector,
            StubEmbedder.embed("always use docker").await.unwrap()
        );
    }

    /// History is not a chain to rewrite: correct the current row instead.
    #[tokio::test]
    async fn a_retired_memory_cannot_be_superseded_again() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
        let args = serde_json::json!({ "id": old_id, "namespace": "probe", "content": "second" });

        handle_supersede_memory(args.clone(), emb.clone(), store.clone(), quiet_bus()).await;
        let again = handle_supersede_memory(args, emb, store, quiet_bus()).await;
        assert!(again.contains("already superseded"), "got {}", again);
    }

    /// Retiring a row hides it from every project on the machine, so the global
    /// stratum takes the same guard the destructive tools carry.
    #[tokio::test]
    async fn the_global_namespace_needs_an_explicit_flag() {
        let (store, old_id) = store_with_one_rule("global").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let refused = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "global", "content": "changed" }),
            emb.clone(),
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(refused.contains("ERROR [GLOBAL]"), "got {}", refused);

        let (_, untouched) = store.get("global", &old_id).await.unwrap().unwrap();
        assert!(
            untouched.metadata.get("valid_to").is_none(),
            "nothing was retired"
        );

        let allowed = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "global", "content": "changed", "allow_global": true }),
            emb,
            store,
            quiet_bus(),
        )
        .await;
        assert!(allowed.starts_with("Superseded"), "got {}", allowed);
    }

    /// A correction is still an insertion, so it meets the same scanner.
    #[tokio::test]
    async fn a_secret_in_the_correction_is_refused() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let refused = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "probe", "content": "use ghp_aaaabbbbccccdddd" }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(refused.contains("ERROR [SECURITY]"), "got {}", refused);

        let (_, untouched) = store.get("probe", &old_id).await.unwrap().unwrap();
        assert!(
            untouched.metadata.get("valid_to").is_none(),
            "nothing was retired"
        );
    }

    // ── Task 3a: non-object metadata rejection ─────────────────────────

    #[tokio::test]
    async fn add_memory_returns_id_and_structured_deferred_edge_details() {
        let (store, _) = store_with_one_rule("probe").await;
        let reply = handle_add_memory(
            serde_json::json!({
                "content": "rule with a not-yet-ingested target",
                "namespace": "probe",
                "metadata": { "governs": ["src/not-ingested.rs"] }
            }),
            Arc::new(StubEmbedder),
            store,
            None,
            quiet_bus(),
        )
        .await;

        assert!(reply.contains("Memory ID:"), "{}", reply);
        assert!(reply.contains("Deferred GOVERNS edge[0]"), "{}", reply);
        assert!(reply.contains("src/not-ingested.rs"), "{}", reply);
        assert!(reply.contains("missing"), "{}", reply);
    }

    #[tokio::test]
    async fn non_object_metadata_on_add_is_rejected() {
        let (store, _) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        // String metadata — legacy shape
        let reply = handle_add_memory(
            serde_json::json!({
                "content": "a rule",
                "namespace": "probe",
                "create_new_namespace": true,
                "metadata": "not-an-object",
            }),
            emb,
            store.clone(),
            None,
            quiet_bus(),
        )
        .await;
        assert!(
            reply.contains("ERROR"),
            "string metadata should be rejected: {}",
            reply
        );
        assert!(reply.contains("metadata"), "{}", reply);
    }

    /// The write path normalizes a string domain to its slug, so a stored
    /// domain is already a key. A non-string domain is rejected by the
    /// registry check (see `a_non_string_domain_is_rejected`), so nothing is
    /// guessed at and nothing is stored.
    #[tokio::test]
    async fn a_string_domain_is_normalized_on_write() {
        let (store, _) = store_with_one_rule("probe").await;

        let added = handle_add_memory(
            serde_json::json!({
                "content": "the database access rule",
                "namespace": "probe",
                "domain": "Database Access",
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            quiet_bus(),
        )
        .await;
        assert!(added.starts_with("Successfully added memory"), "{}", added);

        let stored = store
            .list("probe", None, None)
            .await
            .expect("list")
            .into_iter()
            .find(|r| r.payload.content == "the database access rule")
            .expect("the memory is stored");
        assert_eq!(
            stored.payload.metadata["domain"],
            serde_json::json!("database-access")
        );
    }

    /// A string domain that has no usable slug is stored exactly as it was
    /// given: the write never drops a domain and never invents one.
    #[tokio::test]
    async fn a_domain_that_normalizes_to_nothing_is_stored_as_given() {
        let (store, _) = store_with_one_rule("probe").await;

        let added = handle_add_memory(
            serde_json::json!({
                "content": "a rule whose domain has no slug",
                "namespace": "probe",
                "domain": "!!!",
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            quiet_bus(),
        )
        .await;
        assert!(added.starts_with("Successfully added memory"), "{}", added);

        let stored = store
            .list("probe", None, None)
            .await
            .expect("list")
            .into_iter()
            .find(|r| r.payload.content == "a rule whose domain has no slug")
            .expect("the memory is stored");
        assert_eq!(stored.payload.metadata["domain"], serde_json::json!("!!!"));
    }

    /// A domain that arrived in the caller's own metadata is normalized like
    /// any other: the spec's data contract holds for every memory's
    /// `metadata.domain`, not only for the ones carried by the argument.
    #[tokio::test]
    async fn a_metadata_supplied_domain_is_normalized_on_write() {
        let (store, _) = store_with_one_rule("probe").await;

        let added = handle_add_memory(
            serde_json::json!({
                "content": "a rule whose domain arrives in the metadata",
                "namespace": "probe",
                "metadata": { "domain": "Database Access" },
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            quiet_bus(),
        )
        .await;
        assert!(added.starts_with("Successfully added memory"), "{}", added);

        let stored = store
            .list("probe", None, None)
            .await
            .expect("list")
            .into_iter()
            .find(|r| r.payload.content == "a rule whose domain arrives in the metadata")
            .expect("the memory is stored");
        assert_eq!(
            stored.payload.metadata["domain"],
            serde_json::json!("database-access")
        );
    }

    // ── Task 3: the write-path domain registry check ───────────────────

    /// Seed one row carrying `domain` exactly as a legacy write left it (raw,
    /// not normalized), so the bootstrap's and the reconcile's scans are
    /// exercised against the shape the store really holds.
    async fn seed_domain_row(
        store: &Arc<dyn VectorStore>,
        namespace: &str,
        content: &str,
        domain: serde_json::Value,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let vector = StubEmbedder.embed(content).await.unwrap();
        let mut p = payload(content);
        p.metadata = serde_json::json!({ "domain": domain });
        store
            .upsert(namespace, &id, vector, p)
            .await
            .expect("seed the row");
        id
    }

    async fn seed_row(store: &Arc<dyn VectorStore>, namespace: &str, content: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let vector = StubEmbedder.embed(content).await.unwrap();
        store
            .upsert(namespace, &id, vector, payload(content))
            .await
            .expect("seed the row");
        id
    }

    async fn registry_of(store: &Arc<dyn VectorStore>, namespace: &str) -> Option<Vec<String>> {
        store
            .get(namespace, &crate::registry::registry_row_id(namespace))
            .await
            .expect("read the registry row")
            .map(|(_, p)| crate::registry::registry_domains(&p.metadata))
    }

    async fn add_with_domain(
        store: &Arc<dyn VectorStore>,
        domain: serde_json::Value,
        register_domain: Option<bool>,
    ) -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CTR: AtomicUsize = AtomicUsize::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let mut arguments = serde_json::json!({
            "content": format!("a rule written under a domain: {} #{}", domain, n),
            "namespace": "probe",
            "domain": domain,
        });
        if let Some(register) = register_domain {
            arguments["register_domain"] = serde_json::json!(register);
        }
        handle_add_memory(
            arguments,
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            quiet_bus(),
        )
        .await
    }

    #[tokio::test]
    async fn a_write_in_a_namespace_with_no_registry_row_bootstraps_the_registry() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        seed_domain_row(
            &store,
            "probe",
            "the legacy database rule",
            serde_json::json!("Database Access"),
        )
        .await;

        let reply = add_with_domain(&store, serde_json::json!("Auth"), None).await;

        assert!(reply.starts_with("Successfully added memory"), "{}", reply);
        assert_eq!(
            registry_of(&store, "probe").await,
            Some(vec!["auth".to_string(), "database-access".to_string()]),
            "the bootstrap grandfathers the namespace's legacy domain and adds the new one"
        );
    }

    #[tokio::test]
    async fn an_unregistered_slug_is_rejected_and_nothing_is_written() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        seed_domain_row(
            &store,
            "probe",
            "the legacy database rule",
            serde_json::json!("Database Access"),
        )
        .await;
        let boot = add_with_domain(&store, serde_json::json!("Auth"), None).await;
        assert!(boot.starts_with("Successfully added memory"), "{}", boot);

        let before = store.list("probe", None, None).await.expect("list").len();
        let reply = add_with_domain(&store, serde_json::json!("storage"), None).await;

        assert!(reply.contains("ERROR [DOMAIN]"), "{}", reply);
        assert!(
            reply.contains("not registered in namespace 'probe'"),
            "{}",
            reply
        );
        assert!(
            reply.contains(r#"["auth","database-access"]"#),
            "the rejection names the registered list: {}",
            reply
        );
        assert!(reply.contains("register_domain: true"), "{}", reply);

        let after = store.list("probe", None, None).await.expect("list");
        assert_eq!(after.len(), before, "a rejection writes nothing");
        assert!(
            after
                .iter()
                .all(|r| r.payload.metadata["domain"] != "storage"),
            "the rejected row never landed"
        );
    }

    #[tokio::test]
    async fn register_domain_true_accepts_the_slug_and_registers_it() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        seed_domain_row(
            &store,
            "probe",
            "the legacy database rule",
            serde_json::json!("Database Access"),
        )
        .await;
        let boot = add_with_domain(&store, serde_json::json!("Auth"), None).await;
        assert!(boot.starts_with("Successfully added memory"), "{}", boot);

        let reply = add_with_domain(&store, serde_json::json!("storage"), Some(true)).await;

        assert!(reply.starts_with("Successfully added memory"), "{}", reply);
        assert_eq!(
            registry_of(&store, "probe").await,
            Some(vec![
                "auth".to_string(),
                "database-access".to_string(),
                "storage".to_string()
            ])
        );
    }

    #[tokio::test]
    async fn a_slug_the_namespace_already_carries_is_accepted_by_the_reconcile() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        seed_domain_row(
            &store,
            "probe",
            "the legacy database rule",
            serde_json::json!("Database Access"),
        )
        .await;
        let boot = add_with_domain(&store, serde_json::json!("Auth"), None).await;
        assert!(boot.starts_with("Successfully added memory"), "{}", boot);

        // A cache that lost the slug the namespace's own rows still carry: the
        // next write of that slug must be accepted, not rejected.
        let registry_id = crate::registry::registry_row_id("probe");
        let (vector, mut row) = store
            .get("probe", &registry_id)
            .await
            .expect("read the registry row")
            .expect("the bootstrap wrote it");
        row.metadata = crate::registry::registry_metadata(&["auth".to_string()]);
        store
            .upsert("probe", &registry_id, vector, row)
            .await
            .expect("stale the cache");

        let reply = add_with_domain(&store, serde_json::json!("database-access"), None).await;

        assert!(reply.starts_with("Successfully added memory"), "{}", reply);
        assert_eq!(
            registry_of(&store, "probe").await,
            Some(vec!["auth".to_string(), "database-access".to_string()]),
            "the reconcile repaired the cache"
        );

        // And a deleted registry row re-bootstraps from the namespace's rows.
        store
            .delete("probe", &registry_id)
            .await
            .expect("delete the cache");
        let reply = add_with_domain(&store, serde_json::json!("database-access"), None).await;
        assert!(reply.starts_with("Successfully added memory"), "{}", reply);
        assert_eq!(
            registry_of(&store, "probe").await,
            Some(vec!["auth".to_string(), "database-access".to_string()])
        );
    }

    #[tokio::test]
    async fn a_non_string_domain_is_rejected() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        seed_row(&store, "probe", "the legacy rule").await;

        let before = store.list("probe", None, None).await.expect("list").len();
        let reply = add_with_domain(&store, serde_json::json!(42), None).await;

        assert!(reply.contains("ERROR [DOMAIN]"), "{}", reply);
        assert!(
            reply.contains("Registered domains: []"),
            "the rejection names the registered list: {}",
            reply
        );
        assert!(
            reply.contains("not a string"),
            "the rejection says what is wrong: {}",
            reply
        );
        assert_eq!(
            store.list("probe", None, None).await.expect("list").len(),
            before,
            "nothing is written"
        );
    }

    #[tokio::test]
    async fn a_domain_with_no_usable_slug_is_stored_as_it_was_given() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        seed_row(&store, "probe", "the legacy rule").await;

        let reply = add_with_domain(&store, serde_json::json!("!!!"), None).await;

        assert!(reply.starts_with("Successfully added memory"), "{}", reply);
        let stored = store
            .list("probe", None, None)
            .await
            .expect("list")
            .into_iter()
            .find(|r| r.payload.content.starts_with("a rule written under a domain"))
            .expect("the memory is stored");
        assert_eq!(stored.payload.metadata["domain"], serde_json::json!("!!!"));
        assert_eq!(
            registry_of(&store, "probe").await,
            None,
            "a slugless domain is not governed by the registry"
        );
    }

    #[tokio::test]
    async fn a_write_with_no_domain_touches_no_registry_row() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        seed_row(&store, "probe", "the legacy rule").await;

        let reply = handle_add_memory(
            serde_json::json!({
                "content": "a rule with no domain",
                "namespace": "probe",
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            quiet_bus(),
        )
        .await;

        assert!(reply.starts_with("Successfully added memory"), "{}", reply);
        assert_eq!(registry_of(&store, "probe").await, None);
    }

    #[tokio::test]
    async fn non_object_metadata_on_supersede_is_rejected() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        // Manually write a row with scalar metadata (legacy shape)
        let mut legacy = payload("old rule");
        legacy.metadata = serde_json::json!("legacy-string");
        let v = StubEmbedder.embed("old rule").await.unwrap();
        store.upsert("probe", &old_id, v, legacy).await.unwrap();

        let reply = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "probe", "content": "new rule" }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.contains("legacy-shaped"), "got {}", reply);

        // Original still active — no silent no-op with no lineage stamps.
        let (_, still) = store.get("probe", &old_id).await.unwrap().unwrap();
        assert!(
            still.metadata.get("valid_to").is_none(),
            "must not be retired"
        );
    }

    // ── Task 3b: future valid_to transfers to replacement ──────────────

    #[tokio::test]
    async fn future_expiry_supersede_succeeds_and_transfers_expiry() {
        let dir = std::env::temp_dir().join(format!("ns-supersede-{}", uuid::Uuid::new_v4()));
        let store: Arc<dyn VectorStore> = Arc::new(
            crate::store::ladybug::LadybugStore::for_testing(&dir, 4).expect("open temp database"),
        );
        store.init("probe").await.expect("create the schema");
        let id = uuid::Uuid::new_v4().to_string();

        // Row with a future valid_to — still active (not yet retired).
        let future_ts = chrono::Utc::now().timestamp() + 3600;
        let mut meta_payload = payload("scheduled rule");
        meta_payload.metadata = serde_json::json!({ "valid_to": future_ts });
        let v = StubEmbedder.embed("scheduled rule").await.unwrap();
        store.upsert("probe", &id, v, meta_payload).await.unwrap();

        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
        let reply = handle_supersede_memory(
            serde_json::json!({ "id": id, "namespace": "probe", "content": "corrected rule" }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.starts_with("Superseded"), "got {}", reply);

        // Replacement inherits the future expiry.
        let (_, new_row) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            new_row.metadata.get("valid_to").and_then(|v| v.as_i64()),
            Some(future_ts),
            "future expiry should transfer to replacement"
        );
    }

    /// A correction to a claimed task keeps the claim: the work continues
    /// under the new id, and the reply names the release path so the claim
    /// is released deliberately rather than by accident.
    #[tokio::test]
    async fn superseding_a_claimed_task_inherits_the_claim_and_names_the_release_path() {
        let dir = std::env::temp_dir().join(format!("ns-supersede-claim-{}", uuid::Uuid::new_v4()));
        let store: Arc<dyn VectorStore> = Arc::new(
            crate::store::ladybug::LadybugStore::for_testing(&dir, 4).expect("open temp database"),
        );
        store.init("probe").await.expect("create the schema");
        let id = uuid::Uuid::new_v4().to_string();

        let mut claimed = payload("claimed task");
        claimed.memory_type = "task".to_string();
        claimed.metadata = serde_json::json!({
            "task": {
                "status": "in_progress",
                "assignee": "agent-a",
                "session_id": "ses-old",
                "updated_at": "2026-10-18T00:00:00Z"
            }
        });
        let v = StubEmbedder.embed("claimed task").await.unwrap();
        store.upsert("probe", &id, v, claimed).await.unwrap();

        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
        let reply = handle_supersede_memory(
            serde_json::json!({ "id": id, "namespace": "probe", "content": "corrected claimed task" }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.starts_with("Superseded"), "got {}", reply);
        let successor = new_id_from(&reply);

        let (_, new_row) = store.get("probe", &successor).await.unwrap().unwrap();
        assert_eq!(
            new_row.metadata["task"]["status"],
            serde_json::json!("in_progress"),
            "the work continues under the new id"
        );
        assert_eq!(
            new_row.metadata["task"]["assignee"],
            serde_json::json!("agent-a")
        );
        assert!(
            reply.contains("neurostrata_task_update") && reply.contains(&successor),
            "the reply must name the release path for the inherited claim: {}",
            reply
        );
    }

    // ── Task 3c: optimistic guard aborts on concurrent change ──────────

    /// Guinea-pig BUG-10: a correction must be able to fix a stale structural
    /// pointer. Explicit metadata replaces wholesale; absent metadata is
    /// carried over AND named in the reply -- never silently stale.
    #[tokio::test]
    async fn supersede_replaces_structural_metadata_and_reports_what_it_kept() {
        let dir = std::env::temp_dir().join(format!("ns-supersede-{}", uuid::Uuid::new_v4()));
        let store: Arc<dyn VectorStore> = Arc::new(
            crate::store::ladybug::LadybugStore::for_testing(&dir, 4).expect("open temp database"),
        );
        store.init("probe").await.expect("create the schema");
        let id = uuid::Uuid::new_v4().to_string();

        let mut meta_payload = payload("stale pointer rule");
        meta_payload.metadata =
            serde_json::json!({ "governs": ["gone/file.rs"], "related_to": ["someone"] });
        let v = StubEmbedder.embed("stale pointer rule").await.unwrap();
        store.upsert("probe", &id, v, meta_payload).await.unwrap();

        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": id,
                "namespace": "probe",
                "content": "corrected pointer rule",
                "governs": ["live/file.rs"]
            }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.starts_with("Superseded"), "got {}", reply);
        assert!(
            reply.contains("Replaced metadata: governs"),
            "got {}",
            reply
        );
        assert!(
            reply.contains("Carried over UNCHANGED from the old memory: related_to"),
            "the carried pointer must be named: {}",
            reply
        );

        let (_, new_row) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            new_row.metadata["governs"],
            serde_json::json!(["live/file.rs"])
        );
        assert_eq!(
            new_row.metadata["related_to"],
            serde_json::json!(["someone"])
        );
    }

    /// Guinea-pig round 3 (Option 1 completion + Option 2): locations replace
    /// wholesale, an unambiguous content Governs line is honoured, and the
    /// reply names which source set the governing pointer. Conflicting lines
    /// are refused rather than guessed.
    #[tokio::test]
    async fn supersede_replaces_locations_and_honours_the_content_governs_line() {
        let dir = std::env::temp_dir().join(format!("ns-supersede-{}", uuid::Uuid::new_v4()));
        let store: Arc<dyn VectorStore> = Arc::new(
            crate::store::ladybug::LadybugStore::for_testing(&dir, 4).expect("open temp database"),
        );
        store.init("probe").await.expect("create the schema");
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        // A: locations parameter replaces refs wholesale and derives governs.
        let id = uuid::Uuid::new_v4().to_string();
        let mut meta_payload = payload("anchored rule");
        meta_payload.metadata = serde_json::json!({
            "governs": ["dead/old.go"],
            "refs": [{ "file": "dead/old.go" }]
        });
        let v = StubEmbedder.embed("anchored rule").await.unwrap();
        store.upsert("probe", &id, v, meta_payload).await.unwrap();
        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": id,
                "namespace": "probe",
                "content": "anchored rule v2",
                "locations": [{ "path": "live/new.go", "lines": "1-9" }]
            }),
            emb.clone(),
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.starts_with("Superseded"), "got {}", reply);
        assert!(
            reply.contains("Replaced metadata: locations"),
            "got {}",
            reply
        );
        assert!(
            reply.contains("set from the locations parameter"),
            "got {}",
            reply
        );
        let (_, row_a) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row_a.metadata["refs"][0]["file"],
            serde_json::json!("live/new.go")
        );
        let expected = crate::parser::ingest::normalize_node_path("live/new.go");
        assert_eq!(row_a.metadata["governs"], serde_json::json!([expected]));

        // B: no parameters -- the prose's single Governs line is the author's
        // declaration and replaces the stale inherited pointer.
        let id = uuid::Uuid::new_v4().to_string();
        let mut meta_payload = payload("pointer rule");
        meta_payload.metadata = serde_json::json!({ "governs": ["dead/old.go"] });
        let v = StubEmbedder.embed("pointer rule").await.unwrap();
        store.upsert("probe", &id, v, meta_payload).await.unwrap();
        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": id,
                "namespace": "probe",
                "content": "pointer rule v2\nGoverns: [\"live/new.go\"]"
            }),
            emb.clone(),
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(
            reply.contains("governs (from your content line)"),
            "got {}",
            reply
        );
        assert!(
            reply.contains("set from the Governs line in your content"),
            "got {}",
            reply
        );
        let (_, row_b) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row_b.metadata["governs"],
            serde_json::json!(["live/new.go"])
        );

        // C: two conflicting lines and no parameter -- refuse to guess, say so.
        let id = uuid::Uuid::new_v4().to_string();
        let mut meta_payload = payload("conflicted rule");
        meta_payload.metadata = serde_json::json!({ "governs": ["dead/old.go"] });
        let v = StubEmbedder.embed("conflicted rule").await.unwrap();
        store.upsert("probe", &id, v, meta_payload).await.unwrap();
        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": id,
                "namespace": "probe",
                "content": "conflicted rule v2\nGoverns: [\"a.go\"]\nGoverns: [\"b.go\"]"
            }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(
            reply.contains("2 conflicting Governs lines"),
            "got {}",
            reply
        );
        let (_, row_c) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row_c.metadata["governs"],
            serde_json::json!(["dead/old.go"])
        );
    }

    /// FEATURE-4 (rr66): a correction must be able to say WHY. The why is
    /// stamped on the row that became history -- beside the pointer to its
    /// successor -- so reading the retired memory answers "what did it say" AND
    /// "why did it stop being true".
    #[tokio::test]
    async fn the_reason_is_recorded_on_the_retired_memory() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": old_id,
                "namespace": "probe",
                "content": "always use podman",
                "reason": "the blanket rule was wrong for host-network services"
            }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(
            reply.contains("Recorded why on the retired memory as superseded_reason"),
            "got {}",
            reply
        );
        assert!(
            reply.contains("blanket rule was wrong"),
            "the reply quotes the why: {}",
            reply
        );

        let (_, old) = store.get("probe", &old_id).await.unwrap().expect("history");
        assert_eq!(
            old.metadata["superseded_reason"],
            serde_json::json!("the blanket rule was wrong for host-network services")
        );
        assert!(
            old.metadata["superseded_by"].as_str().is_some(),
            "the why sits next to the pointer"
        );

        // The live assertion is not a correction record; its lineage lives on
        // the old row, reachable through the supersedes pointer.
        let (_, new) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new.metadata["supersedes"], serde_json::json!(old_id));
        assert!(
            new.metadata.get("superseded_reason").is_none(),
            "the why belongs to history: {:?}",
            new.metadata
        );
    }

    /// The corrected text asserts something new and needs its own origin: the
    /// old source says where the wrong wording came from, not where this does.
    #[tokio::test]
    async fn a_passed_source_replaces_the_provenance_on_the_correction() {
        let store = crate::store::ladybug::LadybugStore::for_testing(
            std::env::temp_dir().join(format!("ns-supersede-{}", uuid::Uuid::new_v4())),
            4,
        )
        .expect("open temp database");
        let store: Arc<dyn VectorStore> = Arc::new(store);
        store.init("probe").await.expect("create the schema");
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        // A: the dated-string form, replacing the old provenance.
        let id = uuid::Uuid::new_v4().to_string();
        let mut old = payload("rule about containers");
        old.metadata = serde_json::json!({ "source": "hallucinated 2020-01-01" });
        store
            .upsert(
                "probe",
                &id,
                StubEmbedder.embed("rule about containers").await.unwrap(),
                old,
            )
            .await
            .unwrap();
        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": id,
                "namespace": "probe",
                "content": "rule about containers, v2",
                "source": "owner 2026-10-09",
                "reason": "the old rule was invented"
            }),
            emb.clone(),
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.contains("Replaced metadata: source"), "got {}", reply);
        assert!(
            !reply.contains("Carried over UNCHANGED from the old memory: source"),
            "nothing carried: {}",
            reply
        );
        let (_, new_a) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            new_a.metadata["source"],
            serde_json::json!("owner 2026-10-09")
        );
        let (_, old_a) = store.get("probe", &id).await.unwrap().unwrap();
        assert_eq!(
            old_a.metadata["source"],
            serde_json::json!("hallucinated 2020-01-01"),
            "history keeps its own origin"
        );

        // B: the structured form, with the same semantics add_memory accepts.
        let id = uuid::Uuid::new_v4().to_string();
        store
            .upsert(
                "probe",
                &id,
                StubEmbedder.embed("another rule").await.unwrap(),
                payload("another rule"),
            )
            .await
            .unwrap();
        let structured = serde_json::json!({
            "kind": "commit",
            "ref": "abc1234",
            "captured_at": "2026-10-09"
        });
        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": id,
                "namespace": "probe",
                "content": "another rule, v2",
                "source": structured
            }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        let (_, new_b) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new_b.metadata["source"], structured);
    }

    /// Regression drill (rr66): the omission paths. A superseded-without-a-why
    /// and a rule corrected without provenance are both reported out loud --
    /// neither writes a half-record and calls it success.
    #[tokio::test]
    async fn a_correction_without_a_reason_or_source_says_so() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let reply = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "probe", "content": "always use podman, v2" }),
            emb.clone(),
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(
            reply.starts_with("Superseded"),
            "the correction itself still lands: {}",
            reply
        );
        assert!(
            reply.contains("NO reason was recorded"),
            "the missing why is named: {}",
            reply
        );
        assert!(
            reply.contains("NO metadata.source"),
            "the missing provenance is named: {}",
            reply
        );
        assert!(
            reply.contains("rule_without_source"),
            "and what it costs: {}",
            reply
        );

        let (_, old) = store.get("probe", &old_id).await.unwrap().unwrap();
        assert!(
            old.metadata.get("superseded_reason").is_none(),
            "nothing written for a reason that was never given: {:?}",
            old.metadata
        );
        let (_, new) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert!(
            new.metadata.get("source").is_none(),
            "and no invented provenance"
        );
    }

    /// Whitespace is not a reason and a blank source is not provenance: both
    /// take the omission path rather than satisfying a presence check with noise.
    #[tokio::test]
    async fn a_blank_reason_and_source_are_treated_as_omissions() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": old_id,
                "namespace": "probe",
                "content": "always use podman, v3",
                "reason": "   ",
                "source": ""
            }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply.contains("NO reason was recorded"), "got {}", reply);
        assert!(reply.contains("NO metadata.source"), "got {}", reply);
        let (_, old) = store.get("probe", &old_id).await.unwrap().unwrap();
        assert!(
            old.metadata.get("superseded_reason").is_none(),
            "no blank why stored"
        );
    }

    /// Provenance omitted but inherited: the old origin carries over, and the
    /// reply names what it inherited so nobody reads "superseded" as "re-sourced".
    #[tokio::test]
    async fn an_omitted_source_carries_the_old_provenance_over_and_says_so() {
        let store = crate::store::ladybug::LadybugStore::for_testing(
            std::env::temp_dir().join(format!("ns-supersede-{}", uuid::Uuid::new_v4())),
            4,
        )
        .expect("open temp database");
        let store: Arc<dyn VectorStore> = Arc::new(store);
        store.init("probe").await.expect("create the schema");
        let id = uuid::Uuid::new_v4().to_string();
        let mut old = payload("sourced rule");
        old.metadata = serde_json::json!({ "source": "owner 2026-10-08", "governs": ["a.rs"] });
        store
            .upsert(
                "probe",
                &id,
                StubEmbedder.embed("sourced rule").await.unwrap(),
                old,
            )
            .await
            .unwrap();

        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);
        let reply = handle_supersede_memory(
            serde_json::json!({ "id": id, "namespace": "probe", "content": "sourced rule, v2" }),
            emb,
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(
            reply.contains("Carried over UNCHANGED from the old memory: governs, source"),
            "got {}",
            reply
        );
        assert!(
            !reply.contains("NO metadata.source"),
            "inherited provenance is not absent: {}",
            reply
        );
        let (_, new) = store
            .get("probe", &new_id_from(&reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            new.metadata["source"],
            serde_json::json!("owner 2026-10-08")
        );
    }

    /// A record must never render two Governs lists with nothing marking the
    /// authoritative half (guinea-pig "in every option" ask).
    #[test]
    fn a_prose_governs_line_that_disagrees_is_marked_in_the_render() {
        let mut disagreeing = payload("rule text\nGoverns: [\"a.go\"]");
        disagreeing.metadata = serde_json::json!({ "governs": ["b.go"] });
        let out = format_memory("ns-1", &disagreeing);
        assert!(
            out.contains("DISAGREES with the authoritative metadata"),
            "got {}",
            out
        );

        let mut agreeing = payload("rule text\nGoverns: [\"a.go\"]");
        agreeing.metadata = serde_json::json!({ "governs": ["a.go"] });
        assert!(governance_conflict_note(&agreeing.content, &agreeing.metadata).is_none());
    }

    /// A concurrent supersede between the first read and the guard re-read
    /// causes the second supersede to detect the change and abort.
    #[tokio::test]
    async fn concurrent_supersede_is_detected_by_optimistic_guard() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        // First supersede succeeds.
        let reply1 = handle_supersede_memory(
            serde_json::json!({ "id": old_id, "namespace": "probe", "content": "first correction" }),
            emb.clone(),
            store.clone(),
            quiet_bus(),
        )
        .await;
        assert!(reply1.starts_with("Superseded"), "first: {}", reply1);

        // The old row is now retired, so a second supersede targeting the
        // ORIGINAL id should be refused (retired check), which proves the
        // guard would also catch a concurrent race. For a true race test,
        // we verify the re-read detects valid_to.
        let (_, retired) = store.get("probe", &old_id).await.unwrap().unwrap();
        assert!(
            retired.metadata.get("valid_to").is_some(),
            "first supersede retired the row"
        );
    }

    // ── Secret rejection on add prevents store write ────────────────────

    #[tokio::test]
    async fn secret_in_add_content_prevents_store_write() {
        let (store, _) = store_with_one_rule("probe").await;
        let emb: Arc<dyn Embedder> = Arc::new(StubEmbedder);

        let before = store.list("probe", None, None).await.unwrap().len();

        let reply = handle_add_memory(
            serde_json::json!({
                "content": "password = secret123",
                "namespace": "probe",
                "create_new_namespace": true,
            }),
            emb,
            store.clone(),
            None,
            quiet_bus(),
        )
        .await;
        assert!(reply.contains("ERROR [SECURITY]"), "got {}", reply);

        let after = store.list("probe", None, None).await.unwrap().len();
        assert_eq!(before, after, "no memory should be stored after rejection");
    }

    // ── Snapshot handler: fallible encoding (Task 2) ───────────────────

    #[tokio::test]
    async fn snapshot_handler_returns_ok_for_normal_data() {
        let (store, _) = store_with_one_rule("probe").await;
        let result = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store).await;
        assert!(result.is_ok(), "snapshot should succeed: {:?}", result);
        let text = result.unwrap();
        assert!(
            text.contains("always use podman"),
            "should contain the rule: {}",
            text
        );
    }

    #[tokio::test]
    async fn snapshot_handler_missing_namespace_is_err() {
        let dir = std::env::temp_dir().join(format!("ns-snap-{}", uuid::Uuid::new_v4()));
        let store: Arc<dyn VectorStore> = Arc::new(
            crate::store::ladybug::LadybugStore::for_testing(&dir, 4).expect("open temp database"),
        );
        let result = handle_get_snapshot(serde_json::json!({}), store).await;
        assert!(result.is_err(), "missing namespace should be err");
    }

    // ── Search evidence rendering tests ───────────────────────────────

    use crate::traits::{EvidenceEdge, EvidenceKind, SearchEvidence};

    fn search_result_with_evidence(
        id: &str,
        memory_type: &str,
        content: &str,
        evidence: Option<Vec<SearchEvidence>>,
    ) -> SearchResult {
        SearchResult {
            id: id.to_string(),
            score: if evidence.is_some() { 10.0 } else { 0.0 },
            payload: MemoryPayload {
                content: content.to_string(),
                user_id: "u".into(),
                memory_type: memory_type.to_string(),
                agent_name: None,
                location: String::new(),
                location_lines: String::new(),
                metadata: serde_json::json!({}),
            },
            evidence,
        }
    }

    #[test]
    fn search_direct_match_text_is_byte_identical() {
        let r = search_result_with_evidence(
            "probe::src/a.rs::run",
            "code_ast",
            "symbol",
            Some(vec![SearchEvidence {
                kind: EvidenceKind::DirectMatch,
                path: vec![],
                explanation: "This memory matched the query vector.".into(),
            }]),
        );
        let text = render_search_results(&[r]);
        let expected = format_memory(
            "probe::src/a.rs::run",
            &MemoryPayload {
                content: "symbol".into(),
                user_id: "u".into(),
                memory_type: "code_ast".into(),
                agent_name: None,
                location: String::new(),
                location_lines: String::new(),
                metadata: serde_json::json!({}),
            },
        );
        assert_eq!(
            text, expected,
            "direct match should produce byte-identical output to format_memory"
        );
    }

    #[test]
    fn get_memory_text_is_unchanged_by_search_evidence() {
        let p = MemoryPayload {
            content: "rule text".into(),
            user_id: "u".into(),
            memory_type: "rule".into(),
            agent_name: None,
            location: String::new(),
            location_lines: String::new(),
            metadata: serde_json::json!({}),
        };
        let text = format_memory("rule-1", &p);
        assert!(
            !text.contains("Path"),
            "get output should not contain evidence paths"
        );
        assert!(
            !text.contains("Why:"),
            "get output should not contain evidence explanations"
        );
    }

    #[test]
    fn search_evidence_respects_per_result_byte_budget() {
        // Very short budget: 10 bytes. Evidence suffix is much longer.
        let r = search_result_with_evidence(
            "probe::src/a.rs",
            "file",
            "file",
            Some(vec![SearchEvidence {
                kind: EvidenceKind::OneHop,
                path: vec![EvidenceEdge {
                    source: "probe::src/a.rs::run".into(),
                    relation: "CONTAINS".into(),
                    target: "probe::src/a.rs".into(),
                }],
                explanation: "This memory is one graph hop from a direct match.".into(),
            }]),
        );
        let suffix = render_evidence_suffix(&r, 10);
        assert!(suffix.is_none(), "suffix should not fit in 10 bytes");
    }

    #[test]
    fn search_evidence_respects_total_added_byte_budget() {
        let r1 = search_result_with_evidence(
            "probe::src/a.rs",
            "file",
            "file",
            Some(vec![SearchEvidence {
                kind: EvidenceKind::OneHop,
                path: vec![EvidenceEdge {
                    source: "probe::src/a.rs::run".into(),
                    relation: "CONTAINS".into(),
                    target: "probe::src/a.rs".into(),
                }],
                explanation: "This memory is one graph hop from a direct match.".into(),
            }]),
        );
        // First result fits in 2048 bytes
        let text = render_search_results(&[r1]);
        assert!(text.len() <= 2048);

        // Many results with evidence should still respect the total budget
        let results: Vec<_> = (0..20)
            .map(|i| {
                search_result_with_evidence(
                    &format!("mem-{}", i),
                    "rule",
                    "content",
                    Some(vec![SearchEvidence {
                        kind: EvidenceKind::OneHop,
                        path: vec![EvidenceEdge {
                            source: "probe::src/a.rs::run".into(),
                            relation: "CONTAINS".into(),
                            target: format!("mem-{}", i),
                        }],
                        explanation: "This memory is one graph hop from a direct match.".into(),
                    }]),
                )
            })
            .collect();
        let text = render_search_results(&results);
        // Account for varying ID lengths ("mem-0" vs "mem-19") and separators.
        let max_per_result_len = results
            .iter()
            .map(|r| format_memory(&r.id, &r.payload).len())
            .max()
            .unwrap();
        let separator_overhead = (results.len() - 1) * 2; // "\n\n" between results
        assert!(
            text.len() <= max_per_result_len * 20 + separator_overhead + 2048,
            "total response should respect budget"
        );
    }

    #[test]
    fn search_evidence_never_truncates_or_splits_ids() {
        let r = search_result_with_evidence(
            "probe::src/a.rs",
            "file",
            "file",
            Some(vec![SearchEvidence {
                kind: EvidenceKind::OneHop,
                path: vec![EvidenceEdge {
                    source: "probe::src/a.rs::run".into(),
                    relation: "CONTAINS".into(),
                    target: "probe::src/a.rs".into(),
                }],
                explanation: "This memory is one graph hop from a direct match.".into(),
            }]),
        );
        let text = render_search_results(&[r]);
        assert!(
            text.contains("probe::src/a.rs::run"),
            "full source ID must appear"
        );
        assert!(
            text.contains("probe::src/a.rs"),
            "full target ID must appear"
        );
        assert!(
            !text.contains("\"probe::src/a.\""),
            "no truncated ID in path quotes"
        );
    }

    #[test]
    fn search_evidence_escapes_newlines_in_ids() {
        // Newlines in IDs would break the one-line-per-path rule.
        // In practice IDs don't contain newlines, but the renderer must not crash.
        let r = search_result_with_evidence(
            "normal-id",
            "file",
            "file",
            Some(vec![SearchEvidence {
                kind: EvidenceKind::OneHop,
                path: vec![EvidenceEdge {
                    source: "id-with\nnewline".into(),
                    relation: "CONTAINS".into(),
                    target: "normal-id".into(),
                }],
                explanation: "This memory is one graph hop from a direct match.".into(),
            }]),
        );
        // Should not panic; renderer handles it
        let _text = render_search_results(&[r]);
    }

    #[test]
    fn search_evidence_tries_later_shorter_paths() {
        // Long path that doesn't fit: a two-hop path whose ids are long
        // enough that the rendered line exceeds the byte budget.
        let long_id = "a-very-long-namespace-identifier-".repeat(3);
        let long_path = vec![
            EvidenceEdge {
                source: long_id.clone(),
                relation: "RELATES_TO".into(),
                target: format!("{}-mid", long_id),
            },
            EvidenceEdge {
                source: format!("{}-mid", long_id),
                relation: "GOVERNS".into(),
                target: "target-id".into(),
            },
        ];
        // Short path that fits: single edge from primary to result.id
        let short_path = vec![EvidenceEdge {
            source: "a".into(),
            relation: "CONTAINS".into(),
            target: "target-id".into(),
        }];
        let r = search_result_with_evidence(
            "target-id",
            "rule",
            "rule",
            Some(vec![
                SearchEvidence {
                    kind: EvidenceKind::Governs2hop,
                    path: long_path,
                    explanation: "long".into(),
                },
                SearchEvidence {
                    kind: EvidenceKind::OneHop,
                    path: short_path,
                    explanation: "short".into(),
                },
            ]),
        );
        let suffix_result = render_evidence_suffix(&r, 200);
        let suffix = suffix_result.expect("should get a suffix");
        // The short path has node "a" — the long path has "a-very-long-..."
        assert!(
            suffix.contains("\"a\""),
            "should use the shorter path that fits"
        );
    }

    /// The mesh dispatch arm is the only production route from the wire to
    /// the mesh module. Mutation-probe evidence (checker-free, integration
    /// round): with the prefix sabotaged, the ENTIRE suite stayed green —
    /// this test is what makes that arm load-bearing. It asserts ROUTING,
    /// not mesh behaviour (the unit tests own behaviour): a mesh tool name
    /// must NOT fall through to the "Unknown tool" handler.
    #[tokio::test]
    async fn a_mesh_tool_routes_through_the_wire_surface() {
        let scratch =
            std::env::temp_dir().join(format!("ns-mcp-mesh-route-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&scratch).unwrap();
        let (store, _) = store_with_one_rule("probe").await;
        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "neurostrata_synaptic_mesh_status",
                "arguments": { "project_root": scratch.display().to_string() }
            })),
        };
        let (bus, _seen) = recording_bus();
        let response =
            process_mcp_request(request, Arc::new(StubEmbedder), store, ingests, None, bus).await;
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("content text");
        assert!(
            !text.contains("Unknown tool"),
            "mesh status must route past the fallthrough: {text}"
        );
        assert!(
            text.starts_with("no mesh joined") || text.contains("mesh"),
            "expected the canonical no-mesh answer, got: {text}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    // ── Characterization suite (Task 20, J28/J20) ────────────────────────
    //
    // Golden request/response pairs for `process_mcp_request`, written
    // BEFORE the dispatch split and required to pass against the
    // unmodified dispatch: they pin the wire behaviour the split must
    // preserve — same request JSON, same response JSON, same error
    // strings, same side effects. Where a response carries a
    // nondeterministic value (a minted id), the test pins the shape
    // (prefix, keys, types) and threads the value into the next call
    // instead of asserting the literal.

    /// Drive one `tools/call` through the dispatch exactly as the stdio
    /// surface does and return the whole JSON-RPC response value.
    async fn call_tool(store: Arc<dyn VectorStore>, name: &str, arguments: Value) -> Value {
        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({ "name": name, "arguments": arguments })),
        };
        let (bus, _seen) = recording_bus();
        process_mcp_request(request, Arc::new(StubEmbedder), store, ingests, None, bus).await
    }

    /// The text payload of a successful `tools/call` response.
    fn result_text(response: &Value) -> String {
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("a successful tools/call carries content text")
            .to_string()
    }

    /// The id an add_memory reply minted: the `Memory ID: <id>` line's
    /// value (the reply may carry further lines after it).
    fn minted_id(add_reply: &str) -> String {
        add_reply
            .lines()
            .find_map(|line| line.strip_prefix("Memory ID: "))
            .expect("the add reply names the minted id")
            .trim()
            .to_string()
    }

    #[tokio::test]
    async fn characterization_add_and_get_memory_round_trip() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let added = call_tool(
            store.clone(),
            "neurostrata_add_memory",
            serde_json::json!({
                "content": "characterization probe rule for the dispatch split",
                "namespace": "probe",
                "memory_type": "fact",
            }),
        )
        .await;
        let text = result_text(&added);
        assert!(
            text.starts_with("Successfully added memory for namespace: probe"),
            "add reply shape: {text}"
        );
        let id = minted_id(&text);
        let got = call_tool(
            store,
            "neurostrata_get_memory",
            serde_json::json!({ "id": id, "namespace": "probe" }),
        )
        .await;
        let got_text = result_text(&got);
        assert!(
            got_text.contains("characterization probe rule for the dispatch split"),
            "get returns what add stored: {got_text}"
        );
    }

    #[tokio::test]
    async fn characterization_search_memory_returns_the_seeded_rule() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let found = call_tool(
            store,
            "neurostrata_search_memory",
            serde_json::json!({ "namespace": "probe", "query": "podman" }),
        )
        .await;
        let text = result_text(&found);
        assert!(
            text.contains("always use podman"),
            "search renders the seeded rule: {text}"
        );
    }

    #[tokio::test]
    async fn characterization_list_namespaces_uses_the_live_format() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let listed = call_tool(store, "neurostrata_list_namespaces", serde_json::json!({})).await;
        assert_eq!(
            result_text(&listed),
            "Existing namespaces: [\"probe\"]",
            "the dispatch's namespace listing format is pinned verbatim"
        );
    }

    #[tokio::test]
    async fn characterization_get_snapshot_carries_head_and_seeded_row() {
        let (store, seed_id) = store_with_one_rule("probe").await;
        let snapshot = call_tool(
            store,
            "neurostrata_get_snapshot",
            serde_json::json!({ "namespace": "probe" }),
        )
        .await;
        let text = result_text(&snapshot);
        assert!(
            text.contains("Zero-Action Start: claim or create a task before editing."),
            "snapshot opens with the task head: {text}"
        );
        assert!(
            text.contains(&seed_id),
            "snapshot body carries the seeded row: {text}"
        );
    }

    #[tokio::test]
    async fn characterization_get_snapshot_without_namespace_is_a_jsonrpc_error() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let response = call_tool(store, "neurostrata_get_snapshot", serde_json::json!({})).await;
        assert!(
            response.get("result").is_none(),
            "a refused snapshot is not a success payload: {response}"
        );
        assert_eq!(response["error"]["code"], serde_json::json!(-32603));
        assert_eq!(
            response["error"]["message"],
            serde_json::json!("Missing 'namespace' parameter.")
        );
    }

    #[tokio::test]
    async fn characterization_task_create_claim_complete_lifecycle() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let created = call_tool(
            store.clone(),
            "neurostrata_task_create",
            serde_json::json!({ "namespace": "probe", "title": "characterization task" }),
        )
        .await;
        let created_text = result_text(&created);
        let created_json: Value =
            serde_json::from_str(&created_text).expect("task_create replies with JSON");
        let task_id = created_json["id"]
            .as_str()
            .expect("the create reply names the task id")
            .to_string();
        assert_eq!(created_json["task"]["status"], serde_json::json!("open"));
        assert_eq!(
            created_json["task"]["title"],
            serde_json::json!("characterization task")
        );

        let claimed = call_tool(
            store.clone(),
            "neurostrata_task_claim",
            serde_json::json!({
                "id": task_id,
                "namespace": "probe",
                "session_id": "characterization-session",
                "assignee": "characterization",
            }),
        )
        .await;
        let claimed_text = result_text(&claimed);
        let claimed_json: Value =
            serde_json::from_str(&claimed_text).expect("task_claim replies with JSON");
        assert_eq!(claimed_json["status"], serde_json::json!("in_progress"));
        assert_eq!(claimed_json["claimed"], serde_json::json!(true));
        assert_eq!(
            claimed_json["transition"],
            serde_json::json!({ "from": "open", "to": "in_progress" })
        );

        let completed = call_tool(
            store,
            "neurostrata_task_complete",
            serde_json::json!({
                "id": task_id,
                "namespace": "probe",
                "memory": { "content": "characterization lesson: golden pairs pin the dispatch" },
            }),
        )
        .await;
        let completed_text = result_text(&completed);
        let completed_json: Value =
            serde_json::from_str(&completed_text).expect("task_complete replies with JSON");
        assert_eq!(completed_json["task"]["status"], serde_json::json!("done"));
        assert_eq!(
            completed_json["transition"]["to"],
            serde_json::json!("done")
        );
        assert!(
            !completed_json["extracted_memory_ids"]
                .as_array()
                .expect("extracted ids are an array")
                .is_empty(),
            "the inline extraction was recorded: {completed_text}"
        );
    }

    #[tokio::test]
    async fn characterization_task_complete_without_extraction_is_blocked() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let created = call_tool(
            store.clone(),
            "neurostrata_task_create",
            serde_json::json!({ "namespace": "probe", "title": "characterization blocked task" }),
        )
        .await;
        let created_json: Value =
            serde_json::from_str(&result_text(&created)).expect("task_create replies with JSON");
        let task_id = created_json["id"].as_str().expect("task id").to_string();
        let claimed = call_tool(
            store.clone(),
            "neurostrata_task_claim",
            serde_json::json!({
                "id": task_id,
                "namespace": "probe",
                "session_id": "characterization-session",
            }),
        )
        .await;
        assert!(
            result_text(&claimed).contains("in_progress"),
            "the claim precedes the refused completion"
        );
        let response = call_tool(
            store,
            "neurostrata_task_complete",
            serde_json::json!({ "id": task_id, "namespace": "probe" }),
        )
        .await;
        assert!(
            response.get("result").is_none(),
            "a refused completion is a JSON-RPC error, never a success payload: {response}"
        );
        assert_eq!(response["error"]["code"], serde_json::json!(-32603));
        let message = response["error"]["message"]
            .as_str()
            .expect("the error carries a message");
        assert!(
            message.starts_with("BLOCKED (Lock 2)"),
            "the Lock 2 refusal text is pinned: {message}"
        );
    }

    #[tokio::test]
    async fn characterization_procedure_perform_spends_one_fire() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let added = call_tool(
            store.clone(),
            "neurostrata_add_memory",
            serde_json::json!({
                "content": "characterization procedure body",
                "namespace": "probe",
                "memory_type": "procedure",
                "metadata": { "trigger": "session-start", "remaining_fires": 3 },
            }),
        )
        .await;
        let proc_id = minted_id(&result_text(&added));
        let performed = call_tool(
            store,
            "neurostrata_procedure_perform",
            serde_json::json!({ "id": proc_id }),
        )
        .await;
        let reply_text = result_text(&performed);
        let reply: Value =
            serde_json::from_str(&reply_text).expect("procedure_perform replies with JSON");
        assert_eq!(reply["performed"], serde_json::json!(true));
        assert_eq!(reply["remaining_fires"], serde_json::json!(2));
        assert_eq!(reply["performance_count"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn characterization_supersede_records_its_reason() {
        let (store, seed_id) = store_with_one_rule("probe").await;
        let superseded = call_tool(
            store.clone(),
            "neurostrata_supersede_memory",
            serde_json::json!({
                "id": seed_id,
                "namespace": "probe",
                "content": "always use podman, never docker (characterization correction)",
                "reason": "characterization correction",
            }),
        )
        .await;
        let text = result_text(&superseded);
        assert!(
            text.contains(
                "Recorded why on the retired memory as superseded_reason: characterization correction."
            ),
            "the supersede report names the recorded reason: {text}"
        );
        let found = call_tool(
            store,
            "neurostrata_search_memory",
            serde_json::json!({ "namespace": "probe", "query": "podman" }),
        )
        .await;
        assert!(
            result_text(&found).contains("never docker (characterization correction)"),
            "search finds the correction afterwards"
        );
    }

    #[tokio::test]
    async fn characterization_append_log_lands_in_the_sessions_buffer() {
        let scratch =
            std::env::temp_dir().join(format!("ns-char-append-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&scratch).unwrap();
        let (store, _seed) = store_with_one_rule("probe").await;
        let appended = call_tool(
            store,
            "neurostrata_append_log",
            serde_json::json!({
                "content": "characterization log entry",
                "project_root": scratch.display().to_string(),
            }),
        )
        .await;
        let text = result_text(&appended);
        assert!(
            !text.contains("ERROR"),
            "append_log accepts the entry: {text}"
        );
        let sessions = scratch.join(".NeuroStrata").join("sessions");
        let entries = std::fs::read_dir(&sessions)
            .expect("the sessions buffer directory exists")
            .count();
        assert!(entries >= 1, "the entry landed in the buffer: {sessions:?}");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn characterization_guard_and_ingest_route_to_their_validators() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let guard = call_tool(store.clone(), "neurostrata_guard_validate", serde_json::json!({}))
            .await;
        assert_eq!(
            result_text(&guard),
            "ERROR: 'action_type' is required for guard_validate."
        );
        let ingest = call_tool(store, "neurostrata_ingest_directory", serde_json::json!({})).await;
        assert_eq!(result_text(&ingest), "ERROR: dir_path missing.");
    }

    #[tokio::test]
    async fn characterization_unknown_tool_missing_params_and_method_shapes() {
        let (store, _seed) = store_with_one_rule("probe").await;
        let unknown = call_tool(store.clone(), "neurostrata_nope", serde_json::json!({})).await;
        assert_eq!(
            result_text(&unknown),
            "Unknown tool: neurostrata_nope",
            "the unknown-tool fallthrough text is pinned verbatim"
        );

        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        let (bus, _seen) = recording_bus();
        let bare_call = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(7)),
            method: "tools/call".to_string(),
            params: None,
        };
        let response = process_mcp_request(
            bare_call,
            Arc::new(StubEmbedder),
            store.clone(),
            ingests.clone(),
            None,
            bus.clone(),
        )
        .await;
        assert_eq!(result_text(&response), "Tool execution failed");

        let initialize = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(9)),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({})),
        };
        let response = process_mcp_request(
            initialize,
            Arc::new(StubEmbedder),
            store.clone(),
            ingests.clone(),
            None,
            bus.clone(),
        )
        .await;
        assert_eq!(
            response["result"]["protocolVersion"],
            serde_json::json!("2024-11-05")
        );
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            serde_json::json!("neurostrata-mcp")
        );
        assert_eq!(response["id"], serde_json::json!(9));

        for method in ["notifications/initialized", "bogus/method"] {
            let request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: None,
                method: method.to_string(),
                params: None,
            };
            let response = process_mcp_request(
                request,
                Arc::new(StubEmbedder),
                store.clone(),
                ingests.clone(),
                None,
                bus.clone(),
            )
            .await;
            assert_eq!(
                response,
                serde_json::json!({}),
                "{method} answers with the empty object"
            );
        }
    }

    /// Every tool the surface exposes, in the order tools/list returns them.
    /// Clients pin this list; a silent rename orphans their wiring.
    /// Keep in sync with the entries in `process_mcp_request` -> `"tools/list"`.
    #[tokio::test]
    async fn tools_list_carries_all_thirty_tools_in_a_stable_order() {
        let (store, _) = store_with_one_rule("probe").await;
        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "tools/list".to_string(),
            params: Some(serde_json::json!({})),
        };
        let (bus, _seen) = recording_bus();
        let response =
            process_mcp_request(request, Arc::new(StubEmbedder), store, ingests, None, bus).await;
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools/list returns an array");
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().expect("every tool is named"))
            .collect();
        assert_eq!(
            names,
            vec![
                "neurostrata_add_memory",
                "neurostrata_get_memory",
                "neurostrata_get_snapshot",
                "neurostrata_ingest_directory",
                "neurostrata_list_namespaces",
                "neurostrata_search_memory",
                "neurostrata_supersede_memory",
                "neurostrata_procedure_perform",
                "neurostrata_task_create",
                "neurostrata_task_claim",
                "neurostrata_task_update",
                "neurostrata_task_list",
                "neurostrata_task_complete",
                "neurostrata_task_validate",
                "neurostrata_bootstrap",
                "neurostrata_task_setup",
                "neurostrata_append_log",
                "neurostrata_guard_validate",
                "neurostrata_guard_learn",
                "neurostrata_guard_list_rules",
                "neurostrata_archive_memory",
                "neurostrata_task_intake",
                "neurostrata_task_intake_list",
                "neurostrata_task_intake_resolve",
                // SynapticMesh: appended by `tools.extend(synaptic_mesh_tool
                // _definitions())` — this order IS the append order, so it
                // pins where the mesh module's list enters the surface.
                "neurostrata_synaptic_mesh_init",
                "neurostrata_synaptic_mesh_join",
                "neurostrata_synaptic_mesh_push",
                "neurostrata_synaptic_mesh_pull",
                "neurostrata_synaptic_mesh_leave",
                "neurostrata_synaptic_mesh_status",
            ]
        );
        for tool in tools {
            assert!(
                tool["inputSchema"].is_object(),
                "{} needs an input schema",
                tool["name"]
            );
        }
        // rr66: the correction surface can say WHY and WHERE FROM. If these
        // vanish from the schema, provenance on corrections dies silently.
        let supersede = tools
            .iter()
            .find(|t| t["name"] == "neurostrata_supersede_memory")
            .expect("the correction tool is exposed");
        assert!(
            supersede["inputSchema"]["properties"]["reason"].is_object(),
            "supersede takes a reason"
        );
        assert!(
            supersede["inputSchema"]["properties"]["source"].is_object(),
            "supersede takes a source"
        );
        // The tombstone tool's schema is the only thing a client reads before
        // calling it, so a schema that silently lost a required field would
        // turn every archive call into a runtime error instead.
        let archive = tools
            .iter()
            .find(|t| t["name"] == "neurostrata_archive_memory")
            .expect("the archive tool is exposed");
        assert!(
            archive["inputSchema"]["properties"]["id"].is_object(),
            "archive takes an id"
        );
        assert!(
            archive["inputSchema"]["properties"]["namespace"].is_object(),
            "archive takes a namespace"
        );
        let required: Vec<&str> = archive["inputSchema"]["required"]
            .as_array()
            .expect("archive declares what it requires")
            .iter()
            .map(|v| v.as_str().expect("a required field name"))
            .collect();
        assert_eq!(
            required,
            vec!["id", "namespace"],
            "both fields are required"
        );
        // The vocabulary summaries are still appended to their tools.
        let summaries = crate::traits::memory_vocabulary()
            .get("tool_summaries")
            .unwrap()
            .clone();
        assert!(summaries.get("add").is_some());
        assert!(summaries.get("search").is_some());
        assert!(summaries.get("ingest").is_some());
        assert!(summaries.get("memory_type").is_some());
        let add_desc = tools[0]["description"].as_str().unwrap();
        assert!(
            add_desc.len() > "Store an architectural rule, project pattern, or task insight.".len(),
            "summary appended: {}",
            add_desc
        );
        // v4 procedural memory: the tool carries both halves of its contract,
        // so a client can read `performed` without a trial firing.
        let perform = tools
            .iter()
            .find(|t| t["name"] == "neurostrata_procedure_perform")
            .expect("the procedure tool is exposed");
        assert_eq!(
            perform["inputSchema"]["required"],
            serde_json::json!(["id"])
        );
        for field in [
            "performed",
            "lapsed",
            "spent",
            "remaining_fires",
            "performance_count",
            "episodic_pointer",
            "reason",
        ] {
            assert!(
                perform["outputSchema"]["properties"][field].is_object(),
                "the reply declares {}",
                field
            );
        }
        // vocabulary v4: an agent reading `tools/list` must see `procedure` in
        // the memory_type description, otherwise the type effectively doesn't
        // exist as far as the surface is concerned. The patch at
        // process_mcp_request -> "tools/list" overlays the shipped vocab's
        // summary on top of the hardcoded value at server.rs:116; this
        // assertion catches any drift that leaves the overlay stale.
        let add_memory = tools
            .iter()
            .find(|t| t["name"] == "neurostrata_add_memory")
            .expect("the add tool is exposed");
        let mem_type_desc = add_memory["inputSchema"]["properties"]["memory_type"]["description"]
            .as_str()
            .expect("memory_type has a description");
        assert!(
            mem_type_desc.contains("procedure"),
            "memory_type description must mention `procedure` (vocab v4): {mem_type_desc:?}"
        );
    }

    /// Vocabulary v4: `procedure` is an ordinary memory type on the
    /// add_memory surface, and every one of its six fields survives the round
    /// trip untouched -- a rehearsal count that does not round-trip is a
    /// rehearsal count that does not exist.
    #[tokio::test]
    async fn a_procedure_round_trips_all_six_fields_through_add_memory() {
        let (store, _) = store_with_one_rule("probe").await;
        let bus = Arc::new(crate::events::ThalamicBus::new(16));

        let added = handle_add_memory(
            serde_json::json!({
                "content": "Ritual: run the pre-push gate before every push.",
                "namespace": "probe",
                "memory_type": "procedure",
                "metadata": {
                    "trigger": "session-start",
                    "remaining_fires": 5,
                    "valid_to": null,
                    "last_performed_at": null,
                    "performance_count": 0,
                    "last_episodic_pointer": null
                }
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            bus.clone(),
        )
        .await;
        assert!(added.starts_with("Successfully added memory"), "{}", added);

        let stored = store
            .list("probe", None, None)
            .await
            .expect("list")
            .into_iter()
            .find(|r| r.payload.memory_type == "procedure")
            .expect("the procedure is stored");
        let meta = &stored.payload.metadata;
        assert_eq!(meta["trigger"], serde_json::json!("session-start"));
        assert_eq!(meta["remaining_fires"], serde_json::json!(5));
        assert_eq!(meta["valid_to"], Value::Null);
        assert_eq!(meta["last_performed_at"], Value::Null);
        assert_eq!(meta["performance_count"], serde_json::json!(0));
        assert_eq!(meta["last_episodic_pointer"], Value::Null);
    }

    /// The strap is wired into the snapshot body, not just implemented: a
    /// live procedure is surfaced, a lapsed one is not, and neither one has to
    /// survive the top-5 access-count cut to be seen.
    #[tokio::test]
    async fn the_snapshot_carries_the_procedures_due_strap() {
        let (store, _) = store_with_one_rule("probe").await;
        let now = chrono::Utc::now().timestamp();
        let mut live = payload("run the gate before every push");
        live.memory_type = "procedure".to_string();
        live.metadata = serde_json::json!({
            "trigger": "session-start",
            "remaining_fires": 2,
            "performance_count": 0
        });
        let mut lapsed = payload("an expired ritual");
        lapsed.memory_type = "procedure".to_string();
        lapsed.metadata = serde_json::json!({
            "trigger": "session-start",
            "valid_to": now - 60
        });
        store
            .upsert("probe", "proc-live", vec![0.1, 0.2, 0.3, 0.4], live)
            .await
            .expect("seed the live procedure");
        store
            .upsert("probe", "proc-lapsed", vec![0.1, 0.2, 0.3, 0.4], lapsed)
            .await
            .expect("seed the lapsed procedure");

        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");

        assert!(text.contains("Procedures due (1):"), "{}", text);
        assert!(text.contains("proc-live"), "{}", text);
        assert!(
            !text.contains("proc-lapsed"),
            "a lapsed procedure is not due: {}",
            text
        );
    }

    /// The whole v1 loop over one MCP call: a procedure surfaces on the strap,
    /// the agent acknowledges it, and the next snapshot shows the spent fire.
    #[tokio::test]
    async fn a_procedure_performed_over_mcp_comes_back_decremented_on_the_strap() {
        let bus = Arc::new(crate::events::ThalamicBus::new(16));
        let (store, _) = store_with_one_rule("probe").await;
        let mut procedure = payload("run the gate before every push");
        procedure.memory_type = "procedure".to_string();
        procedure.metadata = serde_json::json!({
            "trigger": "session-start",
            "remaining_fires": 1,
            "performance_count": 0
        });
        store
            .upsert("probe", "proc-mcp", vec![0.1, 0.2, 0.3, 0.4], procedure)
            .await
            .expect("seed the procedure");

        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        // Scratch root: without an explicit project_root the tool falls back to
        // the working directory, and a test must not append to the repo's own
        // Episodic Buffer.
        let root = std::env::temp_dir().join(format!("ns-procedure-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("scratch project root");
        let call = |id: &str| {
            let ingests = ingests.clone();
            let store = store.clone();
            let arguments =
                serde_json::json!({ "id": id, "project_root": root.display().to_string() });
            async move {
                let request = JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!(7)),
                    method: "tools/call".to_string(),
                    params: Some(serde_json::json!({
                        "name": "neurostrata_procedure_perform",
                        "arguments": arguments
                    })),
                };
                process_mcp_request(
                    request,
                    Arc::new(StubEmbedder),
                    store,
                    ingests,
                    None,
                    bus.clone(),
                )
                .await
            }
        };

        let call2 = call.clone();
        let first: Value = serde_json::from_str(
            &call("proc-mcp").await["result"]["content"][0]["text"]
                .as_str()
                .expect("text")
                .to_string(),
        )
        .expect("json");
        assert_eq!(first["performed"], serde_json::json!(true), "{}", first);
        assert_eq!(first["remaining_fires"], serde_json::json!(0));

        let second: Value = serde_json::from_str(
            &call2("proc-mcp").await["result"]["content"][0]["text"]
                .as_str()
                .expect("text")
                .to_string(),
        )
        .expect("json");
        assert_eq!(second["performed"], serde_json::json!(false));
        assert_eq!(second["spent"], serde_json::json!(true));
        assert_eq!(second["reason"], serde_json::json!("spent"));

        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");
        assert!(text.contains("\"performance_count\": 1"), "{}", text);
        assert!(text.contains("\"remaining_fires\": 0"), "{}", text);
    }

    /// Emit site 1: the write that created a memory is the moment subscribers
    /// get told, without anyone remembering to tell them. Asserts the pulse
    /// carries the id the store actually holds -- a `Created` naming some other
    /// row would send every subscriber to read the wrong memory.
    #[tokio::test]
    async fn adding_a_memory_emits_created_naming_the_stored_row() {
        let (store, _) = store_with_one_rule("probe").await;
        let (bus, seen) = recording_bus();
        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());

        let response = process_mcp_request(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "tools/call".to_string(),
                params: Some(serde_json::json!({
                    "name": "neurostrata_add_memory",
                    "arguments": {
                        "content": "checkpoint after every write",
                        "namespace": "probe",
                        "memory_type": "rule"
                    }
                })),
            },
            Arc::new(StubEmbedder),
            store.clone(),
            ingests,
            None,
            bus,
        )
        .await;

        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .expect("a text result")
                .starts_with("Successfully added memory"),
            "{}",
            response
        );

        let pulses = pulses_seen(&seen, 1).await;
        match &pulses[0] {
            ThalamicPulse::Created {
                id,
                namespace,
                kind,
            } => {
                assert_eq!(namespace, "probe");
                assert_eq!(kind, "rule", "the pulse carries the memory_type");
                // The id is the store's, not a fresh uuid: the whole point of
                // the pulse is that a subscriber can read that row back.
                let stored: Vec<String> = store
                    .list("probe", None, None)
                    .await
                    .expect("list")
                    .into_iter()
                    .map(|r| r.id)
                    .filter(|row| row == id)
                    .collect();
                assert_eq!(stored.len(), 1, "pulse names a row the store holds: {}", id);
            }
            other => panic!("expected Created, got {:?}", other),
        }
    }

    /// Program Task 3 (a): a procedure created through add_memory is
    /// stamped trust="local" at creation — a caller-supplied trust value
    /// is overwritten, because local authorship is a fact about the
    /// write path, not a claim the caller gets to make. The stamped row
    /// fires through the gate; a non-procedure row gains no trust key.
    #[tokio::test]
    async fn locally_created_procedure_carries_local_stamp() {
        let (store, _) = store_with_one_rule("probe").await;
        let (bus, _seen) = recording_bus();

        let reply = handle_add_memory(
            serde_json::json!({
                "content": "rehearse the release checklist",
                "namespace": "probe",
                "memory_type": "procedure",
                "metadata": {
                    "trust": "untrusted",
                    "trigger": "session-start",
                    "remaining_fires": 2
                }
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            bus.clone(),
        )
        .await;
        assert!(
            reply.starts_with("Successfully added memory"),
            "{reply}"
        );

        let rows = store.list("probe", None, None).await.expect("list");
        let proc_row = rows
            .iter()
            .find(|r| r.payload.memory_type == "procedure")
            .expect("the procedure row was stored");
        assert_eq!(
            proc_row.payload.metadata["trust"],
            serde_json::json!("local"),
            "creation stamps local authorship over the caller's claim: {:?}",
            proc_row.payload.metadata
        );

        let root =
            std::env::temp_dir().join(format!("ns-task3-local-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("scratch root");
        let outcome_text = crate::tools::procedure::handle_procedure_perform(
            serde_json::json!({
                "id": proc_row.id,
                "project_root": root.display().to_string()
            }),
            store.clone(),
        )
        .await;
        let outcome: Value = serde_json::from_str(&outcome_text).expect("perform returns JSON");
        assert_eq!(
            outcome["performed"],
            serde_json::json!(true),
            "a locally created procedure fires: {outcome}"
        );

        let reply = handle_add_memory(
            serde_json::json!({
                "content": "a plain local rule about stamping",
                "namespace": "probe",
                "memory_type": "rule",
                "metadata": {}
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            bus,
        )
        .await;
        assert!(
            reply.starts_with("Successfully added memory"),
            "{reply}"
        );
        let rows = store.list("probe", None, None).await.expect("list");
        let rule_row = rows
            .iter()
            .find(|r| r.payload.content == "a plain local rule about stamping")
            .expect("the rule row was stored");
        assert!(
            rule_row.payload.metadata.get("trust").is_none(),
            "non-procedure rows gain no trust stamp: {:?}",
            rule_row.payload.metadata
        );
    }

    /// Emit site 2: a correction is two rows moving, and the pulse has to name
    /// both ends of the move. Asserts old_id is the row that was retired and
    /// new_id the row that replaced it -- a `Superseded` with the ends swapped
    /// would send every subscriber down the wrong chain.
    #[tokio::test]
    async fn superseding_emits_superseded_naming_both_ends_of_the_move() {
        let (store, old_id) = store_with_one_rule("probe").await;
        let (bus, seen) = recording_bus();

        let reply = handle_supersede_memory(
            serde_json::json!({
                "id": old_id,
                "content": "always use podman, never docker",
                "namespace": "probe",
                "reason": "docker is forbidden"
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            bus,
        )
        .await;
        assert!(
            reply.starts_with(&format!("Superseded {}", old_id)),
            "{}",
            reply
        );

        let pulses = pulses_seen(&seen, 1).await;
        match &pulses[0] {
            ThalamicPulse::Superseded {
                old_id: from,
                new_id: to,
                namespace,
            } => {
                assert_eq!(from, &old_id, "the retired row");
                assert_eq!(namespace, "probe");
                assert_ne!(to, from, "the replacement is a different row");
                // Both ends are real rows the store holds, and the old one
                // really is retired: the pulse must not describe a move that
                // did not happen.
                let (vector, retired) = store
                    .get("probe", from)
                    .await
                    .expect("read")
                    .expect("the retired row still exists");
                assert_eq!(vector.len(), 4);
                assert!(
                    retired.metadata["valid_to"].is_i64(),
                    "the old row is retired"
                );
                assert_eq!(
                    retired.metadata["superseded_by"],
                    serde_json::json!(to.as_str())
                );
                let (_, replacement) = store
                    .get("probe", to)
                    .await
                    .expect("read")
                    .expect("the replacement");
                assert_eq!(
                    replacement.metadata["supersedes"],
                    serde_json::json!(from.as_str())
                );
            }
            other => panic!("expected Superseded, got {:?}", other),
        }
    }

    /// A refused supersede changed nothing, so it publishes nothing. The
    /// negative half of the contract: an emit that fires on the failure paths
    /// would have subscribers tracking a move that never landed.
    #[tokio::test]
    async fn a_refused_supersede_emits_nothing() {
        let (store, _) = store_with_one_rule("probe").await;
        let (bus, seen) = recording_bus();

        // No id: refused before any write.
        let reply = handle_supersede_memory(
            serde_json::json!({ "namespace": "probe", "content": "x" }),
            Arc::new(StubEmbedder),
            store.clone(),
            bus,
        )
        .await;
        assert!(reply.starts_with("Missing 'id' parameter"), "{}", reply);

        // The dispatcher is asynchronous, so a pulse in flight would show up
        // late. Give it the window a refusal must stay silent through.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            seen.lock().expect("recorder lock").is_empty(),
            "a refused supersede publishes no pulse"
        );
    }

    /// Emit site 4: the audit row a verdict earns cannot depend on the caller
    /// remembering to record it, so the verdict itself publishes the pulse.
    /// Asserts the pulse is joinable to the response the caller received --
    /// same `trace_id`, same rules -- because an audit row that cannot be tied
    /// back to its call is a list of hashes.
    // Multi-threaded because `GuardValidator::new` builds its sandbox evaluator
    // through `block_in_place`, which a current-thread runtime refuses.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_guard_verdict_emits_guarded_action_fired_joinable_to_the_response() {
        let (store, _) = store_with_one_rule("probe").await;
        let (bus, seen) = recording_bus();

        let reply = handle_guard_validate(
            serde_json::json!({
                "action_type": "bash",
                "payload": "rm -rf /tmp/scratch",
                "namespace": "probe"
            }),
            store.clone(),
            Arc::new(StubEmbedder),
            bus,
        )
        .await;
        assert!(
            reply.starts_with("namespace: probe\ntrace_id: "),
            "{}",
            reply
        );

        let pulses = pulses_seen(&seen, 1).await;
        match &pulses[0] {
            ThalamicPulse::GuardedActionFired {
                trace_id,
                action_type,
                payload_hash,
                verdict,
                rule_ids_triggered,
                namespace,
            } => {
                assert_eq!(namespace, "probe");
                assert_eq!(action_type, "bash");
                // The trace_id is the one the caller was handed, so the audit
                // row joins back to the response rather than to nothing.
                assert!(
                    reply.contains(trace_id.as_str()),
                    "pulse trace_id {} is not in the response:\n{}",
                    trace_id,
                    reply
                );
                // The verdict is the bucket tag the response itself carries,
                // verbatim -- not a re-derived or re-labelled copy.
                let body: Value =
                    serde_json::from_str(reply.splitn(3, '\n').nth(2).expect("the json body"))
                        .expect("the body is json");
                let response_trace = body["trace_id"].as_str().expect("trace in body");
                assert_eq!(trace_id, response_trace);
                assert_eq!(
                    verdict,
                    &body["verdict"]["bucket"]
                        .as_str()
                        .expect("a verdict bucket")
                        .to_string()
                );
                assert_eq!(
                    rule_ids_triggered,
                    &body["rule_ids_triggered"]
                        .as_array()
                        .expect("a rules array")
                        .iter()
                        .map(|v| v.as_str().unwrap_or_default().to_string())
                        .collect::<Vec<String>>()
                );
                // A digest, not a payload: it fits in 32 bits and the raw
                // command is nowhere in it.
                assert_ne!(*payload_hash, 0, "a hash of a real payload is not zero");
                assert!(
                    !reply.contains(&format!("payload_hash\": \"{payload_hash}")),
                    "the hash is a number, not a string"
                );
            }
            other => panic!("expected GuardedActionFired, got {:?}", other),
        }
    }

    /// The audit commitment is a digest of *the payload*, so two calls with
    /// different payloads cannot produce the same commitment -- that is the
    /// whole reason the field exists. Also pins that it is a digest and not a
    /// copy: nothing reconstructable comes back out.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_payload_hash_commits_to_the_payload_and_carries_nothing_else() {
        let (store, _) = store_with_one_rule("probe").await;
        let (bus, seen) = recording_bus();

        let hash_of = |payload: &'static str| {
            let store = store.clone();
            let bus = bus.clone();
            async move {
                handle_guard_validate(
                    serde_json::json!({
                        "action_type": "bash",
                        "payload": payload,
                        "namespace": "probe"
                    }),
                    store,
                    Arc::new(StubEmbedder),
                    bus,
                )
                .await
            }
        };

        hash_of("echo one").await;
        hash_of("echo two").await;
        hash_of("echo one").await;

        let pulses = pulses_seen(&seen, 3).await;
        let hashes: Vec<u32> = pulses
            .iter()
            .map(|p| match p {
                ThalamicPulse::GuardedActionFired { payload_hash, .. } => *payload_hash,
                other => panic!("expected GuardedActionFired, got {:?}", other),
            })
            .collect();
        assert_ne!(
            hashes[0], hashes[1],
            "different payloads, different commitments"
        );
        assert_eq!(
            hashes[0], hashes[2],
            "the same payload commits the same way"
        );
    }

    /// A refused call never reached a verdict, so there is nothing to record.
    /// The audit trail is a trail of *calls that happened*, and a call the
    /// handler rejected on its arguments did not validate anything.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_guard_validate_refused_before_the_verdict_emits_nothing() {
        let (store, _) = store_with_one_rule("probe").await;
        let (bus, seen) = recording_bus();

        let reply = handle_guard_validate(
            serde_json::json!({ "action_type": "bash", "namespace": "probe" }),
            store,
            Arc::new(StubEmbedder),
            bus,
        )
        .await;
        assert!(
            reply.starts_with("ERROR: 'payload' is required"),
            "{}",
            reply
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            seen.lock().expect("recorder lock").is_empty(),
            "a refused guard_validate publishes no audit row"
        );
    }

    /// Emit site 5 (the `Archived` variant's only producer): a tombstone is a
    /// storage mutation like any other, so it publishes one. Asserts the
    /// tombstone is a *merge* -- archiving must not drop the metadata that was
    /// already there, or the row loses its provenance the moment it is retired.
    #[tokio::test]
    async fn archiving_marks_the_row_and_emits_archived_without_dropping_metadata() {
        let (store, id) = store_with_one_rule("probe").await;
        // Give the row something worth losing, so "the tombstone is a merge"
        // is a claim about behaviour rather than a vacuous truth about two
        // empty objects.
        let (vector, mut seeded) = store.get("probe", &id).await.unwrap().expect("the rule");
        seeded.metadata = serde_json::json!({
            "access_count": 3,
            "governs": ["src/daemon.rs"],
            "source": "docs/design-task-subsystem.md",
        });
        store.upsert("probe", &id, vector, seeded).await.unwrap();

        let (bus, seen) = recording_bus();

        let reply = crate::handlers::archive_memory::handle_archive_memory(
            serde_json::json!({ "id": id, "namespace": "probe" }),
            store.clone(),
            bus,
        )
        .await;
        assert!(reply.contains("Archived"), "{}", reply);

        let (_, payload) = store
            .get("probe", &id)
            .await
            .unwrap()
            .expect("still readable");
        assert_eq!(payload.metadata["archived"], serde_json::json!(true));
        assert_eq!(
            payload.metadata["access_count"],
            serde_json::json!(3),
            "the existing metadata survives the tombstone"
        );
        assert_eq!(
            payload.metadata["governs"],
            serde_json::json!(["src/daemon.rs"])
        );
        assert_eq!(
            payload.metadata["source"],
            serde_json::json!("docs/design-task-subsystem.md"),
            "provenance survives too"
        );

        let pulses = pulses_seen(&seen, 1).await;
        match &pulses[0] {
            ThalamicPulse::Archived {
                id: archived,
                namespace,
            } => {
                assert_eq!(archived, &id, "the pulse names the row that was tombstoned");
                assert_eq!(namespace, "probe");
            }
            other => panic!("expected Archived, got {:?}", other),
        }
    }

    /// The Archived variant must not be reachable without a producer. The
    /// negative half of the contract: a handler that publishes on its refusal
    /// paths would tell subscribers a row was retired that is still active.
    #[tokio::test]
    async fn an_archived_id_publishes_nothing() {
        let (store, _) = store_with_one_rule("probe").await;
        let (bus, seen) = recording_bus();

        let reply = crate::handlers::archive_memory::handle_archive_memory(
            serde_json::json!({ "id": "no-such-id", "namespace": "probe" }),
            store,
            bus,
        )
        .await;
        assert!(reply.contains("No memory with id no-such-id"), "{}", reply);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            seen.lock().expect("recorder lock").is_empty(),
            "an unknown id publishes no pulse"
        );
    }

    /// Lock 2 over the wire: a completion with nothing extracted comes back
    /// as a JSON-RPC error carrying the exact section 3.2 body -- never as a
    /// success payload holding an error string.
    #[tokio::test]
    async fn a_blocked_task_complete_is_a_jsonrpc_error_not_a_success_payload() {
        let (store, _) = store_with_one_rule("probe").await;
        let created = crate::task::handle_task_create(
            serde_json::json!({ "namespace": "probe", "title": "the work" }),
            Arc::new(StubEmbedder),
            store.clone(),
        )
        .await;
        let created: Value = serde_json::from_str(&created).expect("create replies with json");
        let id = created["id"].as_str().expect("id").to_string();

        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(41)),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "neurostrata_task_complete",
                "arguments": { "namespace": "probe", "id": id }
            })),
        };
        let (bus, _seen) = recording_bus();
        let response = process_mcp_request(
            request,
            Arc::new(StubEmbedder),
            store.clone(),
            ingests,
            None,
            bus,
        )
        .await;

        assert!(
            response.get("result").is_none(),
            "a blocked completion is not a success: {}",
            response
        );
        assert_eq!(response["error"]["code"], INTERNAL_ERROR, "{}", response);
        let message = response["error"]["message"].as_str().expect("a message");
        let expected = format!(
            "BLOCKED (Lock 2): task {id} cannot complete without an extracted memory. Either call neurostrata_add_memory with metadata extracted_from: [\"{id}\"], or retry neurostrata_task_complete with `memory.content` (what did you learn? what rule does this task prove?) or `link_memory_id`."
        );
        assert_eq!(message, expected);

        let (_, payload) = store.get("probe", &id).await.unwrap().expect("the task");
        assert_eq!(
            payload.metadata["task"]["status"],
            serde_json::json!("open"),
            "the refused completion changes nothing"
        );
    }

    /// Section 6.1: the mandatory pre-flight carries the Zero-Action Start
    /// line first, before anything else in the snapshot body.
    #[tokio::test]
    async fn the_snapshot_opens_with_the_zero_action_start_line() {
        let (store, _) = store_with_one_rule("probe").await;
        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");
        assert!(
            text.starts_with("Zero-Action Start: claim or create a task before editing."),
            "{}",
            text
        );
        assert!(
            text.contains("always use podman"),
            "the body still follows: {}",
            text
        );
    }

    // ---- archived filter across the read surfaces -----------------------
    //
    // `archive_memory` writes the tombstone; the read surfaces must now hide
    // the row by default and surface it through `include_archived: true`. The
    // filter is the only thing that turns a tombstone into a tomb (otherwise
    // a retired row keeps showing up in every search until the operator
    // remembers to also delete it).
    //
    // Both reads run off the same `is_archived` helper, so each surface gets
    // its own pair of tests: a default call that excludes the row and an
    // `include_archived: true` call that surfaces it. The behavior must be
    // uniform across surfaces, or the operator cannot predict where a
    // tombstone will be honored.

    /// Seeds the fixture with one live row and one row tombstoned by
    /// `archive_memory`. The test owns the bus so the dispatcher's queue is
    /// drained by the time the read surfaces run.
    async fn store_with_live_and_archived(
        namespace: &str,
    ) -> (Arc<dyn VectorStore>, String, String) {
        let (store, live_id) = store_with_one_rule(namespace).await;

        let archived_id = uuid::Uuid::new_v4().to_string();
        let vector = StubEmbedder.embed("deprecated rule").await.unwrap();
        store
            .upsert(namespace, &archived_id, vector, payload("deprecated rule"))
            .await
            .expect("seed the row that will be archived");

        let bus = Arc::new(ThalamicBus::new(16));
        let outcome = crate::handlers::archive_memory::archive_memory(
            serde_json::json!({ "namespace": namespace, "id": archived_id }),
            store.clone(),
            bus,
        )
        .await;
        assert!(outcome.is_archived(), "the second row really was archived");

        (store, live_id, archived_id)
    }

    /// `handle_search_memory` hides tombstoned rows by default. The point of
    /// a tombstone is that it retires the row from the operator's recall, so
    /// a search that still returns the row defeats the operator's intent.
    #[tokio::test]
    async fn search_hides_archived_rows_by_default() {
        let (store, live_id, archived_id) = store_with_live_and_archived("probe").await;
        let text = handle_search_memory(
            serde_json::json!({ "namespace": "probe", "query": "podman" }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;
        assert!(text.contains(&live_id), "the live row surfaces: {text}");
        assert!(
            !text.contains(&archived_id),
            "the archived row is hidden by default: {text}"
        );
    }

    /// `include_archived: true` is the operator's escape hatch -- the row
    /// stays in the store, so an audit, a recovery, or a deliberate
    /// second-look has to be able to find it.
    #[tokio::test]
    async fn search_includes_archived_rows_when_include_archived_is_true() {
        let (store, live_id, archived_id) = store_with_live_and_archived("probe").await;
        let text = handle_search_memory(
            serde_json::json!({
                "namespace": "probe",
                "query": "podman",
                "include_archived": true
            }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;
        assert!(text.contains(&live_id), "the live row surfaces: {text}");
        assert!(
            text.contains(&archived_id),
            "include_archived surfaces the tombstoned row: {text}"
        );
    }

    /// `handle_get_snapshot` hides tombstoned rows by default. The snapshot
    /// is the operator's daily-start view; an archived row that still shows
    /// up there means the operator cannot trust the snapshot to mean "what
    /// is currently live in this namespace".
    #[tokio::test]
    async fn snapshot_hides_archived_rows_by_default() {
        let (store, _live_id, archived_id) = store_with_live_and_archived("probe").await;
        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");
        assert!(
            !text.contains(&archived_id),
            "the archived row is hidden by default: {text}"
        );
    }

    /// `include_archived: true` on the snapshot surfaces every row, live and
    /// tombstoned. Used for audits and for an operator verifying what
    /// `archive_memory` actually did.
    #[tokio::test]
    async fn snapshot_includes_archived_rows_when_include_archived_is_true() {
        let (store, _live_id, archived_id) = store_with_live_and_archived("probe").await;
        let text = handle_get_snapshot(
            serde_json::json!({ "namespace": "probe", "include_archived": true }),
            store,
        )
        .await
        .expect("snapshot");
        assert!(
            text.contains(&archived_id),
            "include_archived surfaces the tombstoned row: {text}"
        );
    }

    // ---- conflict suppression across the read surfaces ------------------
    //
    // A conflicting mesh row (`metadata.mesh.conflict == true`) is suppressed
    // from search/snapshot even under `include_archived: true` — an unresolved
    // conflict is never served until it is resolved, unlike a tombstone, which
    // has an audit escape hatch. Search and snapshot both ask the one shared
    // helper (`synaptic_mesh::conflict::is_conflicting`).

    /// Seeds one live row plus one row marked `metadata.mesh.conflict = true`.
    async fn store_with_live_and_conflicting(
        namespace: &str,
    ) -> (Arc<dyn VectorStore>, String, String) {
        let (store, live_id) = store_with_one_rule(namespace).await;

        let conflicting_id = uuid::Uuid::new_v4().to_string();
        let mut p = payload("always use podman, conflicting edit");
        p.metadata = serde_json::json!({
            "mesh": { "conflict": true, "conflict_reason": "incompatible content edits" }
        });
        let vector = StubEmbedder
            .embed("always use podman, conflicting edit")
            .await
            .unwrap();
        store
            .upsert(namespace, &conflicting_id, vector, p)
            .await
            .expect("seed the conflicting row");

        (store, live_id, conflicting_id)
    }

    #[tokio::test]
    async fn search_hides_conflicting_rows_even_with_include_archived() {
        let (store, live_id, conflicting_id) = store_with_live_and_conflicting("probe").await;
        let text = handle_search_memory(
            serde_json::json!({
                "namespace": "probe",
                "query": "podman",
                "include_archived": true
            }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;
        assert!(text.contains(&live_id), "the live row surfaces: {text}");
        assert!(
            !text.contains(&conflicting_id),
            "the conflicting row is suppressed from search: {text}"
        );
    }

    #[tokio::test]
    async fn snapshot_hides_conflicting_rows_even_with_include_archived() {
        let (store, _live_id, conflicting_id) = store_with_live_and_conflicting("probe").await;
        let text = handle_get_snapshot(
            serde_json::json!({ "namespace": "probe", "include_archived": true }),
            store,
        )
        .await
        .expect("snapshot");
        assert!(
            !text.contains(&conflicting_id),
            "the conflicting row is never served, even under include_archived: {text}"
        );
    }

    #[tokio::test]
    async fn the_snapshot_body_holds_no_registry_row() {
        let (store, _live_id) = store_with_one_rule("probe").await;
        let registry_id = crate::registry::registry_row_id("probe");
        let vector = StubEmbedder.embed("Domain registry for probe").await.unwrap();
        let mut p = payload("Domain registry for probe");
        p.memory_type = "fact".to_string();
        p.metadata = crate::registry::registry_metadata(&["auth".to_string()]);
        store
            .upsert("probe", &registry_id, vector, p)
            .await
            .expect("seed the registry row");

        let text = handle_get_snapshot(
            serde_json::json!({ "namespace": "probe" }),
            store,
        )
        .await
        .expect("snapshot");
        assert!(
            !text.contains(&registry_id),
            "the registry row is absent from the snapshot body: {text}"
        );
    }

    #[tokio::test]
    async fn a_fact_written_with_a_free_text_domain_is_recalled_by_its_slug_tag() {
        let dir = std::env::temp_dir().join(format!(
            "ns-recall-real-{}",
            uuid::Uuid::new_v4()
        ));
        // Create the parent directory for both the store and buffer
        std::fs::create_dir_all(&dir).expect("create parent dir");
        // Use a subdirectory path for the LadybugStore database (not created yet)
        // to avoid conflict with the buffer's .NeuroStrata directory.
        let store_dir = dir.join("store");
        let store: Arc<dyn VectorStore> =
            Arc::new(crate::store::LadybugStore::for_testing(&store_dir, 4).expect("open temp"));
        store.init("probe").await.expect("init");

        // Write a fact with free-text domain "Database Access" (normalizes to "database-access")
        let add_result = handle_add_memory(
            serde_json::json!({
                "namespace": "probe",
                "create_new_namespace": true,
                "memory_type": "fact",
                "content": "Use connection pooling for database access",
                "domain": "Database Access",
            }),
            Arc::new(StubEmbedder),
            store.clone(),
            None,
            quiet_bus(),
        )
        .await;
        assert!(add_result.starts_with("Successfully added memory"), "add failed: {}", add_result);
        // Extract the fact ID from the response
        let fact_id = add_result
            .lines()
            .find(|l| l.starts_with("Memory ID: "))
            .map(|l| l[11..].trim().to_string())
            .expect("fact ID in response");

        // Read access_count BEFORE recall (must be present and 0)
        let before = store
            .get("probe", &fact_id)
            .await
            .unwrap()
            .expect("fact must exist after add");
        let access_before = before
            .1
            .metadata
            .get("access_count")
            .expect("access_count must be present on a fact after add")
            .as_u64()
            .expect("access_count must be a number");
        assert_eq!(access_before, 0, "access_count must be 0 after add");

        // Now recall via append_log with the slug tag
        // Use the parent dir (not store_dir) for the buffer's project_root
        let recall_result = crate::buffer::handle_append_log(
            serde_json::json!({
                "content": "working on db",
                "project_root": dir.display().to_string(),
                "tags": ["database-access"],
                "namespace": "probe",
            }),
            Arc::new(StubEmbedder),
            store.clone(),
        )
        .await;

        let parsed: serde_json::Value = serde_json::from_str(&recall_result).unwrap();
        assert!(parsed.get("appended").and_then(|v| v.as_bool()).unwrap_or(false), "appended was false or missing: {}", recall_result);
        let recalled = parsed.get("recalled").expect("recalled field must be present");
        let recalled_arr = recalled.as_array().unwrap();
        assert_eq!(recalled_arr.len(), 1, "exactly one fact recalled: {}", recall_result);
        assert_eq!(
            recalled_arr[0].get("id").and_then(|v| v.as_str()),
            Some(fact_id.as_str())
        );
        assert_eq!(parsed.get("recalled_from").and_then(|v| v.as_str()), Some("domain"));

        // Read access_count AFTER recall (must be present and unchanged)
        let after = store
            .get("probe", &fact_id)
            .await
            .unwrap()
            .expect("fact must still exist after recall");
        let access_after = after
            .1
            .metadata
            .get("access_count")
            .expect("access_count must be present on a fact after recall")
            .as_u64()
            .expect("access_count must be a number");
        assert_eq!(access_after, 0, "access_count must still be 0 after recall");
        assert_eq!(access_after, access_before, "access_count must not change during recall");
    }

    /// The neurostrata_append_log tool's inputSchema must carry the optional
    /// `namespace` property (not in required).
    #[tokio::test]
    async fn append_log_tool_schema_carries_optional_namespace() {
        let (store, _) = store_with_one_rule("probe").await;
        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "tools/list".to_string(),
            params: Some(serde_json::json!({})),
        };
        let (bus, _seen) = recording_bus();
        let response = process_mcp_request(
            request,
            Arc::new(StubEmbedder),
            store,
            ingests,
            None,
            bus,
        )
        .await;
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools/list returns an array");
        let append_log = tools
            .iter()
            .find(|t| t["name"] == "neurostrata_append_log")
            .expect("neurostrata_append_log must be in tools/list");
        let props = append_log["inputSchema"]["properties"]
            .as_object()
            .expect("inputSchema.properties must be an object");
        assert!(props.contains_key("namespace"), "namespace property missing: {:?}", props);
        let required = append_log["inputSchema"]["required"]
            .as_array()
            .expect("required must be an array");
        let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(!required_names.contains(&"namespace"), "namespace must not be in required: {:?}", required_names);
    }

    /// End-to-end MCP path: tools/call → dispatch → handler → JSON with recalled.
    /// Uses the branch-tolerant pattern from disabled_mode_is_a_no_op.
    #[tokio::test]
    async fn append_log_mcp_call_returns_recalled_in_text_result() {
        let root = std::env::temp_dir().join(format!("ns-append-log-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("scratch project root");
        let store = Arc::new(MockStore::new());
        store.init("probe").await.unwrap();

        // Seed a fact to recall
        let fact_id = "fact-mcp";
        let payload = crate::traits::MemoryPayload {
            content: "Use connection pooling for database access".to_string(),
            user_id: "tester".to_string(),
            memory_type: "fact".to_string(),
            agent_name: None,
            location: String::new(),
            location_lines: String::new(),
            metadata: serde_json::json!({ "domain": "database-access" }),
        };
        store
            .upsert("probe", fact_id, vec![0.1, 0.2, 0.3, 0.4], payload)
            .await
            .unwrap();

        let ingests = Arc::new(crate::ingest_jobs::IngestJobs::new());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(7)),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "neurostrata_append_log",
                "arguments": {
                    "content": "working on db",
                    "project_root": root.display().to_string(),
                    "tags": ["database-access"],
                    "namespace": "probe",
                }
            })),
        };
        let (bus, _seen) = recording_bus();
        let response = process_mcp_request(
            request,
            Arc::new(StubEmbedder),
            store.clone(),
            ingests,
            None,
            bus,
        )
        .await;

        // The response shape is { result: { content: [{ type: "text", text: "..." }] } }
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("content[0].text must be a string");
        // Branch-tolerant: buffer may be disabled by machine config
        if text.contains("Episodic Buffer disabled") {
            assert!(!text.contains("\"appended\":true"), "disabled response must not claim success: {}", text);
        } else {
            let parsed: serde_json::Value = serde_json::from_str(text)
                .expect("handler returns JSON");
            assert!(parsed.get("appended").and_then(|v| v.as_bool()).unwrap_or(false));
            assert!(parsed.get("recalled").is_some(), "recalled must be present: {}", text);
            let recalled = parsed.get("recalled").unwrap().as_array().unwrap();
            assert_eq!(recalled.len(), 1);
            assert_eq!(recalled[0].get("id").and_then(|v| v.as_str()), Some(fact_id));
            assert_eq!(parsed.get("recalled_from").and_then(|v| v.as_str()), Some("domain"));
        }
    }

    /// The printed path carries the graph's own direction. The store's
    /// orientation fix (task neurostrata-ttmr) changes the one-hop line only
    /// for a directed relation -- RELATES_TO is drawn undirected there
    /// (src/server.rs:2468) -- so this pins an outgoing GOVERNS edge's text.
    #[tokio::test]
    async fn an_outgoing_directed_edge_is_printed_from_the_primary() {
        let dir =
            std::env::temp_dir().join(format!("ns-outgoing-render-{}", uuid::Uuid::new_v4()));
        let store = crate::store::ladybug::LadybugStore::for_testing(&dir, 4)
            .expect("open temp database");
        store.init("probe").await.expect("create the schema");

        // The target first: an edge target must exist before the row that
        // declares the edge, or the edge is skipped as a write-shape warning.
        store
            .upsert(
                "probe",
                "probe::target.rs",
                vec![0.1, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "a file".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "file".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();

        // The rule governs the file: the edge points from the rule to the file.
        store
            .upsert(
                "probe",
                "probe::rule-1",
                vec![1.1, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "a rule".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "rule".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({ "governs": ["probe::target.rs"] }),
                },
            )
            .await
            .unwrap();

        let results = store
            .search("probe", vec![1.1, 0.0, 0.0, 0.0], 1, false, false)
            .await
            .unwrap();
        let text = render_search_results(&results);
        assert!(
            text.contains("\"probe::rule-1\" -GOVERNS- \"probe::target.rs\""),
            "an outgoing edge must print from the primary: {}",
            text
        );
    }

    /// A two-hop path must be walked from the direct match, and its second
    /// edge must not be dropped: the neighbour the evidence is about has to
    /// appear in the printed path.
    ///
    /// The graph is
    ///
    ///   probe::primary -RELATES_TO-> probe::mid <-GOVERNS- probe::ruler
    ///
    /// so the printed path has to run from the direct match, through the
    /// intermediate, to the neighbour that governs the intermediate.
    #[tokio::test]
    async fn a_two_hop_path_is_printed_from_the_primary_to_the_neighbour() {
        let dir =
            std::env::temp_dir().join(format!("ns-two-hop-render-{}", uuid::Uuid::new_v4()));
        let store = crate::store::ladybug::LadybugStore::for_testing(&dir, 4)
            .expect("open temp database");
        store.init("probe").await.expect("create the schema");

        // The intermediate: an edge target must exist before the row that
        // declares the edge, or the edge is skipped as a write-shape warning.
        store
            .upsert(
                "probe",
                "probe::mid",
                vec![0.5, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the intermediate".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "fact".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();

        // The neighbour: it governs the intermediate, so it is two hops away.
        store
            .upsert(
                "probe",
                "probe::ruler",
                vec![0.2, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the neighbour".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "rule".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({ "governs": ["probe::mid"] }),
                },
            )
            .await
            .unwrap();

        // The direct match: one hop from the intermediate.
        store
            .upsert(
                "probe",
                "probe::primary",
                vec![1.1, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the direct match".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "fact".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({ "related_to": ["probe::mid"] }),
                },
            )
            .await
            .unwrap();

        let results = store
            .search("probe", vec![1.1, 0.0, 0.0, 0.0], 1, false, false)
            .await
            .unwrap();
        let text = render_search_results(&results);
        let line = text
            .lines()
            .find(|l| l.contains("Path [governs_2hop]:"))
            .expect("the neighbour must carry a two-hop path");
        // The intermediate is printed by both segments: a path segment is
        // rendered as a self-contained "from -relation- to" pair.
        assert_eq!(
            line,
            "Path [governs_2hop]: \"probe::primary\" --RELATES_TO-- \"probe::mid\" \
             \"probe::mid\" <-GOVERNS- \"probe::ruler\"",
            "the two-hop path must run from the direct match to the neighbour"
        );
    }

    /// An undirected relation is printed once, with a connector that says so.
    /// The relation's declared direction decides the connector, not the kind of
    /// path the relation appears in.
    #[tokio::test]
    async fn an_undirected_relation_is_printed_once_with_an_undirected_connector() {
        let dir =
            std::env::temp_dir().join(format!("ns-undirected-render-{}", uuid::Uuid::new_v4()));
        let store = crate::store::ladybug::LadybugStore::for_testing(&dir, 4)
            .expect("open temp database");
        store.init("probe").await.expect("create the schema");

        // The neighbour: an edge target must exist before the row that
        // declares the edge, or the edge is skipped as a write-shape warning.
        store
            .upsert(
                "probe",
                "probe::neighbour",
                vec![0.5, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the neighbour".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "fact".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();

        // The direct match: RELATES_TO is declared undirected in the vocabulary.
        store
            .upsert(
                "probe",
                "probe::primary",
                vec![1.1, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the direct match".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "fact".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({ "related_to": ["probe::neighbour"] }),
                },
            )
            .await
            .unwrap();

        let results = store
            .search("probe", vec![1.1, 0.0, 0.0, 0.0], 1, false, false)
            .await
            .unwrap();
        let text = render_search_results(&results);
        let line = text
            .lines()
            .find(|l| l.contains("Path [one_hop]:"))
            .expect("the neighbour must carry the one-hop path");
        assert_eq!(
            line,
            "Path [one_hop]: \"probe::primary\" --RELATES_TO-- \"probe::neighbour\"",
            "an undirected relation is printed once, with an undirected connector"
        );
    }

    /// The renderer walks the path from the origin. An edge the walk is not
    /// standing on is not part of it, so it is dropped rather than drawn as a
    /// segment the graph does not contain.
    #[test]
    fn a_path_edge_the_walk_is_not_standing_on_is_dropped() {
        let ev = SearchEvidence {
            kind: EvidenceKind::OneHop,
            path: vec![
                EvidenceEdge {
                    source: "probe::a".into(),
                    relation: "RELATES_TO".into(),
                    target: "probe::b".into(),
                },
                EvidenceEdge {
                    source: "probe::x".into(),
                    relation: "RELATES_TO".into(),
                    target: "probe::y".into(),
                },
            ],
            explanation: "a path with one disconnected edge".into(),
        };
        assert_eq!(
            render_path(&ev, "probe::a"),
            "\"probe::a\" --RELATES_TO-- \"probe::b\"",
            "the edge the walk is not standing on is dropped"
        );
    }

    /// The two-hop line shows the first hop's orientation when the relation
    /// is directed. The graph is
    ///
    ///   probe::primary -GOVERNS-> probe::mid <-GOVERNS- probe::ruler
    ///
    /// so the printed path runs from the direct match to the intermediate with
    /// a forward arrow and from the intermediate to the neighbour with a
    /// backward one -- the graph's own directions on both hops.
    #[tokio::test]
    async fn a_two_hop_path_with_a_directed_first_hop_shows_its_orientation() {
        let dir =
            std::env::temp_dir().join(format!("ns-two-hop-directed-{}", uuid::Uuid::new_v4()));
        let store = crate::store::ladybug::LadybugStore::for_testing(&dir, 4)
            .expect("open temp database");
        store.init("probe").await.expect("create the schema");

        // The intermediate: the target of both GOVERNS edges.
        store
            .upsert(
                "probe",
                "probe::mid",
                vec![0.5, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the intermediate".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "fact".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();

        // The neighbour: it governs the intermediate.
        store
            .upsert(
                "probe",
                "probe::ruler",
                vec![0.2, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the neighbour".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "fact".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({ "governs": ["probe::mid"] }),
                },
            )
            .await
            .unwrap();

        // The direct match: it governs the intermediate too.
        store
            .upsert(
                "probe",
                "probe::primary",
                vec![1.1, 0.0, 0.0, 0.0],
                MemoryPayload {
                    content: "the direct match".to_string(),
                    user_id: "tester".to_string(),
                    memory_type: "fact".to_string(),
                    agent_name: None,
                    location: String::new(),
                    location_lines: String::new(),
                    metadata: serde_json::json!({ "governs": ["probe::mid"] }),
                },
            )
            .await
            .unwrap();

        let results = store
            .search("probe", vec![1.1, 0.0, 0.0, 0.0], 1, false, false)
            .await
            .unwrap();
        let text = render_search_results(&results);
        let line = text
            .lines()
            .find(|l| l.contains("Path [governs_2hop]:"))
            .expect("the neighbour must carry the two-hop path");
        assert_eq!(
            line,
            "Path [governs_2hop]: \"probe::primary\" -GOVERNS- \"probe::mid\" \
             \"probe::mid\" <-GOVERNS- \"probe::ruler\"",

            "a directed first hop must show the graph's direction"
        );
    }

    // ── Snapshot domain strap (Task 6) ────────────────────────────────

    #[tokio::test]
    async fn the_domain_strap_shows_a_row_the_access_count_cut_would_hide() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());

        // Five high-access rows that fill the top-5.
        let mut high_ids = Vec::new();
        for i in 0..5 {
            let id = format!("high-{}", i);
            let mut p = payload(&format!("high access rule {}", i));
            p.metadata = serde_json::json!({ "access_count": 10 });
            store
                .upsert("probe", &id, vec![0.1, 0.2, 0.3, 0.4], p)
                .await
                .expect("seed high");
            high_ids.push(id);
        }

        // One new fact at access_count 0 with a domain: invisible to the top-5.
        let low_id = "low-domain".to_string();
        let mut low = payload("a fresh fact in the auth domain");
        low.memory_type = "fact".to_string();
        low.metadata = serde_json::json!({ "domain": "auth", "access_count": 0 });
        store
            .upsert("probe", &low_id, vec![0.5, 0.6, 0.7, 0.8], low)
            .await
            .expect("seed low");

        // Without an explicit domain argument (recent_domain mode).
        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store.clone())
            .await
            .expect("snapshot");
        for id in &high_ids {
            assert!(text.contains(id), "top-5 holds the high row: {text}");
        }
        assert!(text.contains(&low_id), "strap holds the low fact: {text}");

        // With an explicit domain argument (explicit mode).
        let text2 = handle_get_snapshot(
            serde_json::json!({ "namespace": "probe", "domain": "auth" }),
            store,
        )
        .await
        .expect("snapshot");
        assert!(text2.contains(&low_id), "strap holds the low fact (explicit): {text2}");
    }

    #[tokio::test]
    async fn a_domain_tagged_row_already_in_the_top_five_is_not_duplicated() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());

        // Five high-access rows, one of them domain-tagged.
        let domain_high_id = "high-domain".to_string();
        for i in 0..5 {
            let id = if i == 0 { domain_high_id.clone() } else { format!("high-{}", i) };
            let mut p = payload(&format!("high access rule {}", i));
            if i == 0 {
                p.metadata = serde_json::json!({ "domain": "auth", "access_count": 10 });
            } else {
                p.metadata = serde_json::json!({ "access_count": 10 });
            }
            store
                .upsert("probe", &id, vec![0.1, 0.2, 0.3, 0.4], p)
                .await
                .expect("seed high");
        }

        // A low domain row so the strap is non-empty.
        let low_id = "low-domain".to_string();
        let mut low = payload("a fresh fact");
        low.memory_type = "fact".to_string();
        low.metadata = serde_json::json!({ "domain": "auth", "access_count": 0 });
        store
            .upsert("probe", &low_id, vec![0.5, 0.6, 0.7, 0.8], low)
            .await
            .expect("seed low");

        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");

        // The high domain row is in the top-5 and must not appear again in the strap.
        assert_eq!(
            text.matches(&domain_high_id).count(),
            1,
            "a top-5 id is not duplicated into the strap: {text}"
        );
    }

    #[tokio::test]
    async fn the_snapshot_does_not_mutate_any_access_count() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());

        for i in 0..3 {
            let id = format!("row-{}", i);
            let mut p = payload(&format!("rule {}", i));
            p.metadata = serde_json::json!({ "access_count": i as i64 + 1, "domain": "auth" });
            store
                .upsert("probe", &id, vec![0.1, 0.2, 0.3, 0.4], p)
                .await
                .expect("seed");
        }

        let before: Vec<(String, i64)> = store
            .list("probe", None, None)
            .await
            .unwrap()
            .iter()
            .map(|r| {
                let c = r
                    .payload
                    .metadata
                    .get("access_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                (r.id.clone(), c)
            })
            .collect();

        let _ = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store.clone())
            .await
            .expect("snapshot");

        let after: Vec<(String, i64)> = store
            .list("probe", None, None)
            .await
            .unwrap()
            .iter()
            .map(|r| {
                let c = r
                    .payload
                    .metadata
                    .get("access_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                (r.id.clone(), c)
            })
            .collect();

        assert_eq!(before, after, "access counts are unchanged by the snapshot");
    }

    #[tokio::test]
    async fn the_strap_is_bounded_and_names_its_mode() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());

        // Five high-access rows fill the top-5.
        for i in 0..5 {
            let id = format!("high-{}", i);
            let mut p = payload(&format!("high {}", i));
            p.metadata = serde_json::json!({ "access_count": 10 });
            store
                .upsert("probe", &id, vec![0.1, 0.2, 0.3, 0.4], p)
                .await
                .expect("seed high");
        }

        // Four domain-tagged rows in one domain.
        for i in 0..4 {
            let id = format!("auth-{}", i);
            let mut p = payload(&format!("auth fact {}", i));
            p.memory_type = "fact".to_string();
            p.metadata = serde_json::json!({ "domain": "auth", "access_count": 0 });
            store
                .upsert("probe", &id, vec![0.5, 0.6, 0.7, 0.8], p)
                .await
                .expect("seed auth");
        }

        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");

        assert!(
            text.contains("Domain strap [mode: recent_domain]"),
            "mode is named: {text}"
        );
        // At most 3 strap entries: count "snippet" occurrences.
        let snippet_count = text.matches("\"snippet\"").count();
        assert!(snippet_count <= 3, "strap holds at most 3 entries: {text}");
        // At most 2 from the one domain.
        let domain_count = text.matches("\"domain\": \"auth\"").count();
        assert!(domain_count <= 2, "at most 2 from one domain: {text}");
    }

    #[tokio::test]
    async fn an_empty_strap_renders_an_explicit_zero() {
        let (store, _) = store_with_one_rule("probe").await;
        let text = handle_get_snapshot(
            serde_json::json!({ "namespace": "probe", "domain": "auth" }),
            store,
        )
        .await
        .expect("snapshot");
        assert!(
            text.contains("Domain strap [mode: explicit] (0): []"),
            "empty strap renders explicit zero: {text}"
        );
    }

    #[tokio::test]
    async fn the_snapshot_names_the_registered_domains() {
        let store: Arc<dyn VectorStore> = Arc::new(MockStore::new());
        let registry_id = crate::registry::registry_row_id("probe");
        let vector = StubEmbedder.embed("Domain registry for probe").await.unwrap();
        let mut p = payload("Domain registry for probe");
        p.memory_type = "fact".to_string();
        p.metadata = crate::registry::registry_metadata(&[
            "auth".to_string(),
            "database-access".to_string(),
            "storage".to_string(),
        ]);
        store
            .upsert("probe", &registry_id, vector, p)
            .await
            .expect("seed registry row");

        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");
        assert!(
            text.contains("Registered domains (3): auth, database-access, storage"),
            "registered domains are named: {text}"
        );
    }

    #[tokio::test]
    async fn a_namespace_with_no_registry_row_names_zero_domains() {
        let (store, _) = store_with_one_rule("probe").await;
        let text = handle_get_snapshot(serde_json::json!({ "namespace": "probe" }), store)
            .await
            .expect("snapshot");
        assert!(
            text.contains("Registered domains (0): none"),
            "zero domains named: {text}"
        );
    }

    #[tokio::test]
    async fn a_domain_filtered_search_counts_the_filter() {
        let _guard = crate::recall_metrics::acquire_test_lock();
        let (store, _auth, _data) = store_with_two_domains("probe-counts").await;
        let before_total = crate::recall_metrics::domain_filter_snapshot()["domain_queries_total"]
            .as_u64()
            .unwrap();
        let _ = handle_search_memory(
            serde_json::json!({ "namespace": "probe-counts", "query": "rules", "domain": "auth" }),
            Arc::new(StubEmbedder),
            store.clone(),
        )
        .await;
        let after_total = crate::recall_metrics::domain_filter_snapshot()["domain_queries_total"]
            .as_u64()
            .unwrap();
        assert_eq!(after_total, before_total + 1);

        let before_none = crate::recall_metrics::domain_filter_snapshot()["domain_queries_total"]
            .as_u64()
            .unwrap();
        let _ = handle_search_memory(
            serde_json::json!({ "namespace": "probe-counts", "query": "rules" }),
            Arc::new(StubEmbedder),
            store,
        )
        .await;
        let after_none = crate::recall_metrics::domain_filter_snapshot()["domain_queries_total"]
            .as_u64()
            .unwrap();
        assert_eq!(after_none, before_none);
    }

    #[tokio::test]
    async fn a_search_in_a_single_domain_namespace_counts_a_bypass_not_a_narrowing_hit() {
        let _guard = crate::recall_metrics::acquire_test_lock();
        let (store, _) = store_with_one_rule("single-domain-ns").await;
        let before_total = crate::recall_metrics::domain_filter_snapshot()["domain_queries_total"]
            .as_u64()
            .unwrap();
        let before_bypass = crate::recall_metrics::domain_filter_snapshot()["single_domain_bypasses"]
            .as_u64()
            .unwrap();
        let before_narrowed = crate::recall_metrics::domain_filter_snapshot()["domain_narrowed_hits"]
            .as_u64()
            .unwrap();

        // filter that does not match -> Vector mode, no narrowing hit
        let _ = handle_search_memory(
            serde_json::json!({ "namespace": "single-domain-ns", "query": "rules", "domain": "no-such-domain" }),
            Arc::new(StubEmbedder),
            store.clone(),
        )
        .await;
        let after_total = crate::recall_metrics::domain_filter_snapshot()["domain_queries_total"]
            .as_u64()
            .unwrap();
        let after_bypass = crate::recall_metrics::domain_filter_snapshot()["single_domain_bypasses"]
            .as_u64()
            .unwrap();
        let after_narrowed = crate::recall_metrics::domain_filter_snapshot()["domain_narrowed_hits"]
            .as_u64()
            .unwrap();
        assert_eq!(after_total, before_total + 1);
        assert_eq!(after_bypass, before_bypass + 1);
        assert_eq!(after_narrowed, before_narrowed);
    }

    #[tokio::test]
    async fn a_promotion_of_a_recalled_id_is_counted() {
        let _guard = crate::recall_metrics::acquire_test_lock();
        let (store, id) = store_with_one_rule("promo-ns").await;
        crate::recall_metrics::record_recall("promo-ns", &[id.clone()]);
        let before = crate::recall_metrics::recall_snapshot()["recall_promotions"]
            .as_u64()
            .unwrap();
        let _ = handle_get_memory(
            serde_json::json!({ "namespace": "promo-ns", "id": id }),
            store,
        )
        .await;
        let after = crate::recall_metrics::recall_snapshot()["recall_promotions"]
            .as_u64()
            .unwrap();
        assert_eq!(after, before + 1);
    }

}
