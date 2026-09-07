//! Database operations organized by entity type

mod downloads;
mod history;
mod playback;
mod playlists;
mod queue;
mod songs;
mod watched_folders;

pub use downloads::*;
pub use history::*;
pub use playback::*;
pub use playlists::*;
pub use queue::*;
pub use songs::*;
pub use watched_folders::*;

use std::time::{SystemTime, UNIX_EPOCH};

/// Get current Unix timestamp
pub fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
