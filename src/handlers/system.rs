//! Session/system tool dispatch (Task 20): the tools that belong to
//! no store family — the episodic-buffer log and procedure firing.
//! Their handlers live in `crate::buffer` and
//! `crate::tools::procedure`; this module is the family's claim fn
//! only.

use serde_json::Value;

/// Claim one tool name for the session/system family. `None` means
/// the name is not this family's; `Some(Ok(text))` carries the result
/// text (see `super::ToolOutcome`).
pub async fn dispatch_system_tool(
    name: &str,
    arguments: &Value,
    ctx: &super::ToolContext,
) -> Option<super::ToolOutcome> {
    let text = match name {
        "neurostrata_append_log" => {
            crate::buffer::handle_append_log(arguments.clone(), ctx.emb.clone(), ctx.store.clone())
                .await
        }
        "neurostrata_procedure_perform" => {
            crate::tools::procedure::handle_procedure_perform(arguments.clone(), ctx.store.clone())
                .await
        }
        _ => return None,
    };
    Some(Ok(text))
}
