//! Support shared by the whole crate's test build.
//!
//! `std::env` is process-global, so tests that mutate it must not run
//! concurrently: one test's `remove_var` strands another's `expect`.
//! Every env-mutating test in the crate holds [`ENV_LOCK`] for its whole
//! body. The lock lives here, once, rather than per module — two locks in
//! two modules would still race across modules, which is exactly the
//! failure this prevents. New env-mutating tests anywhere in the crate
//! must use this lock.

/// The one crate-wide lock serializing env-mutating tests. Held for the
/// full body of every env-mutating test; poisoning (a panic while
/// holding it) is tolerated at the call site so one failure cannot
/// cascade — callers lock with `.unwrap_or_else(|e| e.into_inner())`.
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
