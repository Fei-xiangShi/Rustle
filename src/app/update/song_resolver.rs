//! Song resolver module
//!
//! Provides unified song resolution for both local and NCM songs.
//! Handles caching, URL fetching, and cover downloading.
//! Uses SharedBuffer for streaming playback (no file-based streaming).

use std::path::PathBuf;
use std::sync::Arc;

use crate::api::NcmClient;
use crate::api::{NcmQualityLevel, TrackUrl};
use crate::audio::PlaybackError;
use crate::audio::identity::PlaybackContext;
use crate::audio::streaming::{
    AudioCacheKey, SharedBuffer, SharedBufferHealth, StreamingEvent, StreamingEventKind,
    StreamingIdentity, start_buffer_download, wait_for_buffer_playable,
};
use crate::database::DbSong;
use crate::error::{AppError, ErrorCode};

/// Result of resolving a song with streaming support
#[derive(Debug, Clone)]
pub struct ResolvedSong {
    /// Finalized cache file path with the detected audio extension.
    /// `None` means playback remains ring-buffer backed.
    pub finalized_cache_path: Option<String>,
    /// Local cover path or recoverable remote source (if available).
    pub cover_path: Option<String>,
    /// Shared buffer for direct memory playback (None if using cached file)
    pub shared_buffer: Option<SharedBuffer>,
    /// Duration in seconds (from API)
    pub duration_secs: Option<u64>,
    /// Quality requested by settings and the quality actually returned by NCM.
    pub quality: Option<ResolvedAudioQuality>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAudioQuality {
    pub requested: NcmQualityLevel,
    pub actual: NcmQualityLevel,
    pub bitrate: Option<u32>,
    pub size: Option<u64>,
    pub format: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub immerse_type: Option<String>,
}

impl From<&TrackUrl> for ResolvedAudioQuality {
    fn from(url: &TrackUrl) -> Self {
        Self {
            requested: url.requested_level,
            actual: url.level,
            bitrate: (url.rate > 0).then_some(url.rate),
            size: url.size,
            format: url.format.clone(),
            sample_rate: url.sample_rate,
            bit_depth: url.bit_depth,
            channels: url.channels,
            channel_layout: url.channel_layout.clone(),
            immerse_type: url.immerse_type.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ResolvedAudioSource {
    Cached {
        path: PathBuf,
        quality: ResolvedAudioQuality,
    },
    Streaming {
        url: String,
        cache_path: PathBuf,
        cache_key: AudioCacheKey,
        quality: ResolvedAudioQuality,
    },
}

fn cached_quality(
    requested: NcmQualityLevel,
    actual: NcmQualityLevel,
    path: &std::path::Path,
) -> ResolvedAudioQuality {
    ResolvedAudioQuality {
        requested,
        actual,
        bitrate: None,
        size: std::fs::metadata(path).ok().map(|metadata| metadata.len()),
        format: path
            .extension()
            .and_then(|value| value.to_str())
            .map(ToString::to_string),
        sample_rate: None,
        bit_depth: None,
        channels: None,
        channel_layout: None,
        immerse_type: None,
    }
}

/// Resolve the single authoritative audio cache/URL state shared by playback
/// and preload. Fallback and actual-quality selection belong here; callers only
/// decide how to consume a cached path or the shared streaming coordinator.
pub(crate) async fn resolve_audio_source(
    client: &NcmClient,
    song: &DbSong,
) -> Result<ResolvedAudioSource, AppError> {
    let ncm_id = get_ncm_id(song);
    let song_cache_dir = crate::utils::songs_cache_dir();
    std::fs::create_dir_all(&song_cache_dir).map_err(|error| {
        AppError::with_source(
            ErrorCode::StorageOpenFailed,
            "Rustle could not prepare the audio cache",
            error,
        )
    })?;

    let requested_level = client.current_quality_level();
    let requested_stem = format!("{}_{}", ncm_id, requested_level.api_level());
    let requested_candidate = crate::utils::find_cached_audio(&song_cache_dir, &requested_stem);
    if let Some(cached_path) = requested_candidate.as_ref()
        && crate::cache::is_audio_cache_complete(cached_path, ncm_id, requested_level, None)
    {
        return Ok(ResolvedAudioSource::Cached {
            quality: cached_quality(requested_level, requested_level, cached_path),
            path: cached_path.clone(),
        });
    }

    let url = client
        .resolve_track_url(ncm_id, requested_level)
        .await
        .map_err(AppError::from)?;
    let quality = ResolvedAudioQuality::from(&url);
    let actual_stem = format!("{}_{}", ncm_id, url.level.api_level());
    if actual_stem == requested_stem {
        if let Some(cached_path) = requested_candidate {
            if crate::cache::is_audio_cache_complete(&cached_path, ncm_id, url.level, url.size) {
                return Ok(ResolvedAudioSource::Cached {
                    path: cached_path,
                    quality,
                });
            }
            tracing::info!(
                ?cached_path,
                ncm_id,
                "Removing incomplete preferred-quality cache"
            );
            crate::cache::remove_audio_cache(&cached_path);
        }
    } else {
        if let Some(cached_path) = requested_candidate {
            tracing::info!(
                ?cached_path,
                ncm_id,
                "Removing unverifiable cache after actual-quality negotiation"
            );
            crate::cache::remove_audio_cache(&cached_path);
        }
    }
    if actual_stem != requested_stem
        && let Some(cached_path) = crate::utils::find_cached_audio(&song_cache_dir, &actual_stem)
    {
        if crate::cache::is_audio_cache_complete(&cached_path, ncm_id, url.level, url.size) {
            return Ok(ResolvedAudioSource::Cached {
                path: cached_path,
                quality,
            });
        }
        tracing::info!(
            ?cached_path,
            ncm_id,
            "Removing incomplete actual-quality cache"
        );
        crate::cache::remove_audio_cache(&cached_path);
    }

    Ok(ResolvedAudioSource::Streaming {
        url: url.url,
        cache_path: song_cache_dir.join(actual_stem),
        cache_key: AudioCacheKey {
            song_id: ncm_id,
            actual_quality: url.level,
        },
        quality,
    })
}

/// Check if a song needs resolution (NCM song without local file)
pub fn needs_resolution(song: &DbSong) -> bool {
    // NCM songs have negative IDs or file_path starting with "ncm://"
    let is_ncm = song.id < 0 || song.file_path.starts_with("ncm://");

    if !is_ncm {
        return false;
    }

    // Check if we have a valid local file
    if song.file_path.is_empty() || song.file_path.starts_with("ncm://") {
        return true;
    }

    // Check if the file actually exists
    !std::path::Path::new(&song.file_path).exists()
}

/// Get NCM song ID from DbSong
pub fn get_ncm_id(song: &DbSong) -> u64 {
    crate::image::ncm_song_id(song.id, &song.file_path)
        .or_else(|| u64::try_from(song.id).ok())
        .unwrap_or_default()
}

/// Resolve a song with streaming support
///
/// This function:
/// 1. Checks whether the preferred quality is already cached locally
/// 2. Otherwise negotiates the actual quality with the official playback API
/// 3. Reuses an actual-quality cache or streams into one with SharedBuffer
/// 4. Reuses a cached cover or recovers its remote source from track metadata
pub async fn resolve_song(
    client: Arc<NcmClient>,
    song: &DbSong,
    context: PlaybackContext,
    event_tx: tokio::sync::mpsc::Sender<StreamingEvent>,
) -> Result<ResolvedSong, AppError> {
    let ncm_id = get_ncm_id(song);
    let identity = StreamingIdentity::Playback(context.clone());
    // Audio negotiation and stale-cover recovery are independent network work.
    // Resolve them concurrently so image metadata cannot add a serial delay to
    // streaming startup. The image pipeline still owns the actual download.
    let (cover_path, source) = tokio::join!(
        resolve_cover(&client, song, ncm_id),
        resolve_audio_source(&client, song)
    );
    let source = match source {
        Ok(source) => source,
        Err(error) => {
            tracing::error!(
                ncm_id,
                code = %error.code(),
                "Failed to resolve an authoritative audio source"
            );
            let playback_error = PlaybackError::SourceUnavailable(error.user_summary().to_string());
            let _ = event_tx
                .send(StreamingEvent::new(
                    identity,
                    StreamingEventKind::Error(playback_error),
                ))
                .await;
            return Err(error);
        }
    };
    let (shared_buffer, quality) = match source {
        ResolvedAudioSource::Cached { path, quality } => {
            let _ = event_tx
                .send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::Playable,
                ))
                .await;
            let _ = event_tx
                .send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::Complete,
                ))
                .await;
            return Ok(ResolvedSong {
                finalized_cache_path: Some(path.to_string_lossy().to_string()),
                cover_path,
                shared_buffer: None,
                duration_secs: None,
                quality: Some(quality),
            });
        }
        ResolvedAudioSource::Streaming {
            url,
            cache_path,
            cache_key,
            quality,
        } => (
            start_buffer_download(
                url,
                cache_path,
                cache_key,
                quality.bitrate,
                identity,
                Some(event_tx),
            ),
            quality,
        ),
    };

    if !wait_for_buffer_playable(&shared_buffer, 30).await {
        let playback_error = match shared_buffer.health() {
            SharedBufferHealth::Failed(error) => error,
            SharedBufferHealth::Cancelled => {
                PlaybackError::Cancelled(format!("song {ncm_id} streaming startup was cancelled"))
            }
            SharedBufferHealth::CoordinatorStopped => PlaybackError::StreamingFailed(format!(
                "song {ncm_id} streaming coordinator stopped before startup"
            )),
            SharedBufferHealth::Refillable | SharedBufferHealth::Complete => {
                PlaybackError::StreamingFailed(format!(
                    "song {ncm_id} did not reach the streaming startup watermark"
                ))
            }
        };
        tracing::error!(
            ncm_id,
            code = %playback_error.code(),
            "Song did not reach the streaming startup watermark"
        );
        return Err(playback_error.into());
    }

    // The downloader continues filling the bounded window and sparse cache in
    // the background after the decoder has a stable startup reserve.
    Ok(ResolvedSong {
        finalized_cache_path: None,
        cover_path,
        shared_buffer: Some(shared_buffer),
        duration_secs: Some(song.duration_secs as u64),
        quality: Some(quality),
    })
}

async fn resolve_cover(client: &NcmClient, song: &DbSong, ncm_id: u64) -> Option<String> {
    if let Some(path) = crate::image::resolve_cached(crate::image::ImageKind::SongCover, ncm_id) {
        return Some(path.to_string_lossy().to_string());
    }

    if let Some(source) = song.cover_path.as_deref()
        && (crate::image::is_remote_url(source) || crate::image::is_valid_local_path(source))
    {
        return Some(source.to_string());
    }

    match client.track_detail(&[ncm_id]).await {
        Ok(tracks) => tracks
            .into_iter()
            .find(|track| track.id == ncm_id)
            .map(|track| track.cover_url().to_string())
            .filter(|url| crate::image::is_remote_url(url)),
        Err(error) => {
            tracing::warn!(ncm_id, %error, "Failed to recover current-song cover source");
            None
        }
    }
}
