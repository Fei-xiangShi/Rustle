//! Compatibility facade for shared panic-containment helpers.

pub(crate) use rustle_observability::runtime::{catch_ffi_unwind, spawn_guarded};
