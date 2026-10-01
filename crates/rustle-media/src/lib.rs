//! Local-media adapter for metadata, covers, lyrics, scanning, and watching.

pub mod cover;
pub mod encoding;
pub mod error;
pub mod lyrics;
pub mod metadata;
pub mod scan;
pub mod watch;

pub use error::{MediaError, MediaResult};

#[cfg(test)]
mod test_support;
