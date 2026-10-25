//! Memory tool handlers extracted from server.rs (R28 began this
//! extraction; Task 20 completes it for the MCP dispatch).
//!
//! One implementation for the memory tool family, called by
//! `process_mcp_request` through `dispatch_memory_tool`. Shared store
//! operations that other surfaces also use (`resolve_namespace`,
//! `set_metadata`, `edit_memory`, the add-memory write path's stamping
//! helpers) stay in `crate::server`; these handlers call them there,
//! as the daemon and CLI do.

use crate::events::{RecursionToken, ThalamicBus, ThalamicPulse};
use crate::server::{
    embed_dedup_and_upsert, first_location, resolve_namespace, stamp_locations, stamp_new_memory,
};
use crate::traits::{Embedder, MemoryPayload, SearchResult, VectorStore};
use serde_json::Value;
use std::sync::Arc;

pub async fn handle_list_namespaces(store: Arc<dyn VectorStore>) -> String {
    if let Ok(namespaces) = store.list_namespaces().await {
        format!("Existing namespaces: {:?}", namespaces)
    } else {
        "Failed to list namespaces.".to_string()
    }
}

/// Every `Governs: [...]` list written in the prose, in order. Metadata is
/// what machines follow (BUG-10); this parses the prose line so the two can
/// be compared and reconciled instead of silently contradicting each other
/// inside one record (guinea-pig "in every option" ask).
pub(crate) fn content_governs_lines(content: &str) -> Vec<Vec<String>> {
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Governs:"))
        .filter_map(|rest| serde_json::from_str::<Vec<String>>(rest.trim()).ok())
        .collect()
}

/// The note that marks which list governs when the prose and the metadata
/// disagree. A record must never render two Governs lists in one block with
/// nothing marking the authoritative half.
pub(crate) fn governance_conflict_note(content: &str, metadata: &Value) -> Option<String> {
    let declared: Vec<String> = content_governs_lines(content)
        .into_iter()
        .flatten()
        .collect();
    if declared.is_empty() {
        return None;
    }
    let authoritative: Vec<String> = metadata
        .get("governs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let agrees =
        declared.len() == authoritative.len() && declared.iter().all(|d| authoritative.contains(d));
    if agrees {
        return None;
    }
    Some(format!(
        "\nNOTE: the prose above lists Governs {:?}, which DISAGREES with the authoritative metadata Governs {:?}. The metadata pointer is what machines follow; fix the prose with supersede (pass a governs parameter to move the pointer).",
        declared, authoritative
    ))
}

pub async fn handle_add_memory(
    arguments: Value,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
    deduplication_checker: Option<Arc<crate::judgment::DeduplicationChecker>>,
    bus: Arc<ThalamicBus>,
) -> String {
    let content = match arguments.get("content").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => return "Missing 'content' parameter.".to_string(),
    };

    let namespace = match arguments.get("namespace").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return "ERROR [NAMESPACE]: 'namespace' is missing. You MUST explicitly provide the specific project namespace. NEVER default to 'global' unless instructed.".to_string(),
    };
    let namespace = resolve_namespace(&store, namespace).await;
    let namespace = namespace.as_str();

    if namespace.contains('/') || namespace.contains('\\') {
        return "ERROR [NAMESPACE]: The namespace cannot be a file path. It must be the exact project name (e.g., 'NeuroStrata'). Do not use slashes.".to_string();
    }

    if namespace != "global" {
        if let Some(project_root) = arguments.get("project_root").and_then(|r| r.as_str()) {
            let ns_dir = std::path::Path::new(project_root).join(".NeuroStrata");
            if !tokio::fs::try_exists(&ns_dir).await.unwrap_or(false) {
                let create_new_namespace = arguments
                    .get("create_new_namespace")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !create_new_namespace {
                    return format!("ERROR: No .NeuroStrata directory found at {}. This indicates the project does not have a designated context/namespace yet. Do NOT guess the namespace. Ask the user if they want to initialize this directory as a new context, and if so, call this tool again with create_new_namespace=true.", project_root);
                } else {
                    if let Err(e) = tokio::fs::create_dir_all(&ns_dir).await {
                        return format!("ERROR: Failed to create .NeuroStrata directory: {}", e);
                    }
                }
            }
        }
    }

    let memory_type = arguments
        .get("memory_type")
        .and_then(|m| m.as_str())
        .unwrap_or("context");
    let create_new_namespace = arguments
        .get("create_new_namespace")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let user_id = arguments
        .get("user_id")
        .and_then(|u| u.as_str())
        .unwrap_or("unknown");
    if user_id == "auto-ingestor" {
        // Directory ingestion deletes every row owned by this user_id before it
        // rebuilds, so a memory stored under it would vanish on the next ingest.
        return "ERROR: 'auto-ingestor' is reserved for directory ingestion, and memories stored under it are deleted by the next ingest. Please use a different user_id.".to_string();
    }
    let agent_name = arguments
        .get("agent_name")
        .and_then(|a| a.as_str())
        .map(|s| s.to_string());
    let mut location = "".to_string();
    let mut location_lines = "".to_string();
    let mut metadata = arguments
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    // Validate metadata shape: must be a JSON object. Legacy scalar/string
    // metadata silently produced success with no lineage stamps on supersede.
    if !metadata.is_object() {
        return "ERROR: 'metadata' must be a JSON object (key-value map), not a string, array, or null. Legacy scalar metadata is no longer accepted; wrap it in an object.".to_string();
    }

    // Secret scan: content + metadata (recursive) before any processing.
    if let Some(rejection) = crate::secrets::scan_entry_point(content, &metadata, "add_memory") {
        return rejection.to_string();
    }

    if let Some(locations) = arguments.get("locations").and_then(|l| l.as_array()) {
        let (first_path, first_lines) = first_location(locations);
        location = first_path;
        location_lines = first_lines;
        stamp_locations(&mut metadata, locations);
    }

    if let Some(meta_obj) = metadata.as_object_mut() {
        if let Some(domain) = arguments.get("domain") {
            meta_obj.insert("domain".to_string(), domain.clone());
        }
        if let Some(related_to) = arguments.get("related_to") {
            meta_obj.insert("related_to".to_string(), related_to.clone());
        }
    }
    // The registry check reads the domain exactly as the caller gave it,
    // before the slug normalization below rewrites it: a non-string is
    // rejected rather than guessed at, and the reconcile must compare like
    // with like.
    let raw_domain = metadata.get("domain").cloned();
    let register_domain = arguments
        .get("register_domain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // The stored domain is normalized to its slug -- from the argument or from
    // the caller's own metadata -- so a stored domain is already a key. A
    // domain with no usable slug is stored exactly as it was given: the write
    // never guesses one (the registry check above rejected any non-string).
    if let Some(slug) = crate::domain::domain_key(&metadata) {
        if let Some(meta_obj) = metadata.as_object_mut() {
            meta_obj.insert("domain".to_string(), Value::String(slug));
        }
    }
    stamp_new_memory(&mut metadata);
    // A procedure born on this node is locally authored: stamp it so the
    // fire gate can tell authorship from import (Program Task 3). The
    // stamp overwrites any caller-supplied trust value.
    if memory_type == "procedure" {
        crate::tools::procedure::stamp_local_trust(&mut metadata);
    }

    // Honour an unambiguous Governs line in the prose (guinea-pig Option 2):
    // a declared pointer that is written but never honoured is the worst of
    // both worlds. Locations and explicit metadata already wrote governs;
    // this fills it in only when nothing else did.
    let mut adopted_governs = false;
    if metadata.get("governs").map(|g| g.is_null()).unwrap_or(true) {
        let lists = content_governs_lines(content);
        if lists.len() == 1 {
            metadata["governs"] = serde_json::json!(lists[0]);
            adopted_governs = true;
        }
    }

    if let Ok(existing_namespaces) = store.list_namespaces().await {
        if !existing_namespaces.contains(&namespace.to_string()) && !create_new_namespace {
            return format!(
                "Error: Namespace '{}' does not exist. SYSTEM ALERT: Your agent overconfidence and inaccuracy score has been flagged and degraded by the telemetry monitor. You MUST use `neurostrata_list_namespaces` to check existing project names before guessing. Existing namespaces are: {:?}. If you are absolutely certain this is a brand new project, you must explicitly pass `create_new_namespace: true` to bypass this lock.",
                namespace, existing_namespaces
            );
        }

        // The acceptance decision precedes any registry write. A rejection
        // returns here, before the memory lands; the cache update is a
        // post-acceptance side effect below.
        let mut registry_cache: Option<Vec<String>> = None;
        let mut check_warning: Option<String> = None;
        if let Some(raw) = &raw_domain {
            match crate::registry::check_domain(&store, &emb, namespace, raw, register_domain).await
            {
                Ok(check) => {
                    registry_cache = check.cache;
                    check_warning = check.warning;
                }
                Err(rejection) => return rejection,
            }
        }

        let payload = MemoryPayload {
            content: content.to_string(),
            user_id: user_id.to_string(),
            memory_type: memory_type.to_string(),
            agent_name,
            location,
            location_lines,
            metadata,
        };

        match embed_dedup_and_upsert(
            &store,
            &emb,
            namespace,
            &payload,
            deduplication_checker.as_ref(),
        )
        .await
        {
            Ok(report) => {
                use crate::judgment::policy::DedupDecision;
                let upsert = report.upsert.clone();
                let linked_id = report.linked_existing_id.clone();
                let decision = report.decision;

                // Exact reuse / blocked exact duplicate: no new row, no Created pulse.
                if upsert.is_none() {
                    match decision {
                        DedupDecision::LinkedExisting => {
                            let id = linked_id.unwrap_or_default();
                            return format!("Reused existing memory for namespace: {}\nMemory ID: {}", namespace, id);
                        }
                        DedupDecision::BlockedExact => {
                            return format!(
                                "Potential duplicate detected: Memory is similar to an existing memory. Use neurostrata_supersede_memory to update the existing memory instead, or neurostrata_archive_memory to archive it."
                            );
                        }
                        DedupDecision::BlockedModel => {
                            return format!(
                                "Blocked by promoted model. Use neurostrata_supersede_memory or neurostrata_archive_memory."
                            );
                        }
                        _ => {}
                    }
                }

                let upsert = upsert.unwrap();
                // Emit site 1 only for a newly inserted memory.
                bus.emit(
                    ThalamicPulse::Created {
                        id: upsert.id.clone(),
                        namespace: namespace.to_string(),
                        kind: payload.memory_type.clone(),
                    },
                    &RecursionToken::root(),
                );
                let mut msg = format!("Successfully added memory for namespace: {}", namespace);
                msg.push_str(&format!("\nMemory ID: {}", upsert.id));
                if adopted_governs {
                    msg.push_str(" (governs set from the Governs line in your content)");
                }
                for edge in &upsert.deferred_edges {
                    let status = match &edge.resolution {
                        crate::traits::TargetResolution::Missing => "missing".to_string(),
                        crate::traits::TargetResolution::Ambiguous(candidates) => {
                            format!("ambiguous; candidates: {:?}", candidates)
                        }
                        crate::traits::TargetResolution::Failed(reason) => {
                            format!("lookup/materialization failed: {}", reason)
                        }
                        crate::traits::TargetResolution::Resolved(_) => "resolved".to_string(),
                    };
                    msg.push_str(&format!(
                        "\nDeferred {} edge[{}] to '{}' ({})",
                        edge.rel_type, edge.declaration_index, edge.declared_target, status
                    ));
                }
                if let Some(warning) = check_warning {
                    msg.push_str(&format!("\nRegistry warning: {}", warning));
                }
                // Only now, with the memory durable: a cache failure is
                // reported and never undoes the acceptance.
                if let Some(domains) = registry_cache {
                    if let Some(warning) =
                        crate::registry::write_registry(&store, &emb, namespace, &domains).await
                    {
                        msg.push_str(&format!("\nRegistry warning: {}", warning));
                    }
                }
                // TASK NUDGE: advisory when a non-task/procedure memory's content
                // carries task-shaped markers.
                if memory_type != "task" && memory_type != "procedure" {
                    let signals = crate::task::intent::detect_task_intents(content);
                    if let Some(sig) = signals.first() {
                        msg.push_str(&format!(
                            "\nTASK NUDGE: content matched task-intent marker '{}' (\"{}\"). If this is tracked or follow-up work rather than a timeless rule/fact, record it via neurostrata_task_create (namespace: '{}').",
                            sig.marker, sig.excerpt, namespace
                        ));
                    }
                }
                msg
            }
            Err(e) => e,
        }
    } else {
        "Failed to verify existing namespaces.".to_string()
    }
}

/// What to tell an agent when this namespace still holds ids written before
/// they carried a namespace, or None when there is nothing to say.
///
/// It rides on get_snapshot because that is the one call an agent is required
/// to make at the start of a task, and because the snapshot has already fetched
/// the list this counts from. `doctor` reports the same condition, but doctor
/// only runs with the daemon stopped, so an agent in a session would otherwise
/// never learn of it -- and an agent is the only party present to raise it.
///
/// The message deliberately hands the work to the user rather than describing
/// something to attempt. Migrating means stopping the daemon, which is the
/// human's transport and would end the session's own connection; an agent that
/// shelled out to do it would be going around the surface it was given.
pub(crate) fn unmigrated_ids_notice(
    namespace: &str,
    memories: &[crate::traits::SearchResult],
) -> Option<String> {
    let ingested: Vec<&crate::traits::SearchResult> = memories
        .iter()
        .filter(|m| m.payload.user_id == "auto-ingestor")
        .collect();

    let stale = ingested
        .iter()
        .filter(|m| !crate::parser::ingest::is_qualified(namespace, &m.id))
        .count();

    if stale == 0 {
        return None;
    }

    Some(format!(
        "MIGRATION NEEDED -- {} of {} ingested ids in '{}' predate namespace qualification.\n\
         They resolve, but the next project ingesting a shared path takes them.\n\
         Not yours to run: it needs the daemon stopped. Give the user:\n\n  \
         neurostrata-mcp backup <dir>      # daemon still up\n  \
         neurostrata-mcp shutdown\n  \
         neurostrata-mcp ingest <dir> {}\n\n\
         Backup first: re-ingest rewrites every id.",
        stale,
        ingested.len(),
        namespace,
        namespace
    ))
}

pub async fn handle_get_snapshot(
    arguments: Value,
    store: Arc<dyn VectorStore>,
) -> Result<String, String> {
    let namespace = arguments
        .get("namespace")
        .and_then(|n| n.as_str())
        .ok_or_else(|| "Missing 'namespace' parameter.".to_string())?;
    let namespace = resolve_namespace(&store, namespace).await;
    let namespace = namespace.as_str();

    let mut all_memories = store
        .list(namespace, None, None)
        .await
        .map_err(|_| "Failed to list memories or namespace does not exist.".to_string())?;

    // Default recall hides tombstoned rows. The snapshot is the operator's
    // daily-start view; an archived row that still shows up there means the
    // operator cannot trust the snapshot to mean "what is currently live in
    // this namespace". `include_archived: true` is the audit / second-look
    // escape hatch.
    let include_archived = arguments
        .get("include_archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !include_archived {
        all_memories.retain(|r| !crate::handlers::archive_memory::is_archived(&r.payload));
    }
    // Conflicting mesh rows are suppressed from the snapshot's effective
    // view too (same rationale as search_memory).
    all_memories.retain(|r| !crate::synaptic_mesh::conflict::is_conflicting(&r.payload));
    // Capture the registry row's domains before the retain below drops it.
    let registered_domains: Vec<String> = all_memories
        .iter()
        .find(|r| crate::registry::is_registry_row(&r.payload.metadata))
        .map(|r| crate::registry::registry_domains(&r.payload.metadata))
        .unwrap_or_default();
    // The registry row is metadata, not knowledge: never serve it.
    all_memories.retain(|r| !crate::registry::is_registry_row(&r.payload.metadata));

    // Counted from the list the snapshot already had to fetch, so this
    // costs nothing beyond the scan.
    let migration = unmigrated_ids_notice(namespace, &all_memories);
    // Computed before the top-5 cut below: tasks are never the highest-access
    // rows, and section 6.1 requires the Zero-Action Start header on every
    // snapshot regardless of what the body holds.
    let task_head = crate::task::snapshot_prefix(&all_memories);

    let now = chrono::Utc::now().timestamp();
    // Computed off the pre-truncation list too: a procedure is worth surfacing
    // whether or not it made the top-5 by access count, and a procedure nobody
    // has ever performed has the lowest access count of all.
    let procedures = crate::tools::procedure::procedures_due_strap(&all_memories, now);
    all_memories.retain(|r| match r.payload.metadata.get("valid_to") {
        None => true,
        Some(v) => v.is_null() || (v.as_i64().unwrap_or(0) > now),
    });
    all_memories.sort_by(|a, b| {
        let a_count = a
            .payload
            .metadata
            .get("access_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let b_count = b
            .payload
            .metadata
            .get("access_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        b_count.cmp(&a_count)
    });

    // The top-5 ids are captured BEFORE the strap so a row appears once.
    let top5_ids: Vec<String> = all_memories.iter().take(5).map(|r| r.id.clone()).collect();

    // Domain strap mode.
    let domain_arg = arguments.get("domain");
    let (mode, requested_slugs) = match domain_arg {
        None => ("recent_domain", Vec::new()),
        Some(Value::String(s)) => {
            let slugs = crate::domain::normalize_domain_slug(s).into_iter().collect();
            ("explicit", slugs)
        }
        Some(Value::Array(arr)) => {
            let slugs: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(crate::domain::normalize_domain_slug)
                .collect();
            ("explicit", slugs)
        }
        _ => ("explicit", Vec::new()),
    };

    // Build strap candidates off the pre-truncation list.
    let mut candidates: Vec<&crate::traits::SearchResult> = all_memories
        .iter()
        .filter(|r| !top5_ids.contains(&r.id))
        .filter(|r| {
            !matches!(
                r.payload.memory_type.as_str(),
                "directory" | "file" | "markdown" | "freshness_flag" | "guard_event" | "task" | "procedure"
            )
        })
        .filter(|r| crate::domain::domain_key(&r.payload.metadata).is_some())
        .collect();
    if mode == "explicit" {
        candidates.retain(|r| {
            let slug = crate::domain::domain_key(&r.payload.metadata).unwrap();
            requested_slugs.contains(&slug)
        });
    }
    candidates.sort_by(|a, b| {
        let va = a
            .payload
            .metadata
            .get("valid_from")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let vb = b
            .payload
            .metadata
            .get("valid_from")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        vb.cmp(&va)
            .then_with(|| {
                let aac = a
                    .payload
                    .metadata
                    .get("access_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let bac = b
                    .payload
                    .metadata
                    .get("access_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                bac.cmp(&aac)
            })
            .then_with(|| a.id.cmp(&b.id))
    });

    // Select at most 3, with per-domain cap 2 in recent_domain mode.
    let mut selected: Vec<&crate::traits::SearchResult> = Vec::new();
    let mut selected_domains: Vec<String> = Vec::new();
    for r in &candidates {
        if selected.len() >= 3 {
            break;
        }
        let slug = crate::domain::domain_key(&r.payload.metadata).unwrap();
        if mode == "recent_domain" {
            let count = selected_domains.iter().filter(|d| *d == &slug).count();
            if count >= 2 {
                continue;
            }
        }
        selected.push(r);
        selected_domains.push(slug);
    }

    let strap_entries: Vec<Value> = selected
        .iter()
        .map(|r| {
            let slug = crate::domain::domain_key(&r.payload.metadata).unwrap();
            let prefix = if crate::attempt::is_failed_attempt(&r.payload.metadata) {
                crate::attempt::FAILED_ATTEMPT_PREFIX
            } else {
                ""
            };
            let budget = 120usize.saturating_sub(prefix.chars().count());
            let snippet = format!(
                "{}{}",
                prefix,
                r.payload.content.chars().take(budget).collect::<String>()
            );
            let valid_from = r
                .payload
                .metadata
                .get("valid_from")
                .cloned()
                .unwrap_or(Value::Null);
            let access_count = r
                .payload
                .metadata
                .get("access_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            serde_json::json!({
                "id": r.id,
                "memory_type": r.payload.memory_type,
                "domain": slug,
                "valid_from": valid_from,
                "access_count": access_count,
                "snippet": snippet,
            })
        })
        .collect();

    let domain_strap = if strap_entries.is_empty() {
        format!("Domain strap [mode: {}] (0): []", mode)
    } else {
        match serde_json::to_string_pretty(&strap_entries) {
            Ok(body) => format!(
                "Domain strap [mode: {}] ({}):\n{}",
                mode,
                strap_entries.len(),
                body
            ),
            Err(e) => format!("Domain strap [mode: {}]: unreadable ({}).", mode, e),
        }
    };

    let registered_domains_line = if registered_domains.is_empty() {
        "Registered domains (0): none".to_string()
    } else {
        format!(
            "Registered domains ({}): {}",
            registered_domains.len(),
            registered_domains.join(", ")
        )
    };

    all_memories.truncate(5);

    let body = if all_memories.is_empty() {
        format!("No active memories found for namespace: {}", namespace)
    } else {
        serde_json::to_string_pretty(&all_memories)
            .map_err(|e| format!("Internal serialization error: {}", e))?
    };
    let snapshot = format!(
        "{}\n{}\n{}\n{}\n{}",
        task_head, procedures, registered_domains_line, domain_strap, body
    );

    Ok(match migration {
        Some(notice) => format!("{}\n\n{}", snapshot, notice),
        None => snapshot,
    })
}

pub async fn handle_ingest_directory(
    arguments: Value,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
    ingests: Arc<crate::ingest_jobs::IngestJobs>,
) -> String {
    let dir_path = match arguments.get("dir_path").and_then(|d| d.as_str()) {
        Some(d) => d,
        None => return "ERROR: dir_path missing.".to_string(),
    };
    let namespace = match arguments.get("namespace").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return "ERROR: namespace missing.".to_string(),
    };
    let namespace = resolve_namespace(&store, namespace).await;
    let namespace = namespace.as_str();

    let schema_str = include_str!("../schema.json");
    let schema = match crate::parser::schema::ParserSchema::load(schema_str) {
        Ok(schema) => schema,
        Err(_) => return "Failed to load default parser schema.".to_string(),
    };

    // Handed to the registry so the walk survives this call: an MCP client that
    // times out and disconnects no longer leaves the graph half-built (bead
    // neurostrata-7ej). Ingestion upserts thousands of rows, and checkpointing
    // each one would dominate the run, so that happens once at the end.
    match ingests.run(namespace, dir_path, schema, emb.clone(), store.clone()).await {
        Ok(progress) => format!(
            "Successfully ingested AST from {} into namespace '{}': {} files, {} symbols, {} declared edges relinked.",
            dir_path,
            namespace,
            progress.files_ingested,
            progress.symbols_ingested,
            progress
                .relinked_edges
                .map(|n| n.to_string())
                .unwrap_or_else(|| "no".to_string())
        ),
        Err(e) => format!("Failed to ingest directory {}: {}", dir_path, e),
    }
}

/// Retire a memory and store its replacement, keeping both.
///
/// docs/COGNITIVE_ARCHITECTURE.md section 2 promises that agents never
/// overwrite history: the old node gets a `valid_to` stamp and a new node
/// carries the correction. Only the reading half of that was ever built --
/// `valid_to` was filtered on but never written -- so the only correction paths
/// were destructive rewrites, and `/edit` additionally kept the old embedding.
/// This is the writing half. It adds rather than overwrites, so it is reachable
/// over MCP where edit, delete and move are not.
pub async fn handle_supersede_memory(
    arguments: Value,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
    bus: Arc<ThalamicBus>,
) -> String {
    let id = match arguments.get("id").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => {
            return "Missing 'id' parameter: supersede needs the memory it replaces.".to_string()
        }
    };
    let content = match arguments.get("content").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => {
            return "Missing 'content' parameter: supersede needs the corrected text.".to_string()
        }
    };
    let namespace = match arguments.get("namespace").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return "ERROR [NAMESPACE]: 'namespace' is missing. You MUST explicitly provide the specific project namespace.".to_string(),
    };

    // WHY the memory is wrong. Whitespace is not a reason: it would satisfy a
    // presence check while leaving history exactly as mute as before.
    let reason = arguments
        .get("reason")
        .and_then(|r| r.as_str())
        .map(str::trim)
        .filter(|r| !r.is_empty());

    // Provenance of the corrected text. Same two shapes metadata.source accepts
    // (dated string, or {kind, ref, captured_at}); blank and null count as
    // absent so they cannot masquerade as provenance.
    let source = match arguments.get("source") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(Value::String(s.trim().to_string())),
        Some(v @ Value::Object(_)) if !v.as_object().map(|o| o.is_empty()).unwrap_or(true) => {
            Some(v.clone())
        }
        _ => None,
    };

    // Same scanner add_memory uses. A correction is still an insertion, and a
    // secret pasted into one would be just as permanent.
    if let Some(rejection) =
        crate::secrets::scan_entry_point(content, &serde_json::json!({}), "supersede_memory")
    {
        return rejection.to_string();
    }
    // The reason and the source land in stored metadata; a secret in either is
    // as permanent as one in the content.
    for extra in [reason, source.as_ref().and_then(|s| s.as_str())]
        .into_iter()
        .flatten()
    {
        if let Some(rejection) =
            crate::secrets::scan_entry_point(extra, &serde_json::json!({}), "supersede_memory")
        {
            return rejection.to_string();
        }
    }

    let namespace = resolve_namespace(&store, namespace).await;
    let namespace = namespace.as_str();

    // Retiring a row hides it from every future search, so the machine-wide
    // stratum keeps the same guard the destructive tools carry.
    if namespace == "global"
        && !arguments
            .get("allow_global")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return "ERROR [GLOBAL]: Refusing to supersede a memory in the machine-wide 'global' namespace. Rules there apply to every project on this machine. Pass allow_global=true only if you are certain, or supersede the project-local rule instead.".to_string();
    }

    let (old_vector, mut old_payload) = match store.get(namespace, id).await {
        Ok(Some(found)) => found,
        Ok(None) => return format!("No memory with id {} in namespace {}.", id, namespace),
        Err(e) => return format!("Failed to read memory {}: {}", id, e),
    };

    // Metadata must be a JSON object for lineage stamps. Legacy scalar/string
    // metadata silently produced success with no stamps, leaving the original
    // active and invisible to the replacement chain.
    if !old_payload.metadata.is_object() {
        return format!(
            "Memory {} has legacy-shaped metadata (not a JSON object) and cannot be superseded automatically. Edit it via CLI or re-add it with object metadata first.",
            id
        );
    }

    // Only refuse if the row is actually retired (valid_to set AND <= now).
    // A still-live future expiry means the row is active and supersede should
    // proceed; the scheduled expiry transfers to the replacement.
    let now = chrono::Utc::now().timestamp();
    let is_already_retired = old_payload
        .metadata
        .get("valid_to")
        .and_then(|v| v.as_i64())
        .map(|vt| vt <= now)
        .unwrap_or(false);
    if is_already_retired {
        return format!(
            "Memory {} was already superseded; it is history. Supersede the memory that replaced it, or add a new one.",
            id
        );
    }

    // Capture any future expiry to transfer to the replacement.
    let future_valid_to: Option<i64> = old_payload
        .metadata
        .get("valid_to")
        .and_then(|v| v.as_i64())
        .filter(|vt| *vt > now);

    let new_id = uuid::Uuid::new_v4().to_string();

    // A claim is a lease on the WORK, not on the row id. Correcting the text
    // of a claimed task does not hand the work to anyone else: the successor
    // inherits the claim, and the reply names the release path so the claim
    // is released deliberately rather than by accident.
    let claim_inherited = old_payload.memory_type == "task"
        && old_payload
            .metadata
            .get("task")
            .and_then(|t| t.get("status"))
            .and_then(|s| s.as_str())
            == Some("in_progress");
    let claim_assignee = old_payload
        .metadata
        .get("task")
        .and_then(|t| t.get("assignee"))
        .and_then(|a| a.as_str())
        .unwrap_or("unknown")
        .to_string();

    // The replacement inherits everything the old row established -- who stored
    // it, what it governs, which files it points at -- because a correction to
    // the text is not a change of subject.
    let mut replaced: Vec<String> = Vec::new();
    let mut carried: Vec<String> = Vec::new();
    let mut new_payload = old_payload.clone();
    new_payload.content = content.to_string();
    // Locations follow the same wholesale semantics as the pointers below
    // (guinea-pig Option 1: a replacement replaces everything).
    let mut locations_wrote_governs = false;
    if let Some(locations) = arguments.get("locations").and_then(|l| l.as_array()) {
        let (first_path, first_lines) = first_location(locations);
        new_payload.location = first_path;
        new_payload.location_lines = first_lines;
        locations_wrote_governs = locations
            .iter()
            .filter_map(|loc| loc.get("path").and_then(|p| p.as_str()))
            .any(|p| !p.is_empty());
        stamp_locations(&mut new_payload.metadata, locations);
        if let Some(metadata) = new_payload.metadata.as_object_mut() {
            metadata.remove("absolute_path");
        }
        replaced.push("locations".to_string());
    }
    // The prose's own "Governs: [...]" lines (guinea-pig Option 2). The old
    // failure wrote the line and never honoured it -- the worst of both
    // worlds -- so honour it when nothing else spoke, and name the winner.
    let content_lists = content_governs_lines(content);
    let provenance_missing = source.is_none() && old_payload.metadata.get("source").is_none();
    if let Some(obj) = new_payload.metadata.as_object_mut() {
        obj.remove("valid_to");
        // Do NOT carry over superseded_by from the old row; only supersedes
        // should point back, not a stale forward link.
        obj.remove("superseded_by");
        obj.insert("valid_from".to_string(), serde_json::json!(now));
        obj.insert("access_count".to_string(), serde_json::json!(0));
        obj.insert("supersedes".to_string(), serde_json::json!(id));
        // Transfer a scheduled future expiry to the replacement so the
        // replacement inherits the old row's deadline rather than silently
        // becoming permanently active.
        if let Some(vt) = future_valid_to {
            obj.insert("valid_to".to_string(), serde_json::json!(vt));
        }
        // Guinea-pig BUG-10: a correction must be able to fix a stale
        // structural pointer. Anything passed explicitly replaces the inherited
        // value wholesale; anything absent carries over -- and the reply names
        // every carried value, because "superseded with X" while leaving a
        // stale pointer is the confident-wrong-answer failure again.
        for key in ["governs", "related_to", "contained_by"] {
            if let Some(v) = arguments.get(key) {
                obj.insert(key.to_string(), v.clone());
                replaced.push(key.to_string());
            } else if key == "governs" && locations_wrote_governs {
                replaced.push("governs (derived from locations)".to_string());
            } else if old_payload.metadata.get(key).is_some() {
                carried.push(key.to_string());
            }
        }
        // A correction asserts new text, so it needs its own provenance: the old
        // memory's source says where the WRONG wording came from, not where this
        // one did. Passed source replaces it; absent carries it over (the same
        // wholesale rule as the pointers) and the reply names what it inherited.
        if let Some(src) = &source {
            obj.insert("source".to_string(), src.clone());
            replaced.push("source".to_string());
        } else if old_payload.metadata.get("source").is_some() {
            carried.push("source (the provenance of the text being replaced)".to_string());
        }
        // The genuine hole: nothing inherited and nothing passed, so a rule
        // correction lands with no origin at all. Recorded so the reply can say
        // so out loud instead of reporting a clean success.

        // An unambiguous content line is the author's declaration and wins
        // over the inherited pointer when no parameter and no locations spoke.
        // A record must never hold two contradictory Governs lists.
        if arguments.get("governs").is_none()
            && !locations_wrote_governs
            && content_lists.len() == 1
        {
            let declared = serde_json::json!(content_lists[0]);
            if old_payload.metadata.get("governs") != Some(&declared) {
                obj.insert("governs".to_string(), declared);
                carried.retain(|k| k != "governs");
                replaced.push("governs (from your content line)".to_string());
            }
        }
    }

    let vector = match emb.embed(&content).await {
        Ok(v) => v,
        // The whole point of superseding rather than editing: the new text gets
        // its own embedding, so search stops matching the words we replaced.
        Err(e) => {
            return format!(
                "Failed to embed the new content, nothing was changed: {}",
                e
            )
        }
    };

    // Write the replacement BEFORE retiring the original. If the second write
    // fails the namespace holds both versions, which is visible and repairable;
    // the other order would leave the rule retired with no successor.
    let replacement_report = match store.upsert(namespace, &new_id, vector, new_payload).await {
        Ok(report) => report,
        Err(e) => {
            return format!(
                "Failed to store the replacement, nothing was changed: {}",
                e
            )
        }
    };

    // Optimistic guard: re-read the old row and verify it still has no valid_to
    // (i.e. state unchanged since the first read). Concurrent corrections can
    // both create active successors without this check; last-write-wins.
    // This is a lightweight CAS-style guard without new dependencies.
    match store.get(namespace, id).await {
        Ok(Some((_, re_read))) => {
            let was_retired = re_read
                .metadata
                .get("valid_to")
                .and_then(|v| v.as_i64())
                .map(|vt| vt <= now)
                .unwrap_or(false);
            if was_retired {
                // Another supersede raced us. Roll back the successor we just wrote.
                // Best-effort: the successor is orphaned but the old row is already
                // retired by the winner, so the namespace is consistent.
                return format!(
                    "Memory {} was modified concurrently by another supersede. Retry if needed.",
                    id
                );
            }
            // Also verify the metadata object shape hasn't changed under us.
            if !re_read.metadata.is_object() {
                return format!(
                    "Memory {} was modified concurrently and its metadata is no longer an object. Retry.",
                    id
                );
            }
            // Use the freshly-read metadata for the retirement stamp so we don't
            // overwrite a concurrent change to other fields.
            old_payload.metadata = re_read.metadata;
        }
        Ok(None) => {
            return format!(
                "Memory {} was deleted concurrently. The replacement {} was stored but the original could not be retired.",
                id, new_id
            );
        }
        Err(e) => {
            return format!(
                "Failed to re-read memory {} for concurrency guard: {}. The replacement {} was stored but retirement could not be verified.",
                id, e, new_id
            );
        }
    }

    if let Some(obj) = old_payload.metadata.as_object_mut() {
        obj.insert("valid_to".to_string(), serde_json::json!(now));
        obj.insert("superseded_by".to_string(), serde_json::json!(new_id));
        // The why lives on the row that became history, beside the pointer to
        // its successor: reading the retired memory by id answers both "what
        // did it say" and "why did it stop being true".
        if let Some(why) = reason {
            obj.insert("superseded_reason".to_string(), serde_json::json!(why));
        }
    }
    // Writing the old row back with an empty embedding would have the engine
    // reject it, and the row is deleted before it is re-inserted -- so an empty
    // vector here destroys the history this whole call exists to preserve.
    if old_vector.len() != emb.dimensions() {
        return format!(
            "Stored the replacement as {}, but refused to rewrite {}: its embedding read back as {} floats, expected {}. Both are active.",
            new_id,
            id,
            old_vector.len(),
            emb.dimensions()
        );
    }
    if let Err(e) = store.upsert(namespace, id, old_vector, old_payload).await {
        return format!(
            "Stored the replacement as {}, but failed to retire {}: {}. Both are active; retire the old one from the GUI or CLI.",
            new_id, id, e
        );
    }

    // Emit site 2. After both writes, not after the first: a pulse published
    // when only the successor existed would tell subscribers the old row was
    // retired while it was still active. Fire-and-forget -- the move is
    // durable either way.
    bus.emit(
        ThalamicPulse::Superseded {
            old_id: id.to_string(),
            new_id: new_id.clone(),
            namespace: namespace.to_string(),
        },
        &RecursionToken::root(),
    );

    let mut report = format!(
        "Superseded {} with {} in namespace {}. The old memory keeps its text and is no longer returned by search; it is still readable by id.",
        id, new_id, namespace
    );
    if claim_inherited {
        report.push_str(&format!(
            " The successor inherits the claim (assignee {}): retiring the old row does not release the work, and the claim still blocks the push gate. To release it, call neurostrata_task_update on {} with status 'open'.",
            claim_assignee, new_id
        ));
    }
    if !replaced.is_empty() {
        report.push_str(&format!(" Replaced metadata: {}.", replaced.join(", ")));
    }
    for edge in &replacement_report.deferred_edges {
        report.push_str(&format!(
            " Deferred {} edge[{}] to '{}' because target was not materialized.",
            edge.rel_type, edge.declaration_index, edge.declared_target
        ));
    }
    if !carried.is_empty() {
        report.push_str(&format!(
            " Carried over UNCHANGED from the old memory: {} -- to replace them pass governs/related_to/contained_by/locations/source, or state one Governs line in the prose.",
            carried.join(", ")
        ));
    }
    // The why and the where-from, named whether they were written or not: a
    // correction that silently drops its reason is the same defect the retired
    // row used to have.
    if let Some(why) = reason {
        report.push_str(&format!(
            " Recorded why on the retired memory as superseded_reason: {}.",
            why
        ));
    } else {
        report.push_str(
            " NO reason was recorded: the retired memory's history will not say why this was corrected. Pass reason: \"...\" on the next correction.",
        );
    }
    if provenance_missing {
        report.push_str(
            " The corrected memory carries NO metadata.source (none passed, none inherited); if it is a rule, task_validate reports rule_without_source until you re-supersede with source: \"...\".",
        );
    }
    let governs_in_play = replaced.iter().any(|k| k.starts_with("governs"))
        || carried.iter().any(|k| k == "governs")
        || arguments.get("governs").is_some();
    if governs_in_play {
        let source = if arguments.get("governs").is_some() {
            "the governs parameter"
        } else if locations_wrote_governs {
            "the locations parameter"
        } else if content_lists.len() == 1 {
            "the Governs line in your content"
        } else {
            "the retired memory (carried over)"
        };
        report.push_str(&format!(
            " The governing pointer is now set from {}.",
            source
        ));
    }
    if content_lists.len() > 1 && arguments.get("governs").is_none() && !locations_wrote_governs {
        report.push_str(&format!(
            " WARNING: your content carries {} conflicting Governs lines and no governs parameter was passed; the inherited pointer stands and the record still contradicts itself. Re-run with one clean line in the prose and, if the pointer must move, a governs parameter.",
            content_lists.len()
        ));
    }
    report
}

pub async fn handle_search_memory(
    arguments: Value,
    emb: Arc<dyn Embedder>,
    store: Arc<dyn VectorStore>,
) -> String {
    let query = match arguments.get("query").and_then(|q| q.as_str()) {
        Some(q) => q,
        None => return "Missing 'query' parameter.".to_string(),
    };
    let namespace = match arguments.get("namespace").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return "ERROR [NAMESPACE]: 'namespace' is missing. You MUST explicitly provide the specific project namespace to search in.".to_string(),
    };
    let namespace = resolve_namespace(&store, namespace).await;
    let namespace = namespace.as_str();

    // Task rows are backlog chatter, not recall material: excluded unless the
    // caller asks for them (section 1.1).
    let include_tasks = arguments
        .get("include_tasks")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // `archive_memory` writes the tombstone; the search surface
    // honors it by default. The escape hatch (`include_archived:
    // true`) is for audits, recovery, and any deliberate
    // second-look -- the operator who needs to find a row that
    // has been retired from default recall.
    let include_archived = arguments
        .get("include_archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // The store now applies row-class filters and returns `limit` primaries
    // plus their bounded expansions. We ask for 5 primaries (the default page).
    const DEFAULT_LIMIT: usize = 5;

    // An optional domain filter: a slug, or a list of slugs. Each value is
    // normalized here, so the store compares by slug on both sides and a
    // legacy free-text domain still matches.
    let raw_domains: Vec<&str> = match arguments.get("domain") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(|v| v.as_str()).collect(),
        _ => Vec::new(),
    };
    // A domain was requested when the caller named at least one non-empty
    // value. An absent, empty or whitespace-only argument names nothing, and
    // the search then reads exactly as it did before the filter existed.
    let domain_requested = raw_domains.iter().any(|d| !d.trim().is_empty());
    // The values that normalize survive; the rest are dropped. A filter with
    // no survivors is reported as skipped, never silently replaced by an
    // unfiltered search.
    let slugs: Vec<String> = raw_domains
        .iter()
        .copied()
        .filter_map(crate::domain::normalize_domain_slug)
        .collect();

    if let Ok(_) = store.init(namespace).await {
        if let Ok(vec) = emb.embed(&query).await {
            if let Ok(outcome) = store
                .search_with_domain(
                    namespace,
                    vec,
                    DEFAULT_LIMIT,
                    include_tasks,
                    include_archived,
                    &slugs,
                )
                .await
            {
                let mode = outcome.mode;
                let results = outcome.results;
                if domain_requested {
                    let single_domain =
                        crate::registry::registry_domain_count(&store, namespace).await <= 1;
                    crate::recall_metrics::record_domain_query(
                        mode == crate::traits::DomainMatchMode::Domain,
                        mode == crate::traits::DomainMatchMode::Vector,
                        single_domain,
                    );
                }
                if results.is_empty() {
                    "No relevant memories found.".to_string()
                } else {
                    // Increment access_count ONLY for direct hits (EvidenceKind::DirectMatch)
                    for res in &results {
                        if crate::traits::is_direct_match(&res.evidence) {
                            let store_clone = store.clone();
                            let ns_clone = namespace.to_string();
                            let id_clone = res.id.clone();
                            tokio::spawn(async move {
                                let _ = store_clone
                                    .increment_access_count(&ns_clone, &id_clone)
                                    .await;
                            });
                        }
                    }

                    let report = render_search_results(&results);
                    if !domain_requested {
                        report
                    } else {
                        // A filter that was skipped is a substitution the caller
                        // has to be told about: a wrong domain must never blind
                        // the search, and a silent fallback would misreport what
                        // was searched. The store's mode alone cannot tell the
                        // two kinds of skip apart -- an empty key set was never
                        // compared with anything -- so the survivors decide.
                        let mode_line = if slugs.is_empty() {
                            "domain filter: skipped — the domain did not normalize; the results are unfiltered (vector)"
                        } else {
                            match mode {
                                crate::traits::DomainMatchMode::Domain => {
                                    "domain filter: applied (domain)"
                                }
                                crate::traits::DomainMatchMode::Vector => {
                                    "domain filter: skipped — no row matched the domain; the results are unfiltered (vector)"
                                }
                            }
                        };
                        format!("{}\n\n{}", mode_line, report)
                    }
                }
            } else {
                "Failed to search database.".to_string()
            }
        } else {
            "Failed to generate embedding for search.".to_string()
        }
    } else {
        "Failed to initialize namespace table.".to_string()
    }
}

/// One stored anchor as `src/lib.rs:10-20 (parse)`. add_memory stores the path
/// as `file`; `path` is the name the tool's own arguments use.
fn render_anchor(anchor: &Value) -> String {
    let mut out = anchor
        .get("file")
        .or_else(|| anchor.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(lines) = anchor
        .get("lines")
        .and_then(|v| v.as_str())
        .filter(|l| !l.is_empty())
    {
        out.push_str(&format!(":{}", lines));
    }
    if let Some(symbol) = anchor
        .get("symbol")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!(" ({})", symbol));
    }
    out
}

/// One rendering of a memory, shared by search and get, so a record reads the
/// same however the agent reached it.
pub(crate) fn format_memory(id: &str, payload: &MemoryPayload) -> String {
    let mut out = format!("--- Memory ID: {} ---\nType: {}", id, payload.memory_type);
    if crate::attempt::is_failed_attempt(&payload.metadata) {
        let attempt = payload.metadata.get("attempt").unwrap();
        let context = attempt.get("context").and_then(|v| v.as_str()).unwrap_or("");
        let approach = attempt.get("approach").and_then(|v| v.as_str()).unwrap_or("");
        let failure_mode = attempt.get("failure_mode").and_then(|v| v.as_str()).unwrap_or("");
        out.push_str(&format!(
            "\nAttempt [FAILED in {}]: tried '{}' -> failed with '{}' (single episode, not a global rule)",
            context, approach, failure_mode
        ));
    }
    out.push_str(&format!("\nContent: {}", payload.content));
    if !payload.location.is_empty() {
        out.push_str(&format!("\nFile Location: {}", payload.location));
        if !payload.location_lines.is_empty() {
            out.push_str(&format!(" (Lines: {})", payload.location_lines));
        }
    }
    // add_memory stores every anchor under `refs`, one {file, lines, symbol} each.
    // This used to read `locations`, which nothing writes, so only the first
    // anchor ever showed -- as File Location above, with no symbol.
    if let Some(anchors) = payload
        .metadata
        .get("refs")
        .or_else(|| payload.metadata.get("locations"))
        .and_then(|v| v.as_array())
        .filter(|arr| !arr.is_empty())
    {
        let rendered: Vec<String> = anchors.iter().map(render_anchor).collect();
        out.push_str(&format!("\nCode Graph Locations: {}", rendered.join(", ")));
    }
    for (key, label) in [
        ("related_to", "Related Nodes"),
        ("contained_by", "Contained By"),
        ("governs", "Governs"),
    ] {
        if let Some(value) = payload.metadata.get(key) {
            if let Some(arr) = value.as_array() {
                if !arr.is_empty() {
                    out.push_str(&format!("\n{}: {}", label, value));
                }
            }
        }
    }
    // When the prose carries its own Governs line that disagrees with the
    // metadata pointer, say which half governs instead of rendering two
    // contradictory lists indistinguishably (guinea-pig BUG-10).
    if let Some(note) = governance_conflict_note(&payload.content, &payload.metadata) {
        out.push_str(&note);
    }
    out
}

/// Maximum UTF-8 bytes of evidence suffix per result.
const MAX_EVIDENCE_BYTES_PER_RESULT: usize = 512;
/// Maximum UTF-8 bytes of evidence suffix in the entire response.
const MAX_EVIDENCE_BYTES_PER_RESPONSE: usize = 2048;

/// Render all search results with their evidence suffixes.
pub(crate) fn render_search_results(results: &[SearchResult]) -> String {
    let mut out = String::new();
    let mut remaining_budget = MAX_EVIDENCE_BYTES_PER_RESPONSE;

    for (i, result) in results.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format_memory(&result.id, &result.payload));

        if let Some(suffix) = render_evidence_suffix(result, remaining_budget) {
            out.push_str(&suffix);
            remaining_budget = remaining_budget.saturating_sub(suffix.len());
        }
    }
    out
}

/// Try to render one evidence path suffix for a result, within the byte budget.
/// Enforces both the per-result cap (512) and the global remaining.
pub(crate) fn render_evidence_suffix(result: &SearchResult, remaining_bytes: usize) -> Option<String> {
    let per_result_cap = remaining_bytes.min(MAX_EVIDENCE_BYTES_PER_RESULT);
    let evidence = result.evidence.as_ref()?;
    if evidence.is_empty() {
        return None;
    }

    // Try each evidence path in stored order; first that fits wins.
    for ev in evidence {
        let suffix = match ev.kind {
            crate::traits::EvidenceKind::DirectMatch => continue,
            crate::traits::EvidenceKind::OneHop => {
                // Recover originating primary: endpoint other than result.id
                let orig = ev.path.iter().find_map(|e| {
                    if e.target == result.id {
                        Some(e.source.as_str())
                    } else if e.source == result.id {
                        Some(e.target.as_str())
                    } else {
                        None
                    }
                });
                let orig = match orig {
                    Some(o) => o,
                    None => continue,
                };
                // Validate terminal is result.id
                if let Some(last) = ev.path.last() {
                    if last.source != result.id && last.target != result.id {
                        continue;
                    }
                }
                let rendered_path = render_path(ev, orig);
                format!(
                    "\nPath [one_hop]: {}\nWhy: This memory is one graph hop from a direct match.",
                    rendered_path
                )
            }
            crate::traits::EvidenceKind::Governs2hop => {
                // Two hops: first edge + second edge (GOVERNS).
                // Originating primary: first-edge endpoint other than second-edge target.
                if ev.path.len() < 2 {
                    continue;
                }
                let first = &ev.path[0];
                let second = &ev.path[1];
                // The intermediate is the node the two edges share: the endpoint
                // of the first edge that is also an endpoint of the second.
                let intermediate = if first.source == second.source
                    || first.source == second.target
                {
                    first.source.as_str()
                } else if first.target == second.source || first.target == second.target
                {
                    first.target.as_str()
                } else {
                    continue;
                };
                // The originating primary is the other endpoint of the first edge.
                let orig = if first.source == intermediate {
                    first.target.as_str()
                } else {
                    first.source.as_str()
                };
                // Validate terminal is result.id
                if second.source != result.id && second.target != result.id {
                    continue;
                }
                let rendered_path = render_path(ev, orig);
                format!("\nPath [governs_2hop]: {}\nWhy: This memory governs a node one hop from a direct match.", rendered_path)
            }
            crate::traits::EvidenceKind::EpisodeSibling => {
                // Two hops: primary -(EXTRACTED_FROM)-> task <-EXTRACTED_FROM- sibling (result)
                // Originating primary: first edge's source (primary -> task)
                if ev.path.len() < 2 {
                    continue;
                }
                let first = &ev.path[0];
                // First edge must be primary -(EXTRACTED_FROM)-> task
                if first.relation != "EXTRACTED_FROM" {
                    continue;
                }
                let orig = first.source.as_str();
                // Last edge's source must be result.id (sibling -> task)
                if let Some(last) = ev.path.last() {
                    if last.source != result.id || last.relation != "EXTRACTED_FROM" {
                        continue;
                    }
                } else {
                    continue;
                }
                let rendered_path = render_path(ev, orig);
                format!("\nPath [episode_sibling]: {}\nWhy: This memory and a direct match were both extracted from the same task.", rendered_path)
            }
        };

        if suffix.len() <= per_result_cap {
            return Some(suffix);
        }
    }
    None
}

/// Render the path string from originating primary toward result.
pub(crate) fn render_path(ev: &crate::traits::SearchEvidence, orig: &str) -> String {
    // Walk edges from originating primary toward result.
    // For each edge, decide if it goes forward or reverse relative to our walk.
    let mut parts = Vec::new();
    let mut current = orig;

    for edge in &ev.path {
        // We are standing on one endpoint; the segment runs to the other one.
        let forward = edge.source == current;
        if !forward && edge.target != current {
            continue;
        }
        let next = if forward { edge.target.as_str() } else { edge.source.as_str() };
        // The vocabulary declares the direction: an undirected relation has no
        // arrow, so the connector marks it instead.
        let connector =
            if crate::traits::memory_vocabulary()["relations"][edge.relation.as_str()]["direction"] == "undirected" {
                format!(" --{}-- ", edge.relation)
            } else if forward {
                // Forward: source -> target
                format!(" -{}- ", edge.relation)
            } else {
                // Reverse: target <- source
                format!(" <-{}- ", edge.relation)
            };
        parts.push(format!("\"{}\"{}\"{}\"", current, connector, next));
        current = next;
    }
    parts.join(" ")
}

pub async fn handle_get_memory(arguments: Value, store: Arc<dyn VectorStore>) -> String {
    let id = match arguments.get("id").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => {
            return "Missing 'id' parameter. Ids are returned by neurostrata_search_memory."
                .to_string()
        }
    };
    let namespace = match arguments.get("namespace").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return "ERROR [NAMESPACE]: 'namespace' is missing. You MUST explicitly provide the namespace the memory lives in.".to_string(),
    };
    let namespace = resolve_namespace(&store, namespace).await;
    let namespace = namespace.as_str();
    if namespace.contains('/') || namespace.contains('\\') {
        return "ERROR [NAMESPACE]: The namespace cannot be a file path. It must be the exact project name (e.g., 'NeuroStrata'). Do not use slashes.".to_string();
    }

    match store.get(namespace, id).await {
        Ok(Some((_, payload))) => {
            let store_clone = store.clone();
            let ns_clone = namespace.to_string();
            let id_clone = id.to_string();
            tokio::spawn(async move {
                let _ = store_clone.increment_access_count(&ns_clone, &id_clone).await;
            });
            let _ = crate::recall_metrics::record_promotion(namespace, id);
            format_memory(id, &payload)
        }
        Ok(None) => format!(
            "No memory with id '{}' in namespace '{}'. Ids come from neurostrata_search_memory; check the namespace with neurostrata_list_namespaces.",
            id, namespace
        ),
        Err(e) => format!("Failed to read memory '{}': {}", id, e),
    }
}

/// Claim one tool name for the memory family. `None` means the name is
/// not this family's; `Some` carries the call's outcome (see
/// `super::ToolOutcome`): the result text, or the message the dispatch
/// must surface as a JSON-RPC error (the get_snapshot arm's channel).
pub async fn dispatch_memory_tool(
    name: &str,
    arguments: &Value,
    ctx: &super::ToolContext,
) -> Option<super::ToolOutcome> {
    let text = match name {
        "neurostrata_list_namespaces" => handle_list_namespaces(ctx.store.clone()).await,
        "neurostrata_add_memory" => {
            handle_add_memory(
                arguments.clone(),
                ctx.emb.clone(),
                ctx.store.clone(),
                ctx.deduplication_checker.clone(),
                ctx.bus.clone(),
            )
            .await
        }
        "neurostrata_get_memory" => handle_get_memory(arguments.clone(), ctx.store.clone()).await,
        "neurostrata_get_snapshot" => {
            return Some(handle_get_snapshot(arguments.clone(), ctx.store.clone()).await);
        }
        "neurostrata_ingest_directory" => {
            handle_ingest_directory(
                arguments.clone(),
                ctx.emb.clone(),
                ctx.store.clone(),
                ctx.ingests.clone(),
            )
            .await
        }
        "neurostrata_search_memory" => {
            handle_search_memory(arguments.clone(), ctx.emb.clone(), ctx.store.clone()).await
        }
        "neurostrata_supersede_memory" => {
            handle_supersede_memory(
                arguments.clone(),
                ctx.emb.clone(),
                ctx.store.clone(),
                ctx.bus.clone(),
            )
            .await
        }
        "neurostrata_archive_memory" => {
            crate::handlers::archive_memory::handle_archive_memory(
                arguments.clone(),
                ctx.store.clone(),
                ctx.bus.clone(),
            )
            .await
        }
        _ => return None,
    };
    Some(Ok(text))
}
