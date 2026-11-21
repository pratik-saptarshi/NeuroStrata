//! Shared request handler for `neurostrata_taxonomy` (MCP), `POST /taxonomy` (Daemon),
//! and `neurostrata-mcp taxonomy` (CLI).

use crate::events::{
    MemorySubscriber, RecursionToken, SubscriberContext, ThalamicBus, ThalamicPulse,
};
use crate::rdf::action::ActionSecurityContext;
use crate::rdf::taxonomy::{execute_taxonomy, TaxonomyRequest};
use crate::store::ladybug::LadybugStore;
use crate::traits::{Embedder, VectorStore};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

/// Field names accepted by `TaxonomyRequest` (src/rdf/taxonomy.rs): every
/// struct field name plus every serde alias (`operation` also accepts
/// `action`, `aliases` also accepts `alt_labels`, `related_concepts` also
/// accepts `related`). This list is maintained by hand against the struct
/// definition and can drift when fields are added or renamed there —
/// drift only weakens the unknown-field debug log below (a new field
/// would be reported as unknown until this list is updated); it never
/// changes request handling.
const KNOWN_TAXONOMY_FIELDS: &[&str] = &[
    "access_policy",
    "action",
    "adjacent_domains",
    "aliases",
    "alt_labels",
    "axes",
    "broader",
    "comparable_implementations",
    "concept_iri",
    "content",
    "contrasts_with",
    "core_pillars",
    "current_limitations",
    "definition",
    "deprecation",
    "direction",
    "domain",
    "eligibility",
    "evaluation_axes",
    "evaluation_scores",
    "evidence_refs",
    "extracted_from",
    "frontier_opportunities",
    "governs",
    "implemented_interfaces",
    "limit",
    "linked_action_types",
    "locations",
    "max_hops",
    "mission",
    "namespace",
    "non_goals",
    "not_definition",
    "operation",
    "pref_label",
    "primary_domain",
    "principal",
    "query",
    "realization_status",
    "references",
    "related",
    "related_concepts",
    "related_to",
    "root_slug",
    "slug",
    "slugs",
    "top_k",
    "topic",
    "tradeoffs",
    "visible_to",
];

/// Top-level keys of `arguments` that `TaxonomyRequest` does not recognise.
/// `TaxonomyRequest` defaults every field, so unknown keys are silently
/// dropped by the `from_value` conversion; this helper exists only to make
/// those drops visible in the debug log (fork #124, C3). Returns the sorted
/// unknown names for an object payload, and an empty vec otherwise.
pub fn unknown_taxonomy_fields(arguments: &Value) -> Vec<String> {
    let Some(obj) = arguments.as_object() else {
        return Vec::new();
    };
    let mut unknown: Vec<String> = obj
        .keys()
        .filter(|k| !KNOWN_TAXONOMY_FIELDS.contains(&k.as_str()))
        .cloned()
        .collect();
    unknown.sort();
    unknown
}

/// Synchronously tee a taxonomy mutation pulse into the project's durable
/// SynapticMesh outbox when running in CLI fallback mode (`bus == None`).
pub async fn tee_taxonomy_pulse_to_project_outbox(
    project_root: &Path,
    store: &dyn VectorStore,
    namespace: &str,
    pulse: &ThalamicPulse,
) {
    let Ok(Some((_mesh_id, mesh_dir, _remote))) =
        crate::synaptic_mesh::storage::MeshStorage::read_pointer(project_root)
    else {
        return;
    };
    let Ok(mesh) = crate::synaptic_mesh::storage::MeshStorage::open(&mesh_dir) else {
        return;
    };
    let outbox_dir = crate::synaptic_mesh::outbox::outbox_dir_for(&mesh_dir);
    let wake = Arc::new(tokio::sync::Notify::new());
    let Ok(sub) = crate::events::subscribers::MeshOutbox::new(
        outbox_dir,
        wake,
        mesh.manifest().epoch,
    ) else {
        return;
    };
    let ctx = SubscriberContext {
        store,
        namespace,
        ts: chrono::Utc::now().timestamp(),
    };
    let _ = sub.handle(pulse, &RecursionToken::root(), &ctx).await;
}

/// Dispatch a `neurostrata_taxonomy` request with default local/MCP `"admin"` principal.
pub async fn handle_taxonomy(
    arguments: &Value,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
) -> String {
    handle_taxonomy_with_auth_and_bus(arguments, "admin", emb, store, None).await
}

/// Dispatch a `neurostrata_taxonomy` request with default `"admin"` principal and optional `ThalamicBus`.
pub async fn handle_taxonomy_with_bus(
    arguments: &Value,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
    bus: Option<Arc<ThalamicBus>>,
) -> String {
    handle_taxonomy_with_auth_and_bus(arguments, "admin", emb, store, bus).await
}

/// Dispatch a `neurostrata_taxonomy` request with an explicit authenticated principal and optional `ThalamicBus`.
pub async fn handle_taxonomy_with_auth_and_bus(
    arguments: &Value,
    authenticated_principal: &str,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
    bus: Option<Arc<ThalamicBus>>,
) -> String {
    let Some(ns_raw) = arguments
        .get("namespace")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    else {
        return json!({"ok": false, "error": "namespace required"}).to_string();
    };

    let namespace = crate::server::resolve_namespace(&store, ns_raw).await;
    let Some(lb_store) =
        crate::traits::VectorStore::as_any(store.as_ref()).downcast_ref::<LadybugStore>()
    else {
        return json!({
            "ok": false,
            "error": "subject-matter taxonomy requires LadybugStore backend"
        })
        .to_string();
    };

    let unknown_fields = unknown_taxonomy_fields(arguments);
    if !unknown_fields.is_empty() {
        tracing::debug!(
            "taxonomy request carries unknown fields that will be ignored: {}",
            unknown_fields.join(", ")
        );
    }

    let mut req: TaxonomyRequest = match serde_json::from_value(arguments.clone()) {
        Ok(r) => r,
        Err(e) => {
            return json!({
                "ok": false,
                "error": format!("invalid taxonomy request payload: {e}")
            })
            .to_string();
        }
    };
    req.namespace = namespace.clone();
    if req.operation.trim().is_empty() {
        return json!({
            "ok": false,
            "error": "operation required ('set_thesis', 'upsert_concept', 'tree', 'briefing', 'compare', 'gaps', or 'advise')"
        })
        .to_string();
    }

    let requested_principal = req.principal.clone().or_else(|| {
        arguments
            .get("principal")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    let resolved_principal =
        crate::rebac::resolve_principal(authenticated_principal, requested_principal.as_deref());
    let security = ActionSecurityContext {
        authenticated_principal: authenticated_principal.to_string(),
        requested_principal: Some(resolved_principal),
    };

    let hint = req
        .query
        .clone()
        .or_else(|| req.topic.clone())
        .or_else(|| req.slug.clone())
        .or_else(|| req.domain.clone())
        .unwrap_or_else(|| req.operation.clone());

    match execute_taxonomy(lb_store, emb.as_ref(), req, &security).await {
        Ok(val) => {
            let op = val.get("operation").and_then(|v| v.as_str()).unwrap_or("");
            let pulse_opt = if op == "set_thesis" {
                val.get("thesis_id").and_then(|v| v.as_str()).map(|new_id| {
                    if let Some(old_id) = val.get("superseded_id").and_then(|v| v.as_str()) {
                        ThalamicPulse::Superseded {
                            old_id: old_id.to_string(),
                            new_id: new_id.to_string(),
                            namespace: namespace.clone(),
                        }
                    } else {
                        ThalamicPulse::Created {
                            id: new_id.to_string(),
                            namespace: namespace.clone(),
                            kind: "thesis".to_string(),
                        }
                    }
                })
            } else if op == "upsert_concept" {
                val.get("concept_id")
                    .and_then(|v| v.as_str())
                    .map(|new_id| {
                        if let Some(old_id) = val.get("superseded_id").and_then(|v| v.as_str()) {
                            ThalamicPulse::Superseded {
                                old_id: old_id.to_string(),
                                new_id: new_id.to_string(),
                                namespace: namespace.clone(),
                            }
                        } else {
                            ThalamicPulse::Created {
                                id: new_id.to_string(),
                                namespace: namespace.clone(),
                                kind: "concept".to_string(),
                            }
                        }
                    })
            } else {
                None
            };
            if let Some(pulse) = pulse_opt {
                if let Some(ref b) = bus {
                    b.emit(pulse, &RecursionToken::root());
                } else {
                    let project_root = crate::daemon::resolve_project_root();
                    tee_taxonomy_pulse_to_project_outbox(
                        &project_root,
                        store.as_ref(),
                        &namespace,
                        &pulse,
                    )
                    .await;
                }
            }
            crate::task::record_graph_query(&store, &namespace, "neurostrata_taxonomy", &hint)
                .await;
            serde_json::to_string_pretty(&val)
                .unwrap_or_else(|e| json!({"ok": false, "error": e.to_string()}).to_string())
        }
        Err(err) => json!({
            "ok": false,
            "error": err,
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_returns_sorted_unknown_names_only() {
        let args = json!({
            "namespace": "demo",
            "operation": "tree",
            "max_hops": 2,
            "action": "tree",
            "alt_labels": ["x"],
            "zz_typo_field": true,
            "bogus_key": 1,
        });
        assert_eq!(
            unknown_taxonomy_fields(&args),
            vec!["bogus_key".to_string(), "zz_typo_field".to_string()]
        );
    }

    #[test]
    fn unknown_fields_empty_for_all_known_payload() {
        let args = json!({
            "namespace": "demo",
            "operation": "compare",
            "slugs": ["a", "b"],
            "axes": ["determinism"],
            "related": ["c"],
        });
        assert!(unknown_taxonomy_fields(&args).is_empty());
    }

    #[test]
    fn unknown_fields_empty_for_non_object() {
        assert!(unknown_taxonomy_fields(&json!("tree")).is_empty());
        assert!(unknown_taxonomy_fields(&json!(["operation"])).is_empty());
        assert!(unknown_taxonomy_fields(&Value::Null).is_empty());
    }
}
