//! Feature modules - business logic separated from UI
//!
//! Each feature module contains the core logic for a specific functionality.
//! Features should not depend on UI components directly.

pub mod import;
pub mod keybindings;
pub mod lyrics;
pub mod media;
pub mod settings;

pub use import::{extract_track_gain, resolve_track_gain};
pub use keybindings::{Action, KeyBindings, ShortcutScope};

pub use crate::application::tray::TrayCommand;
pub use settings::{CloseBehavior, EqualizerPreset, MusicQuality, PlayMode, ProxyType, Settings};
