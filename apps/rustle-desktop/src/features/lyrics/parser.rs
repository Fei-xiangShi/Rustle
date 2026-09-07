//! Compatibility facade for media-owned lyric parsers and online orchestration.

mod online;

pub use online::*;
pub use rustle_media::lyrics::parser::*;
