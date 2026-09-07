//! Primitive UI elements - atomic building blocks
//!
//! This module contains the lowest-level UI components that implement
//! iced's `Widget` trait or `canvas::Program` trait directly.
//!
//! # Design Principles
//!
//! - **No business logic**: Primitives must not import desktop application state
//! - **Generic Message types**: Use type parameters for flexibility
//! - **Self-contained**: Each primitive handles its own layout and rendering
//! - **Reusable**: Can be composed by widgets and components
//!
//! # Contents
//!
//! - [`ProgressRing`] - Circular progress indicator using Canvas
//! - [`VirtualList`] - High-performance virtualized list

pub mod progress_ring;
pub mod virtual_list;

pub use progress_ring::{ProgressRing, view_progress_ring_styled};
pub use virtual_list::{VirtualList, VirtualListState};
