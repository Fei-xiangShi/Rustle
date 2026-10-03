//! Root compatibility facade for storage-owned cache behavior and NCM snapshots.

mod audio;
pub use audio::{prepare_song_tags, tagged_audio_cache_store};

pub use rustle_storage::cache::{
    CacheStats, ClearResult, cache_publisher, calculate_cache_stats, cleanup_temp_file,
    clear_all_cache, enforce_cache_limit, is_audio_cache_complete, playlists_cache_dir,
    publish_or_reuse, publish_replace, touch_cache_entry, unique_temp_path,
};

/// Load a cached NCM playlist snapshot.
pub async fn load_ncm_playlist_cache(playlist_id: u64) -> Option<crate::api::PlaylistDetail> {
    let path = playlists_cache_dir().join(format!("{playlist_id}.json"));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::debug!(playlist_id, ?error, "NCM playlist cache miss");
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(detail) => {
            tracing::info!(playlist_id, bytes = bytes.len(), "NCM playlist cache hit");
            Some(detail)
        }
        Err(error) => {
            tracing::warn!(playlist_id, ?error, "NCM playlist cache is invalid");
            None
        }
    }
}

/// Save a complete NCM playlist snapshot for fast subsequent entry.
pub async fn save_ncm_playlist_cache(detail: &crate::api::PlaylistDetail) {
    let dir = playlists_cache_dir();
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        tracing::warn!(
            playlist_id = detail.id,
            "Failed to create NCM playlist cache directory"
        );
        return;
    }
    let path = dir.join(format!("{}.json", detail.id));
    let bytes = match serde_json::to_vec(detail) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                playlist_id = detail.id,
                ?error,
                "Failed to serialize NCM playlist cache"
            );
            return;
        }
    };
    let temporary = unique_temp_path(&path);
    if let Err(error) = tokio::fs::write(&temporary, &bytes).await {
        tracing::warn!(
            playlist_id = detail.id,
            ?error,
            "Failed to write NCM playlist cache"
        );
        cleanup_temp_file(&temporary);
        return;
    }

    if let Err(error) = publish_replace(&temporary, &path) {
        tracing::warn!(
            playlist_id = detail.id,
            ?error,
            "Failed to replace NCM playlist cache"
        );
        return;
    }
    tracing::info!(
        playlist_id = detail.id,
        tracks = detail.tracks.len(),
        "NCM playlist cache saved"
    );
}

/// Preserve the historical startup cleanup behavior while storage stays
/// independent from the root settings model.
pub fn cleanup_temp_files(settings: &crate::features::Settings) -> ClearResult {
    let download_dir = settings.storage.effective_download_dir();
    rustle_storage::cache::cleanup_temp_files(Some(&download_dir))
}
