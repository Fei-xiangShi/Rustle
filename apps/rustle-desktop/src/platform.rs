//! Compatibility facade and runtime-hook composition for `rustle-platform`.

pub use rustle_platform::{discord, global_hotkeys, keybindings, protocol, theme, tray, window};

pub mod media_controls {
    pub use rustle_platform::media_controls::{
        MediaCommand, MediaHandle, MediaMetadata, MediaPlaybackStatus, MediaState, is_available,
    };

    /// Start native media controls through the process-wide guarded worker
    /// boundary owned by the desktop composition root.
    pub fn start_media_controls(
        window_handle: Option<usize>,
    ) -> (
        MediaHandle,
        tokio::sync::mpsc::UnboundedReceiver<MediaCommand>,
    ) {
        rustle_platform::media_controls::start_media_controls_with(
            window_handle,
            super::guarded_platform_worker,
        )
    }
}

/// Initialize process-wide platform policy and inject native panic containment.
pub fn init() {
    #[cfg(target_os = "windows")]
    rustle_platform::runtime::install_ffi_guard(guarded_platform_ffi);

    rustle_platform::init();
}

/// Open a path in the system file manager while keeping legacy callers
/// fire-and-forget at the desktop compatibility edge.
pub fn open_in_file_manager(path: &std::path::Path) {
    if let Err(error) = rustle_platform::open_in_file_manager(path) {
        tracing::warn!(%error, "Failed to open path in the system file manager");
    }
}

fn guarded_platform_worker(
    thread_name: &'static str,
    operation: &'static str,
    worker: rustle_platform::runtime::WorkerTask,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    crate::runtime::spawn_guarded(thread_name, operation, worker)
}

#[cfg(target_os = "windows")]
fn guarded_platform_ffi(
    boundary: &'static str,
    callback: rustle_platform::runtime::FfiCallback,
    fallback: rustle_platform::runtime::FfiCallback,
) -> isize {
    crate::runtime::catch_ffi_unwind(boundary, callback, fallback)
}
