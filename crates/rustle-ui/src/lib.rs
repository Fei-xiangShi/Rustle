//! Business-neutral Iced UI infrastructure for Rustle.
//!
//! # Architecture
//!
//! The UI is organized into three layers:
//!
//! - **Primitives** (`primitives`): Low-level Widget trait implementations
//! - **Widgets** (`widgets`): Composable UI patterns without business logic
//! - **Components** (`components`): Reusable composites with caller-owned messages
//! - **Responsive policy** (`responsive`): Pure viewport, density, and layout
//!   contracts shared by the other UI layers

pub mod animation;
pub mod color;
pub mod effects;
pub mod icons;
pub mod primitives;
pub mod responsive;
pub mod theme;
pub mod widgets;

pub mod components;
