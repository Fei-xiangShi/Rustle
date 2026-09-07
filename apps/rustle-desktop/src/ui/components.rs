//! UI Components module - business-specific composite components
//!
//! Components combine widgets and primitives with application logic.
//! They are the only layer that should import from `crate::app`.
//!
//! # Design Principles
//!
//! - **Business logic**: Components handle Message mapping and state access
//! - **Composition**: Build on widgets and primitives
//! - **Application-specific**: Depend on `crate::app::Message` and state types
//!
//! # Relationship to Other Layers
//!
//! - **Primitives** (`crate::ui::primitives`): Low-level Widget trait implementations
//! - **Widgets** (`crate::ui::widgets`): Composable UI patterns (no business logic)
//! - **Components** (this module): Business-specific UI with Message handling

pub mod context_menu;
pub mod cover_image;
pub mod detail_card;
pub use rustle_ui::components::detail_description;
pub mod download_panel;
pub mod edit_dialog;
pub mod exit_dialog;
pub use rustle_ui::components::feature_card;
pub mod importing_card;
pub mod login_popup;
pub mod playback_controls;
pub mod player_bar;
pub mod playlist_grid;
pub mod playlist_view;
pub mod queue_panel;
pub mod search_bar;
pub mod sidebar;
pub mod sidebar_resize_handle;
pub mod song_info_dialog;
pub mod source_badge;
pub mod window_controls;
pub use rustle_ui::components::window_drag_region;
pub mod window_resize_handles;

pub use importing_card::ImportingPlaylist;
pub use sidebar::{LibraryItem, NavItem};
