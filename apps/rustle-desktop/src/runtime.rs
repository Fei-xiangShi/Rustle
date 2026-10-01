//! Compatibility facade for shared panic-containment helpers.

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) use rustle_observability::runtime::catch_ffi_unwind;
pub(crate) use rustle_observability::runtime::spawn_guarded;
