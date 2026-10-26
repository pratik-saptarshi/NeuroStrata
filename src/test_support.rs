//! Support shared by the whole crate's test build.
//!
//! `std::env` is process-global, so tests that mutate the same variable
//! must not run concurrently: one test's `remove_var` strands another's
//! `expect`. The env-mutating tests over shared variables hold
//! [`ENV_LOCK`] for their whole body — the daemon tests
//! (`NEUROSTRATA_PROJECT_ROOT`), the config tests (`TYPESAFE_API_KEY`),
//! and the store tests (`NEUROSTRATA_WRITE_TIMEOUT_SECS`). New tests
//! that mutate a shared variable must use this lock; a test mutating a
//! variable no other test reads does not race and needs no lock.

/// The crate-wide lock serializing the env-mutating tests that route
/// through it; held for each test's full body. Poisoning (a panic while
/// holding it) is tolerated — `.unwrap_or_else(|e| e.into_inner())`.
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
