//! One lock-failure policy for the whole crate: a poisoned mutex is an
//! error, not a panic. The helper this module provides is pinned by the
//! tests below.

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
