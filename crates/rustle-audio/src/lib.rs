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
mod equalizer;
pub mod events;
mod fade;
mod handle;
pub mod identity;
mod player;
pub mod streaming;
pub mod thread;

pub use analyzer::AudioAnalysisData;
pub use chain::AudioProcessingChain;
pub use events::AudioEvent;
pub use handle::AudioHandle;
pub use player::{PlaybackError, PlaybackInfo, PlaybackResult, PlaybackStatus, get_audio_devices};
pub use streaming::{SharedBuffer, StreamingBuffer};
pub use thread::{AudioThreadHandle, WorkerSpawner, WorkerTask, spawn_audio_thread_with};
