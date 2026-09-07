//! Lyrics module - parsing and rendering
//!
//! - `parser`: Multi-format lyrics parsing (LRC, YRC, QRC, TTML, etc.)
//! - `engine`: GPU-accelerated lyrics rendering

pub mod engine;
pub mod parser;

// Re-export commonly used items
pub use parser::*;
