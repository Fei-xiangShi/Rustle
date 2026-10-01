//! Choose collection artwork without extracting song thumbnails.

use std::path::Path;

use crate::database::{DbPlaylist, DbSong};

pub(super) fn restore_playlist_cover<'a>(
    playlist: &mut DbPlaylist,
    songs: impl Iterator<Item = &'a DbSong>,
) {
    if playlist
        .cover_path
        .as_deref()
        .is_some_and(crate::image::is_valid_local_path)
    {
        return;
    }
    // A valid user-selected cover always wins. A missing generated cover can
    // fall back to a song; this presentation fallback must not rewrite user data.
    playlist.cover_path = songs
        .map(|song| &song.file_path)
        .find(|path| Path::new(path).is_absolute() && Path::new(path).is_file())
        .cloned();
}
