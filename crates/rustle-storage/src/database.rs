//! Database module for persistent storage
//! Uses SQLite via sqlx for storing playlists, songs, and playback state

mod adoption;
mod connection;
mod error;
mod legacy;
mod migrations;
mod models;
mod ops;
mod repository;

pub use error::StorageResult;
pub use models::*;
pub use repository::Database;
