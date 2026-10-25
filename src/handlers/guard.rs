//! Guard tool handlers (the Prefrontal Cortex surface), extracted
//! from server.rs (Task 20). One implementation for the guard family,
//! called by `process_mcp_request` through `dispatch_guard_tool`.

use crate::events::{RecursionToken, ThalamicBus, ThalamicPulse};
use crate::guard::{BehavioralRule, GuardValidator, ValidateResponse};
use crate::traits::{Embedder, VectorStore};
use serde_json::Value;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Guard / Prefrontal Cortex tools (task neurostrata-fv3j)
//
// The src/guard/* module was folded into NeuroStrata as the in-binary
// replacement for the retired NeuroCortex MCP. These handlers expose the
// three reads/writes an in-session agent needs without going through the
// daemon's HTTP /validate surface:
//   neurostrata_guard_validate    -> wraps validator::validate
//   neurostrata_guard_learn       -> wraps validator::add_rule
//   neurostrata_guard_list_rules  -> wraps validator::list_rules
// Per-call construction (one validator per MCP request) keeps the surface
// stateless for the proxy; the daemon's prebuilt cache is the hot path.
// ---------------------------------------------------------------------------

/// Build a `GuardValidator` for the namespace named in the request.
/// Defaults namespace to "global" so the tool works in the common case
/// (rules are usually project-agnostic at the prefront-cortex layer) and
/// surfaces a precise error when required arguments are missing.
async fn build_guard_validator(
    arguments: &Value,
    store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
    what: &str,
) -> Result<(GuardValidator, String), String> {
    let namespace = arguments
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    // Validate namespace shape the same way handle_get_memory does, so the
    // guard surface inherits the same project-vs-path rejection contract.
    if namespace.contains('/') || namespace.contains('\\') || namespace.is_empty() {
        return Err(format!(
            "ERROR [NAMESPACE]: '{}' is not a valid namespace for guard_{}. \
             Namespaces are exact project names (e.g., 'NeuroStrata') or 'global'.",
            namespace, what
        ));
    }
    let validator = GuardValidator::new(store, embedder, namespace).await;
    Ok((validator, namespace.to_string()))
}

/// Validate an action against stored behavioral rules and (optionally) the
/// Podman sandbox. Wraps `GuardValidator::validate`.
pub async fn handle_guard_validate(
    arguments: Value,
    store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
    bus: Arc<ThalamicBus>,
) -> String {
    let action_type = match arguments.get("action_type").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return "ERROR: 'action_type' is required for guard_validate.".to_string(),
    };
    let payload = match arguments.get("payload").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return "ERROR: 'payload' is required for guard_validate.".to_string(),
    };
    let cwd = arguments.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
    let use_sandbox = arguments
        .get("use_sandbox")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (validator, namespace) =
        match build_guard_validator(&arguments, store.clone(), embedder, "validate").await {
            Ok(v) => v,
            Err(e) => return e,
        };

    // Zero-Action Start (honest): in a task-enabled namespace, a mutating
    // action requires at least one claimed (in_progress) task. Read-only
    // actions and namespaces with no task rows at all are exempt.
    let mutating = !matches!(
        action_type.to_lowercase().as_str(),
        "read" | "read_file" | "get" | "list" | "search" | "query" | "view" | "cat" | "inspect"
    );
    if mutating {
        // Determine whether this namespace is task-enabled.
        if let Ok(memories) = store.list(&namespace, None, None).await {
            let has_any_task = memories.iter().any(|m| crate::task::gate::is_task(m));
            if has_any_task {
                let has_in_progress = memories
                    .iter()
                    .any(|m| crate::task::gate::is_task(m) && crate::task::gate::task_status(m) == crate::task::machine::Status::InProgress);
                if !has_in_progress {
                    let trace_id = format!(
                        "guard-validate-{}-{}",
                        std::process::id(),
                        chrono::Utc::now().timestamp_millis()
                    );
                    let body = serde_json::json!({
                        "trace_id": trace_id,
                        "verdict": {
                            "bucket": "DeterministicReject",
                            "reasons": ["Zero-Action Start: no claimed (in_progress) task in this namespace. Claim or create a task before editing."],
                            "constraints": []
                        },
                        "rule_ids_triggered": []
                    });
                    return format!(
                        "namespace: {}\ntrace_id: {}\n{}",
                        namespace,
                        trace_id,
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                }
            }
        }
    }

    let trace_id = format!(
        "guard-validate-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    let (verdict, rule_ids_triggered) = validator
        .validate(action_type, payload, cwd, use_sandbox)
        .await;

    let response = ValidateResponse {
        trace_id: trace_id.clone(),
        verdict,
        rule_ids_triggered,
    };

    // Emit site 4. The verdict's own `bucket` tag, read back off the serialized
    // response rather than re-derived from the enum: the audit row then records
    // exactly what the caller was told, and the two cannot drift apart.
    //
    // The payload travels as a digest, never as itself -- a guard call carries
    // file contents and shell arguments, and this pulse is the thing most
    // likely to be logged and exported.
    let body = serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {}\"}}", e));
    if let Ok(value) = serde_json::from_str::<Value>(&body) {
        let verdict_bucket = value["verdict"]["bucket"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let rules: Vec<String> = value["rule_ids_triggered"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        bus.emit(
            ThalamicPulse::GuardedActionFired {
                trace_id: trace_id.clone(),
                action_type: action_type.to_string(),
                payload_hash: fnv1a_32(payload.as_bytes()),
                verdict: verdict_bucket,
                rule_ids_triggered: rules,
                namespace: namespace.clone(),
            },
            &RecursionToken::root(),
        );
    }

    // Prefix the namespace + trace so the caller can audit which rule set
    // produced the verdict without re-parsing the body.
    format!("namespace: {}\ntrace_id: {}\n{}", namespace, trace_id, body)
}

/// FNV-1a, 32-bit, over the action payload.
///
/// Hand-rolled rather than `std`'s `DefaultHasher` for the reason
/// `daemon::exe_fingerprint` already states: std's hasher makes no stability
/// promise across releases, and an audit commitment that silently changes value
/// between toolchain versions cannot be compared to a row written last week.
/// A collision costs one audit row's precision, never its existence.
fn fnv1a_32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in bytes {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Teach a new behavioral rule to the guard namespace. Wraps
/// `GuardValidator::add_rule`.
pub async fn handle_guard_learn(
    arguments: Value,
    store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
) -> String {
    let rule_value = match arguments.get("rule").cloned() {
        Some(v) => v,
        None => return "ERROR: 'rule' is required for guard_learn.".to_string(),
    };
    let rule: BehavioralRule = match serde_json::from_value(rule_value) {
        Ok(r) => r,
        Err(e) => return format!("ERROR [RULE SHAPE]: {}; required: id, version, rule_class, trigger_pattern, constraint_text, hit_count, status", e),
    };
    let (validator, namespace) =
        match build_guard_validator(&arguments, store, embedder, "learn").await {
            Ok(v) => v,
            Err(e) => return e,
        };
    match validator.add_rule(rule.clone()).await {
        Ok(()) => serde_json::to_string_pretty(&serde_json::json!({
            "learned": true,
            "namespace": namespace,
            "id": rule.id,
            "rule_class": rule.rule_class,
        }))
        .unwrap_or_else(|e| format!("{{\"error\":\"serialization: {}\"}}", e)),
        Err(e) => format!("ERROR [LEARN]: {}", e),
    }
}

/// List all behavioral rules in the namespace. Wraps
/// `GuardValidator::list_rules`.
pub async fn handle_guard_list_rules(
    arguments: Value,
    store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
) -> String {
    let (validator, namespace) =
        match build_guard_validator(&arguments, store, embedder, "list_rules").await {
            Ok(v) => v,
            Err(e) => return e,
        };
    match validator.list_rules().await {
        Ok(rules) => {
            let body = serde_json::to_string_pretty(&rules)
                .unwrap_or_else(|e| format!("{{\"error\":\"serialization: {}\"}}", e));
            format!(
                "{{\"namespace\":\"{}\",\"count\":{},\"rules\":{}}}",
                namespace,
                rules.len(),
                body
            )
        }
        Err(e) => format!("ERROR [LIST_RULES]: {}", e),
    }
}

/// Claim one tool name for the guard family. `None` means the name is
/// not this family's; `Some(Ok(text))` carries the result text (see
/// `super::ToolOutcome`).
pub async fn dispatch_guard_tool(
    name: &str,
    arguments: &Value,
    ctx: &super::ToolContext,
) -> Option<super::ToolOutcome> {
    let text = match name {
        "neurostrata_guard_validate" => {
            handle_guard_validate(
                arguments.clone(),
                ctx.store.clone(),
                ctx.emb.clone(),
                ctx.bus.clone(),
            )
            .await
        }
        "neurostrata_guard_learn" => {
            handle_guard_learn(arguments.clone(), ctx.store.clone(), ctx.emb.clone()).await
        }
        "neurostrata_guard_list_rules" => {
            handle_guard_list_rules(arguments.clone(), ctx.store.clone(), ctx.emb.clone()).await
        }
        _ => return None,
    };
    Some(Ok(text))
}
