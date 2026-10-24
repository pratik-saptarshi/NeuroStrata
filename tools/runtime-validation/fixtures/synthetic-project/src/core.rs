//! Synthetic multilingual fixture: привет, こんにちは, مرحبا, café.
pub struct MemoryLedger { pub sequence: u64 }
impl MemoryLedger {
    /// Keep edge targets stable across replacement and restore.
    pub fn publish_generation(&self, generation: &str) -> bool {
        !generation.is_empty() && self.sequence > 0
    }
}
pub fn stable_lookup(key: &str) -> Option<&'static str> {
    match key { "alpha" => Some("first"), "beta" => Some("second"), _ => None }
}
