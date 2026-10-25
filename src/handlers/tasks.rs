//! Task tool dispatch (Task 20). The task handlers themselves live in
//! `crate::task` / `crate::task::intake` — they were never part of
//! `server.rs` — so this module is the family's claim fn only: it maps
//! each task tool name to the handler the pre-split dispatch called,
//! with the same arguments in the same order.

use serde_json::Value;

/// Claim one tool name for the task family. `None` means the name is
/// not this family's; `Some` carries the call's outcome (see
/// `super::ToolOutcome`) — task_complete's `Err` is the JSON-RPC error
/// channel (a refused Lock 2 completion).
pub async fn dispatch_task_tool(
    name: &str,
    arguments: &Value,
    ctx: &super::ToolContext,
) -> Option<super::ToolOutcome> {
    let text = match name {
        "neurostrata_task_create" => {
            crate::task::handle_task_create(arguments.clone(), ctx.emb.clone(), ctx.store.clone())
                .await
        }
        "neurostrata_task_claim" => {
            crate::task::handle_task_claim(arguments.clone(), ctx.store.clone()).await
        }
        "neurostrata_task_update" => {
            crate::task::handle_task_update(arguments.clone(), ctx.emb.clone(), ctx.store.clone())
                .await
        }
        "neurostrata_task_list" => {
            crate::task::handle_task_list(arguments.clone(), ctx.store.clone()).await
        }
        "neurostrata_task_complete" => {
            return Some(
                crate::task::handle_task_complete(
                    arguments.clone(),
                    ctx.emb.clone(),
                    ctx.store.clone(),
                    ctx.deduplication_checker.clone(),
                    ctx.bus.clone(),
                )
                .await,
            );
        }
        "neurostrata_task_validate" => {
            crate::task::handle_task_validate(arguments.clone(), ctx.store.clone()).await
        }
        "neurostrata_task_intake" => {
            crate::task::intake::handle_intake(arguments.clone(), ctx.store.clone()).await
        }
        "neurostrata_task_intake_list" => {
            crate::task::intake::handle_intake_list(arguments.clone(), ctx.store.clone()).await
        }
        "neurostrata_task_intake_resolve" => {
            crate::task::intake::handle_intake_resolve(arguments.clone(), ctx.store.clone()).await
        }
        "neurostrata_bootstrap" => {
            crate::task::handle_bootstrap(arguments.clone(), ctx.emb.clone(), ctx.store.clone())
                .await
        }
        "neurostrata_task_setup" => {
            crate::task::handle_task_setup(arguments.clone(), ctx.emb.clone(), ctx.store.clone())
                .await
        }
        _ => return None,
    };
    Some(Ok(text))
}
