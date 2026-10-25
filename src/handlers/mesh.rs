//! SynapticMesh tool dispatch (Task 20). The handler is a pure
//! string-in/string-out dispatch (filesystem + git only; no store, no
//! bus, no daemon dependency), so it owns its own name mapping in the
//! mesh module. Slice 4 exception: pull is the consume path — it
//! replays validated committed mesh records into the store through
//! the traits, so it takes store + embedder and lives in
//! `crate::synaptic_mesh::import`.

use serde_json::Value;

/// Claim one tool name for the mesh family: any
/// `neurostrata_synaptic_mesh_*` name. `None` means the name is not
/// this family's; `Some(Ok(text))` carries the result text (see
/// `super::ToolOutcome`).
pub async fn dispatch_mesh_tool(
    name: &str,
    arguments: &Value,
    ctx: &super::ToolContext,
) -> Option<super::ToolOutcome> {
    if !name.starts_with("neurostrata_synaptic_mesh_") {
        return None;
    }
    let text = if name == "neurostrata_synaptic_mesh_pull" {
        crate::synaptic_mesh::import::handle_mesh_pull(
            arguments,
            ctx.store.clone(),
            ctx.emb.clone(),
        )
        .await
    } else {
        crate::synaptic_mesh::handle_mesh_tool_call(name, arguments)
    };
    Some(Ok(text))
}
