//! Media file discovery and metadata extraction
//!
//! Handles finding cover art and lyrics from multiple sources:
//! 1. Embedded metadata (ID3/FLAC tags)
//! 2. External files (same-name, cover.jpg, folder.jpg, etc.)
//! 3. Lyrics files (LRC, YRC, QRC, LYS, TTML)

pub mod cover;
pub mod lyrics;
