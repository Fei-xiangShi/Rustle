//! Unified page loading system
//!
//! This module provides:
//! - "Already on page" detection to prevent redundant loads
//! - Unified handling for local and NCM playlists

use crate::app::state::App;

/// Loading state for playlist pages
#[derive(Debug, Clone, Default)]
pub enum PlaylistLoadState {
    /// No playlist being loaded
    #[default]
    Idle,
    /// Loading playlist (shows skeleton)
    Loading,
    /// Fully loaded
    Ready,
}

impl App {
    /// Check if we're already viewing the specified playlist
    /// Returns true if we should skip loading
    pub fn is_viewing_playlist(&self, playlist_id: i64) -> bool {
        if let Some(current) = &self.ui.playlist_page.current {
            current.id == playlist_id
        } else {
            false
        }
    }

    /// Check if we're already viewing the specified NCM playlist
    /// NCM playlists use negative IDs internally
    pub fn is_viewing_ncm_playlist(&self, ncm_playlist_id: u64) -> bool {
        let internal_id = -(ncm_playlist_id as i64);
        self.is_viewing_playlist(internal_id)
    }
}

/// Convert NCM tracks to PlaylistSongView.
/// Cover resolution is deferred to `image::resolve` at render time.
pub fn convert_ncm_tracks_to_views(
    tracks: &[crate::api::Track],
) -> Vec<crate::ui::pages::PlaylistSongView> {
    convert_ncm_tracks_to_views_with_offset(tracks, 0)
}

/// Convert a batch of NCM tracks while preserving their absolute playlist
/// positions. This is used by the progressive playlist loader.
pub fn convert_ncm_tracks_to_views_with_offset(
    tracks: &[crate::api::Track],
    start_index: usize,
) -> Vec<crate::ui::pages::PlaylistSongView> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let meta = crate::metadata::SongMetadata::from(track);
            let artist_names = track.artist_names();
            let source = crate::utils::compute_source(
                "",
                -(track.id as i64),
                Some(&artist_names),
                Some(&track.title),
            );

            let mut item = crate::ui::components::playlist_view::SongItem::new(
                crate::ui::components::playlist_view::SongItemData {
                    id: -(track.id as i64),
                    cover_key: Some((crate::image::ImageKind::SongCover, track.id)),
                    cover_url: Some(track.cover_url().to_string()),
                    index: start_index + i + 1,
                    title: meta.title.clone(),
                    artist: meta.artist.clone(),
                    album: meta.album.clone(),
                    duration: meta.duration_display(),
                    added_date: String::new(),
                    source,
                },
            );
            item.quality_label = track
                .quality_options
                .iter()
                .max_by_key(|option| option.level.priority())
                .map(|option| option.level.short_name().to_string());
            item.availability_label = track.availability.label().to_string();
            item
        })
        .collect()
}
