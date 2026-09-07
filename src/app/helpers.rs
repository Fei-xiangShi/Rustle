//! Async helper functions for database operations

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use futures_util::future::BoxFuture;
use iced::Task;

use crate::app::Message;
use crate::audio::chain::AudioProcessingChain;
use crate::database::{
    Database, DbPlaybackState, DbPlaylist, DbSong, DbWatchedFolder, NewPlaylist,
};
use crate::features::PlayMode;
use crate::features::import::{CoverCache, default_cache_dir};
use crate::platform::media_controls::{MediaCommand, MediaHandle, start_media_controls};
use crate::platform::tray::{TrayHandle, TrayState};
use crate::ui::pages;
use crate::utils::format_relative_time;

/// Initialize audio system
pub fn init_audio(
    settings: &crate::features::Settings,
) -> (
    Option<crate::audio::AudioThreadHandle>,
    Option<crate::audio::AudioHandle>,
    AudioProcessingChain,
    Task<Message>,
) {
    // Create shared audio processing chain
    let audio_chain = AudioProcessingChain::new();

    // Apply settings to the chain
    audio_chain.set_equalizer_enabled(settings.playback.equalizer_enabled);
    audio_chain.set_equalizer_gains(settings.playback.equalizer_values);
    audio_chain.set_preamp(settings.playback.equalizer_preamp);

    // Spawn audio thread
    let device_name = settings.system.audio_output_device.as_deref();
    match crate::audio::spawn_audio_thread(device_name, audio_chain.clone()) {
        Ok(mut thread_handle) => {
            let handle = thread_handle.handle.clone();
            let event_rx = thread_handle.take_event_rx();
            tracing::info!("Audio thread spawned successfully");

            // Create event listener task
            let listener_task = if let Some(rx) = event_rx {
                Task::run(
                    async_stream::stream! {
                        let mut rx = rx;
                        loop {
                            if let Some(event) = rx.recv().await {
                                yield event;
                            } else {
                                tracing::info!("Audio event channel closed");
                                break;
                            }
                        }
                    },
                    Message::AudioEvent,
                )
            } else {
                Task::none()
            };

            (
                Some(thread_handle),
                Some(handle),
                audio_chain,
                listener_task,
            )
        }
        Err(e) => {
            tracing::error!("Failed to spawn audio thread: {}", e);
            (None, None, audio_chain, Task::none())
        }
    }
}

/// Initialize database connection
pub fn init_database() -> BoxFuture<'static, crate::database::StorageResult<Database>> {
    Box::pin(async move {
        let data_dir = directories::ProjectDirs::from("com", "rustle", "Rustle")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        std::fs::create_dir_all(&data_dir)?;
        let db_path = data_dir.join("rustle.db");

        tracing::info!(
            event = "database_initializing",
            "Initializing database storage"
        );
        Database::new(db_path).await
    })
}

/// Load all songs from database
pub async fn load_songs(db: Arc<Database>) -> Vec<DbSong> {
    db.get_all_songs().await.unwrap_or_default()
}

/// Load all playlists from database
pub async fn load_playlists(db: Arc<Database>) -> Vec<DbPlaylist> {
    db.get_all_playlists().await.unwrap_or_default()
}

/// Load playback state from database
pub async fn load_playback_state(db: Arc<Database>) -> Option<DbPlaybackState> {
    db.get_playback_state().await.ok()
}

/// Load queue from database
pub async fn load_queue(db: Arc<Database>) -> Vec<DbSong> {
    db.get_queue().await.unwrap_or_default()
}

/// Load download history from database
pub async fn load_download_history(db: Arc<Database>) -> Vec<crate::database::DownloadRow> {
    db.get_all_downloads().await.unwrap_or_default()
}

/// Load all watched local library folders from database.
pub async fn load_watched_folders(db: Arc<Database>) -> Vec<DbWatchedFolder> {
    db.get_all_watched_folders().await.unwrap_or_default()
}

/// Validate all songs in database and mark unavailable local files as missing.
/// Returns the number of songs whose availability state changed.
/// NCM songs (file_path starts with "ncm://") are skipped as they are cloud songs
pub async fn validate_songs(db: Arc<Database>) -> u32 {
    let songs = match db.get_all_songs_including_missing().await {
        Ok(songs) => songs,
        Err(e) => {
            tracing::error!("Failed to load songs for validation: {}", e);
            return 0;
        }
    };

    let mut changed_count = 0u32;

    for song in songs {
        // Skip NCM cloud songs - they don't have local files
        if song.file_path.starts_with("ncm://") {
            continue;
        }

        let path = std::path::Path::new(&song.file_path);
        if !path.exists() {
            if !song.is_missing {
                tracing::info!("Marking song missing (file not found): {}", song.file_path);
                if let Err(e) = db.mark_song_missing_by_path(&song.file_path).await {
                    tracing::error!("Failed to mark song missing {}: {}", song.id, e);
                } else {
                    changed_count += 1;
                }
            }
        } else if song.is_missing {
            tracing::info!("Marking song available again: {}", song.file_path);
            if let Err(e) = db.mark_song_available_by_path(&song.file_path).await {
                tracing::error!("Failed to mark song available {}: {}", song.id, e);
            } else {
                changed_count += 1;
            }
        }
    }

    if changed_count > 0 {
        tracing::info!("Updated availability for {} songs", changed_count);
    }

    changed_count
}

/// Initialize cover cache
pub async fn init_cover_cache() -> anyhow::Result<CoverCache> {
    let cache_dir = default_cache_dir();
    CoverCache::new(cache_dir)
}

/// Initialize font system for lyrics text shaping
/// We do it in a background thread
pub async fn init_font_system() -> crate::features::lyrics::engine::SharedFontSystem {
    tokio::task::spawn_blocking(|| {
        tracing::info!("Initializing FontSystem for lyrics...");
        let start = std::time::Instant::now();
        let mut font_system = cosmic_text::FontSystem::new();
        crate::platform::theme::configure_cosmic_font_system(&mut font_system);
        tracing::info!("FontSystem initialized in {:?}", start.elapsed());

        // Warm up font cache with common character sets
        // This pre-loads font fallback information for CJK and Latin characters
        let font_system: crate::features::lyrics::engine::SharedFontSystem =
            std::sync::Arc::new(parking_lot::Mutex::new(font_system));
        warm_up_font_cache(&font_system);

        // Register as the global singleton — all font consumers use this single instance
        crate::features::lyrics::engine::sdf_cache::set_global_font_system(font_system.clone());

        font_system
    })
    .await
    .expect("FontSystem initialization should not panic")
}

/// Warm up font cache with common character sets
/// This pre-loads font fallback information to avoid lag when switching between languages
fn warm_up_font_cache(font_system: &crate::features::lyrics::engine::SharedFontSystem) {
    use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping};

    let start = std::time::Instant::now();

    // Sample text covering common character sets
    let warmup_texts = [
        // Latin (English)
        "The quick brown fox jumps over the lazy dog",
        // CJK (Chinese)
        "你好世界，这是一段中文歌词测试",
        // CJK (Japanese)
        "こんにちは世界、日本語のテスト",
        // CJK (Korean)
        "안녕하세요 세계, 한국어 테스트",
        // Numbers and punctuation
        "0123456789 !@#$%^&*()[]{}",
    ];

    let mut fs = font_system.lock();
    let metrics = Metrics::new(48.0, 48.0 * 1.4);
    let mut buffer = Buffer::new(&mut fs, metrics);
    buffer.set_size(Some(800.0), None);

    let attrs = Attrs::new().family(Family::SansSerif);

    for text in warmup_texts {
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fs, false);
    }

    tracing::info!("Font cache warmed up in {:?}", start.elapsed());
}

/// Get the global tray handle
pub fn get_tray_handle() -> Option<&'static TrayHandle> {
    crate::platform::tray::get_handle()
}

/// Initialize MPRIS/Media Controls
/// Returns the command receiver wrapped in Arc<Mutex>
pub fn init_mpris(
    window_handle: Option<usize>,
) -> anyhow::Result<(
    MediaHandle,
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<MediaCommand>>>,
)> {
    let (handle, rx) = start_media_controls(window_handle);

    tracing::info!("Media controls service started");
    Ok((handle, std::sync::Arc::new(tokio::sync::Mutex::new(rx))))
}

/// Global MPRIS handle for updates
static MPRIS_HANDLE: OnceLock<MediaHandle> = OnceLock::new();

/// Set the global MPRIS handle
pub fn set_mpris_handle(handle: MediaHandle) {
    MPRIS_HANDLE.set(handle).ok();
}

/// Update tray state with full info including favorite status
pub fn update_tray_state_with_favorite(
    is_playing: bool,
    title: Option<String>,
    artist: Option<String>,
    play_mode: PlayMode,
    ncm_song_id: Option<u64>,
    is_favorited: bool,
    language: crate::i18n::Language,
) {
    if let Some(handle) = get_tray_handle() {
        let state = TrayState {
            is_playing,
            title,
            artist,
            play_mode,
            ncm_song_id,
            is_favorited,
            language,
        };
        handle.update(state);
    }
}

/// Open folder dialog
pub async fn open_folder_dialog() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("选择音乐文件夹")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

/// Create playlist from import results
pub async fn create_playlist_from_import(
    db: Arc<Database>,
    name: String,
    cover_path: Option<String>,
    scanned_paths: Vec<std::path::PathBuf>,
) -> crate::database::StorageResult<i64> {
    let playlist = NewPlaylist {
        name,
        description: None,
        cover_path,
        is_smart: false,
    };

    let playlist_id = db.create_playlist(playlist).await?;

    // Add songs from scanned paths to playlist
    for path in scanned_paths {
        let path_str = path.to_string_lossy().to_string();
        // Find song by path in database
        if let Ok(Some(song)) = db.get_song_by_path(&path_str).await {
            if let Err(e) = db.add_song_to_playlist(playlist_id, song.id).await {
                tracing::warn!("Failed to add song {} to playlist: {}", song.id, e);
            }
        } else {
            tracing::warn!("Song not found in database: {}", path_str);
        }
    }

    Ok(playlist_id)
}

/// Sync an existing local-library playlist to the latest import scan.
pub async fn sync_playlist_from_import(
    db: Arc<Database>,
    playlist_id: i64,
    name: String,
    cover_path: Option<String>,
    scanned_paths: Vec<std::path::PathBuf>,
) -> crate::database::StorageResult<i64> {
    db.update_playlist_full(playlist_id, &name, None, cover_path.as_deref())
        .await?;

    for path in scanned_paths {
        let path_str = path.to_string_lossy().to_string();
        if let Some(song) = db.get_song_by_path(&path_str).await? {
            db.add_song_to_playlist(playlist_id, song.id).await?;
        }
    }

    Ok(playlist_id)
}

/// Load playlist view data from database
pub async fn load_playlist_view(
    db: Arc<Database>,
    playlist_id: i64,
) -> Option<crate::app::PlaylistViewPayload> {
    // Get playlist info
    let playlist = db.get_playlist(playlist_id).await.ok()??;
    let watched_folder = db
        .get_watched_folder_by_playlist(playlist_id)
        .await
        .ok()
        .flatten();

    // Get songs in playlist with added_at date
    let songs = db
        .get_playlist_songs_with_date(playlist_id)
        .await
        .unwrap_or_default();

    let mut images = Vec::new();
    if let (Ok(id), Some(path)) = (u64::try_from(playlist.id), playlist.cover_path.as_deref())
        && crate::image::is_valid_local_path(path)
    {
        images.push((
            crate::image::ImageKind::LocalPlaylistCover,
            id,
            PathBuf::from(path),
        ));
    }
    images.extend(songs.iter().filter_map(|song| {
        let path = song.song.cover_path.as_deref()?;
        if !crate::image::is_valid_local_path(path) {
            return None;
        }
        let (kind, id) =
            crate::image::song_cover_key_for_source(song.song.id, &song.song.file_path)?;
        Some((kind, id, PathBuf::from(path)))
    }));

    // Convert to view models
    let song_views: Vec<pages::PlaylistSongView> = songs
        .iter()
        .enumerate()
        .map(|(i, song)| {
            let meta = crate::metadata::SongMetadata::from(&song.song);
            let added_date = format_relative_time(song.added_at);
            pages::PlaylistSongView::new(crate::ui::components::playlist_view::SongItemData {
                id: song.song.id,
                cover_key: crate::image::song_cover_key_for_source(
                    song.song.id,
                    &song.song.file_path,
                ),
                cover_url: None,
                index: i + 1,
                title: meta.title.clone(),
                artist: meta.artist.clone(),
                album: meta.album.clone(),
                duration: meta.duration_display(),
                added_date,
                source: crate::utils::compute_source(
                    &song.song.file_path,
                    song.song.id,
                    None,
                    None,
                ),
            })
        })
        .collect();

    // Calculate total duration
    let total_secs: u64 = songs.iter().map(|s| s.song.duration_secs as u64).sum();
    let total_mins = total_secs / 60;
    let total_hours = total_mins / 60;
    let remaining_mins = total_mins % 60;
    let total_duration = if total_hours > 0 {
        format!("约 {} 小时 {} 分钟", total_hours, remaining_mins)
    } else {
        format!("{} 分钟", total_mins)
    };

    // Extract color palette from cover image
    let palette = playlist
        .cover_path
        .as_ref()
        .and_then(|p| crate::utils::ColorPalette::from_image_path(std::path::Path::new(p)));

    let view = pages::PlaylistView {
        kind: pages::playlist::DetailPageKind::Playlist,
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        profile_stats: None,
        artist_tab: pages::playlist::ArtistPageTab::TopSongs,
        artist_albums: Vec::new(),
        user_playlists: Vec::new(),
        cover_path: playlist.cover_path,
        owner: "本地".to_string(),
        owner_artist_id: None,
        owner_avatar_path: None,
        creator_id: 0,
        song_count: songs.len() as u32,
        total_duration,
        like_count: String::new(),
        songs: song_views,
        palette,
        is_local: true,
        is_subscribed: false,
        watched_folder_path: watched_folder.as_ref().map(|folder| folder.path.clone()),
        watch_enabled: watched_folder
            .as_ref()
            .map(|folder| folder.enabled)
            .unwrap_or(false),
    };

    Some(crate::app::PlaylistViewPayload { view, images })
}

// ============ Personal FM Mode Helpers ============

use crate::app::state::App;

impl App {
    /// Check if currently in Personal FM mode
    pub fn is_fm_mode(&self) -> bool {
        self.playback.personal_fm_mode
    }

    /// Enter Personal FM mode
    pub fn enter_fm_mode(&mut self) {
        self.playback.personal_fm_mode = true;
        self.playback.ncm_scrobble_source_id = None;
        self.persist_personal_fm_mode(true);
        self.clear_shuffle_cache();
    }

    /// Exit Personal FM mode
    pub fn exit_fm_mode(&mut self) {
        self.playback.personal_fm_mode = false;
        self.playback.ncm_scrobble_source_id = None;
        self.persist_personal_fm_mode(false);
    }

    /// Check if more FM songs should be fetched
    /// Returns true if queue_index is within 3 songs of the end
    pub fn should_fetch_more_fm(&self) -> bool {
        if !self.playback.personal_fm_mode {
            return false;
        }
        let queue_len = self.playback.queue.len();
        let current_idx = self.playback.current_index.unwrap_or(0);
        queue_len.saturating_sub(current_idx) <= 3
    }

    /// Fetch more FM songs (returns Task)
    pub fn fetch_more_fm_songs(&self) -> Task<Message> {
        if let Some(client) = &self.core.ncm_client {
            let client = client.clone();
            Task::perform(
                async move {
                    match client.personal_fm_tracks().await {
                        Ok(songs) if !songs.is_empty() => Some(songs),
                        _ => None,
                    }
                },
                |songs_opt| {
                    if let Some(songs) = songs_opt {
                        // FM mode: append songs without starting playback
                        Message::AddNcmPlaylist(songs, false)
                    } else {
                        Message::NoOp
                    }
                },
            )
        } else {
            Task::none()
        }
    }

    /// Fetch more FM songs and start playing the first new song
    /// Used when FM queue is exhausted
    pub fn fetch_more_fm_songs_and_play(&self) -> Task<Message> {
        if let Some(client) = &self.core.ncm_client {
            let client = client.clone();
            Task::perform(
                async move {
                    match client.personal_fm_tracks().await {
                        Ok(songs) if !songs.is_empty() => Some(songs),
                        _ => None,
                    }
                },
                |songs_opt| {
                    if let Some(songs) = songs_opt {
                        // FM mode: append songs and start playback
                        Message::AddNcmPlaylist(songs, true)
                    } else {
                        Message::ShowErrorToast("获取私人FM歌曲失败".to_string())
                    }
                },
            )
        } else {
            Self::toast_warning(
                self.core
                    .locale
                    .get(crate::i18n::Key::NotLoggedIn)
                    .to_string(),
            )
        }
    }
}
