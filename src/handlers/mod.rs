//! Request handlers, extracted from `server.rs` so the daemon's HTTP
//! routes, the CLI, and the MCP tool surface call one implementation
//! instead of copies that can drift from each other.
//!
//! The MCP tool families live here as sibling modules (memory, guard,
//! tasks, system, mesh) next to the handlers that are not part of the
//! tool surface (archive_memory, bus_metrics). Each family module
//! exposes a `dispatch_*_tool` fn the MCP dispatch in
//! `crate::server::process_mcp_request` chains through; `tool_list`
//! holds the catalogue the `tools/list` arm serves.

pub mod archive_memory;
pub mod bus_metrics;
pub mod memory;

use crate::traits::{Embedder, VectorStore};
use std::sync::Arc;

/// The ambient services every MCP tool handler may need, bundled so the
/// per-family dispatch functions take one context instead of five
/// near-identical argument lists. `process_mcp_request` builds it once
/// per request from the same values the pre-split dispatch threaded
/// through its match arms.
pub struct ToolContext {
    pub emb: Arc<dyn Embedder>,
    pub store: Arc<dyn VectorStore>,
    pub ingests: Arc<crate::ingest_jobs::IngestJobs>,
    pub deduplication_checker: Option<Arc<crate::judgment::DeduplicationChecker>>,
    pub bus: Arc<crate::events::ThalamicBus>,
}

/// What a claimed tool call produced. `Ok` is the result text the
/// dispatch wraps in the success envelope; `Err` is a message the
/// dispatch surfaces as a JSON-RPC error response — the pre-split
/// dispatch's `error_response` channel, used by the get_snapshot and
/// task_complete arms.
pub type ToolOutcome = Result<String, String>;

// Test-only re-export: `src/main.rs`'s `bus-metrics` CLI test calls the handler
// through the short path. Production callers use the fully-qualified
// `crate::handlers::bus_metrics::…`, so this stays out of the shipped surface.
#[cfg(test)]
pub use self::bus_metrics::handle_bus_metrics;
