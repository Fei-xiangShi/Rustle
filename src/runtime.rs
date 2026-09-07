//! Panic-safe ownership helpers for long-lived Rust worker threads.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread::{self, JoinHandle};

/// Spawn a named worker whose panic is contained at the thread boundary.
///
/// The process-wide panic hook records the detailed crash context before the
/// unwind reaches this wrapper. This boundary then emits a stable safe event
/// and lets the worker's channels disconnect so its owner can recover using
/// the existing typed protocol.
pub(crate) fn spawn_guarded<F>(
    thread_name: &'static str,
    operation: &'static str,
    worker: F,
) -> io::Result<JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || run_guarded(thread_name, operation, worker))
}

fn run_guarded<F>(thread_name: &'static str, operation: &'static str, worker: F)
where
    F: FnOnce(),
{
    if catch_unwind(AssertUnwindSafe(worker)).is_err() {
        tracing::error!(
            event = "worker_panic",
            worker = thread_name,
            operation,
            "Long-lived worker terminated after a captured panic"
        );
    }
}

/// Run a Rust callback body without allowing an unwind to cross an FFI ABI.
pub(crate) fn catch_ffi_unwind<T, F, R>(boundary: &'static str, callback: F, fallback: R) -> T
where
    F: FnOnce() -> T,
    R: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(_) => {
            tracing::error!(
                event = "ffi_callback_panic",
                boundary,
                "Native callback returned through its panic fallback"
            );
            fallback()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn guarded_worker_contains_panics_and_disconnects_its_channel() {
        let (sender, receiver) = mpsc::channel::<()>();
        let handle = spawn_guarded("panic-fixture", "runtime_test", move || {
            let _sender = sender;
            panic!("worker fixture secret=https://example.test/?token=hidden");
        })
        .unwrap();

        assert!(handle.join().is_ok());
        assert!(receiver.recv().is_err());
    }

    #[test]
    fn guarded_worker_runs_successfully() {
        let (sender, receiver) = mpsc::channel();
        let handle = spawn_guarded("success-fixture", "runtime_test", move || {
            sender.send(7).unwrap();
        })
        .unwrap();

        assert!(handle.join().is_ok());
        assert_eq!(receiver.recv().unwrap(), 7);
    }

    #[test]
    fn ffi_boundary_returns_fallback_after_panic() {
        let value = catch_ffi_unwind("fixture_callback", || panic!("fixture"), || 41);
        assert_eq!(value, 41);
    }
}
