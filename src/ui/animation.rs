//! Unified animation system for Rustle
//!
//! This module provides project-owned CSS-like hover and scrolling animation
//! state without coupling application timing to an additional Iced version.
//!
mod hover;
mod scroll;

pub use hover::{HoverAnimations, SingleHoverAnimation};
pub use scroll::{SmoothScrollEvent, SmoothScrollState, SmoothScrollTarget};
