//! Panic containment hook for native callbacks owned by the audio adapter.

#[cfg(any(target_os = "macos", test))]
pub type FfiCallback = Box<dyn FnOnce() -> i32>;

#[cfg(any(target_os = "macos", test))]
pub type FfiGuard = fn(boundary: &'static str, callback: FfiCallback, fallback: FfiCallback) -> i32;

#[cfg(any(target_os = "macos", test))]
static FFI_GUARD: std::sync::OnceLock<FfiGuard> = std::sync::OnceLock::new();

/// Install the process-owned FFI panic guard. The first installed process
/// policy remains authoritative for the lifetime of the audio subsystem.
#[cfg(target_os = "macos")]
pub fn install(guard: FfiGuard) {
    let _ = FFI_GUARD.set(guard);
}

/// Contain a native callback unwind even when composition has not installed
/// its richer logging hook yet.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn catch_unwind(
    boundary: &'static str,
    callback: FfiCallback,
    fallback: FfiCallback,
) -> i32 {
    if let Some(guard) = FFI_GUARD.get().copied() {
        return guard(boundary, callback, fallback);
    }

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(_) => fallback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_contains_an_unwind_before_it_reaches_the_native_abi() {
        let value = catch_unwind(
            "audio_ffi_fixture",
            Box::new(|| panic!("fixture panic")),
            Box::new(|| 41),
        );

        assert_eq!(value, 41);
    }
}
