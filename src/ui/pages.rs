//! Pages module
//! Full-page views for the music streaming application

pub mod artist;
pub mod audio_engine;
pub mod discover;
pub mod lyrics;
pub mod playlist;
pub mod search;
pub mod settings;
pub mod user;

pub use lyrics::find_current_line;
pub use playlist::{PlaylistSongView, PlaylistView}; // PlaylistSongView used by app when loading playlists
