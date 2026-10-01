//! Compatibility facade and desktop worker-spawn composition for `rustle-audio`.

pub use rustle_audio::*;

/// Spawn the audio subsystem using the process-wide guarded worker boundary.
pub fn spawn_audio_thread(
    device_name: Option<&str>,
    chain: AudioProcessingChain,
) -> PlaybackResult<AudioThreadHandle> {
    #[cfg(target_os = "macos")]
    rustle_audio::install_ffi_guard(guarded_audio_ffi);

    rustle_audio::spawn_audio_thread_with(device_name, chain, guarded_audio_worker)
}

fn guarded_audio_worker(
    thread_name: &'static str,
    operation: &'static str,
    worker: rustle_audio::WorkerTask,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    crate::runtime::spawn_guarded(thread_name, operation, worker)
}

#[cfg(target_os = "macos")]
fn guarded_audio_ffi(
    boundary: &'static str,
    callback: rustle_audio::FfiCallback,
    fallback: rustle_audio::FfiCallback,
) -> i32 {
    crate::runtime::catch_ffi_unwind(boundary, callback, fallback)
}
