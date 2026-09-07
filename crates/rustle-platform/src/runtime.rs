//! Runtime hooks supplied by the desktop composition root.

use std::thread::JoinHandle;

pub type WorkerTask = Box<dyn FnOnce() + Send + 'static>;
pub type WorkerSpawner = fn(
    thread_name: &'static str,
    operation: &'static str,
    worker: WorkerTask,
) -> std::io::Result<JoinHandle<()>>;

#[cfg(target_os = "windows")]
pub type FfiCallback = Box<dyn FnOnce() -> isize>;

#[cfg(target_os = "windows")]
pub type FfiGuard =
    fn(boundary: &'static str, callback: FfiCallback, fallback: FfiCallback) -> isize;

#[cfg(target_os = "windows")]
static FFI_GUARD: std::sync::OnceLock<FfiGuard> = std::sync::OnceLock::new();

/// Install the process-owned FFI panic guard. Repeated installation of the
/// same process policy is harmless; the first owner remains authoritative.
#[cfg(target_os = "windows")]
pub fn install_ffi_guard(guard: FfiGuard) {
    let _ = FFI_GUARD.set(guard);
}

/// Contain a native callback unwind even if composition has not yet installed
/// its richer logging hook.
#[cfg(target_os = "windows")]
pub(crate) fn catch_ffi_unwind(
    boundary: &'static str,
    callback: FfiCallback,
    fallback: FfiCallback,
) -> isize {
    if let Some(guard) = FFI_GUARD.get().copied() {
        return guard(boundary, callback, fallback);
    }

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(_) => fallback(),
    }
}
