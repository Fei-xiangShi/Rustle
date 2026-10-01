//! Audio playback module
//!
//! This module provides audio playback with real-time processing:
//! - `AudioHandle`: Non-blocking audio control from UI thread
//! - `AudioPlayer`: Playback control
//! - `AudioProcessingChain`: Unified audio processing (preamp, EQ, analyzer)
//! - `AudioAnalysisData`: Real-time visualization data
//! - `streaming`: Streaming buffer and download utilities
//! - `events`: Commands and events for audio thread communication
//! - `thread`: Audio thread spawning and management
//!
//! ## Architecture
//! ```text
//! UI Thread (AudioHandle) --[AudioCommand]--> Audio Thread (AudioPlayer)
//! UI Thread              <--[AudioEvent]---- Audio Thread
//! UI Thread              <--[SharedState]--- Audio Thread (non-blocking reads)
//! ```

pub mod analyzer;
pub mod automix;
pub mod chain;
pub mod device;
mod device_watcher;
mod equalizer;
pub mod events;
mod fade;
mod ffi_guard;
mod handle;
pub mod identity;
mod output_recovery;
mod player;
pub mod streaming;
pub mod thread;

pub use analyzer::AudioAnalysisData;
pub use chain::AudioProcessingChain;
pub use device::{AudioDevice, OutputRecoveryReason, get_audio_devices};
pub use events::AudioEvent;
#[cfg(target_os = "macos")]
pub use ffi_guard::{FfiCallback, FfiGuard, install as install_ffi_guard};
pub use handle::AudioHandle;
pub use output_recovery::{OutputGeneration, OutputRecoveryId, OutputRecoveryIntent};
pub use player::{PlaybackError, PlaybackInfo, PlaybackResult, PlaybackStatus};
pub use streaming::{SharedBuffer, StreamingBuffer};
pub use thread::{AudioThreadHandle, WorkerSpawner, WorkerTask, spawn_audio_thread_with};
