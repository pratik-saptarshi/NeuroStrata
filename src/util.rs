//! One lock-failure policy for the whole crate: a poisoned mutex is an
//! error, not a panic.

use std::fmt;
use std::sync::{Mutex, MutexGuard};

/// A mutex was poisoned: a prior holder panicked while holding it, so the
/// state it guards may be mid-update. This is a real [`std::error::Error`]
/// so call sites map it into their module's existing error type with `?`;
/// a call site whose function cannot return an error degrades per the
/// disposition named in its module -- the failure is logged at error
/// level, never silently swallowed, and never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockPoisoned;

impl fmt::Display for LockPoisoned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "mutex poisoned: a prior holder panicked mid-update, \
             so the guarded state is suspect",
        )
    }
}

impl std::error::Error for LockPoisoned {}

/// Lock `m`, or report the poison as an error.
///
/// Policy: a poisoned mutex means a prior holder panicked mid-update; the
/// guarded state is suspect, so the operation fails loudly for *this*
/// caller instead of panicking the process. Recovery (rebuilding the
/// guarded state) is deliberately not attempted here -- whether a given
/// state can be rebuilt is its owner's call, not the lock's.
pub fn lock_or_err<T>(m: &Mutex<T>) -> Result<MutexGuard<'_, T>, LockPoisoned> {
    m.lock().map_err(|_| LockPoisoned)
}

#[cfg(test)]
mod tests {
    use super::lock_or_err;
    use std::sync::Mutex;

    #[test]
    fn helper_locks_a_healthy_mutex() {
        let mutex = Mutex::new(41);
        let mut guard = lock_or_err(&mutex).expect("a healthy mutex locks");
        *guard += 1;
        assert_eq!(*guard, 42);
    }

    #[test]
    fn helper_maps_poison_to_error() {
        let mutex = Mutex::new(());
        // Panic in a scoped thread while it holds the lock: the mutex is
        // poisoned when the thread unwinds out of the guard.
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _guard = mutex.lock().unwrap();
                    panic!("deliberate poison for the lock-policy test");
                })
                .join();
        });
        let err = lock_or_err(&mutex)
            .err()
            .expect("a poisoned mutex is an error, not a guard");
        let message = err.to_string();
        assert!(
            message.contains("poisoned"),
            "the error names the poison: {message}"
        );
        // LockPoisoned is a real std::error::Error, so call sites can map it
        // into their module's error type with `?`.
        let _: &dyn std::error::Error = &err;
    }
}
