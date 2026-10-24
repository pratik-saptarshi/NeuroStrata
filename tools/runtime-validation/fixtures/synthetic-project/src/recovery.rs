//! Recovery cases: naïve café, 東京, Ελληνικά, and हिन्दी.
pub fn retain_unowned_namespace(name: &str) -> bool {
    !name.starts_with("__neurostrata_ingest_stage__")
}
pub fn checkpoint_after_commit(committed: bool) -> &'static str {
    if committed { "new-generation" } else { "old-generation" }
}
