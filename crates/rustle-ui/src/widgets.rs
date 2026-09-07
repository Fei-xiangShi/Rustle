//! Reusable UI widgets - composable components without business logic
//!
//! Widgets combine primitives and basic iced elements into reusable UI patterns.
//! They should not contain business logic or depend on the desktop application.
//!
//! # Design Principles
//!
//! - **No business logic**: Widgets receive caller-owned message types
//! - **Generic callbacks**: Use generic Message types or callback functions
//! - **Composable**: Build on primitives and iced's built-in widgets
//! - **Reusable**: Can be used by multiple components
//!
//! # Relationship to Other Layers
//!
//! - **Primitives** (`crate::primitives`): Low-level Widget trait implementations
//! - **Widgets** (this module): Composable UI patterns
//! - **Components** (`crate::components`): Reusable caller-driven composites

mod centered_button_content;
pub mod cover_play_button;
mod crossfade_image;
pub mod detail_header;
mod foreground_reveal;
mod hover_popup;
mod hover_surface;
pub mod multi_track_slider;
pub mod play_mode_button;
pub mod playback_controls;
pub mod playlist_card;
pub mod progress_slider;
pub mod responsive;
pub mod section_header;
mod smooth_scroll;
mod toast;
mod vertical_slider;

// Re-export from primitives for backward compatibility
pub use crate::primitives::{
    ProgressRing, VirtualList, VirtualListState, view_progress_ring_styled,
};

pub use centered_button_content::centered_button_content;
pub use crossfade_image::{ContentPosition, crossfade_image};
pub use foreground_reveal::foreground_reveal;
pub use hover_popup::hover_popup;
pub use hover_surface::hover_surface;
pub use play_mode_button::ButtonSize as PlayModeButtonSize;
pub use playback_controls::ControlSize;
pub use progress_slider::SliderSize;
pub use responsive::{
    hidden_horizontal_scrollbar, hidden_vertical_scrollbar, page_scrollable,
    responsive_card_columns, vertical_scrollbar,
};
pub use smooth_scroll::{scaled_scroll, smooth_scroll};
pub use toast::{Toast, view_toast};
pub use vertical_slider::vertical_slider;
