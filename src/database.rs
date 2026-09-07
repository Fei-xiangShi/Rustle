//! Database module for persistent storage
//! Uses SQLite via sqlx for storing playlists, songs, and playback state

mod connection;
mod error;
mod legacy;
mod migrations;
mod models;
mod ops;
mod repository;
mod schema;

pub use error::StorageResult;
pub use models::*;
pub use repository::Database;
