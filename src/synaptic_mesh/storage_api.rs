//! The seam the CLI and MCP tool layers call. `MeshStorage` (slice 2a) is
//! the real implementation; it lives in `storage.rs`, which is written
//! against this shape.
//!
//! Everything here is `String`-typed rather than `anyhow`- or
//! `thiserror`-typed on purpose: this is a boundary trait, and the CLI layer
//! that consumes it formats failures as one-line replies. A trait method
//! that panicked would take the MCP server down with it; an `Err(String)`
//! does not.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

/// The operations the synaptic-mesh CLI and MCP tools need.
///
/// Contract notes the callers rely on:
///
/// - `push`, `pull`, and `status` answer `Err` (or a clean "no mesh joined"
///   report) when no mesh is joined. They never block on a network prompt:
///   all git plumbing runs with `GIT_TERMINAL_PROMPT=0`.
/// - `init_mesh` returns the new mesh id, which the caller echoes back.
/// - The git-touching methods (`init_mesh`, `status`, `push`, `pull`) are
///   async: mesh git runs as a timed async subprocess at the storage
///   funnel (DR-4), never as a blocking call on the runtime. The pure
///   pointer operations stay synchronous — no git is reachable from them.
#[async_trait]
pub trait StorageApi {
    /// Create a mesh repo under `base_root` and return its id.
    async fn init_mesh(
        &mut self,
        base_root: &Path,
        node_id: &str,
        mesh_name: &str,
        namespaces: &[String],
        remote: Option<&str>,
    ) -> Result<String, String>;

    /// Adopt an existing mesh repo at `mesh_dir` as the current one.
    ///
    /// Owns the WHOLE adoption, pointer write included: `mesh join` calls
    /// this alone, so an implementation that opens without writing the
    /// pointer leaves the project with no way back to its mesh.
    fn open_mesh(&mut self, mesh_dir: &Path) -> Result<(), String>;

    /// Write the project-side pointer file naming this project's mesh.
    fn write_pointer(
        &mut self,
        project: &Path,
        mesh_id: &str,
        mesh_dir: &Path,
        remote: Option<&str>,
    ) -> Result<(), String>;

    /// Read the project-side pointer: `(mesh_id, mesh_dir, remote)`.
    fn read_pointer(
        &self,
        project: &Path,
    ) -> Result<Option<(String, PathBuf, Option<String>)>, String>;

    /// Remove the project-side pointer file. Absent file is not an error.
    fn remove_pointer(&mut self, project: &Path) -> Result<(), String>;

    /// Human-readable status report (see spec §"Status surfaces").
    async fn status(&self) -> Result<String, String>;

    /// Pull-before-push, fail-soft, non-interactive.
    async fn push(&mut self) -> Result<String, String>;

    /// Non-interactive, fail-soft.
    async fn pull(&mut self) -> Result<String, String>;
}
