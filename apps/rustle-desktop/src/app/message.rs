//! Application messages

use std::path::PathBuf;
use std::sync::Arc;

use iced::keyboard::{Key, Modifiers};
use iced::time::Instant;

use crate::api::{
    AlbumDetail, AlbumSummary, ArtistDetail, ArtistSummary, LoginInfo, PlaylistDetail,
    PlaylistSummary, Track, UserDetail,
};
use crate::app::state::UserInfo;
use crate::app::update::audio_preload_manager::PreloadDirection;
use crate::audio::identity::{PlaybackContext, PreloadIdentity};
use crate::database::{Database, DbPlaybackState, DbPlaylist, DbSong, DbWatchedFolder};
use crate::error::AppError;
use crate::features::Action;
use crate::features::import::{CoverCache, ScanProgress, WatchEvent};
use crate::ui::animation::SmoothScrollEvent;
use crate::ui::components::{LibraryItem, NavItem};
use crate::ui::pages;

/// Settings sections for navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Account,
    Playback,
    Display,
    System,
    Network,
    Storage,
    Shortcuts,
    About,
}

impl SettingsSection {
    /// Widget ID for snap_to scrolling in the settings page
    pub fn widget_id(self) -> iced::widget::Id {
        iced::widget::Id::new(match self {
            SettingsSection::Account => "settings_section_account",
            SettingsSection::Playback => "settings_section_playback",
            SettingsSection::Display => "settings_section_display",
            SettingsSection::System => "settings_section_system",
            SettingsSection::Network => "settings_section_network",
            SettingsSection::Storage => "settings_section_storage",
            SettingsSection::Shortcuts => "settings_section_shortcuts",
            SettingsSection::About => "settings_section_about",
        })
    }
}

/// Request identity attached to every asynchronous search response.
///
/// Search requests can finish out of order when the user changes tabs or
/// submits another keyword. Keeping the route context with the response lets
/// the update layer discard stale work safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequestContext {
    pub keyword: String,
    pub tab: crate::app::state::SearchTab,
    pub page: u32,
}

/// Search error payload for async loading
#[derive(Debug, Clone)]
pub struct SearchErrorPayload {
    pub context: SearchRequestContext,
    pub error: AppError,
}

/// Search results payload for async loading
#[derive(Debug, Clone)]
pub struct SearchResultsPayload {
    pub context: SearchRequestContext,
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
    pub playlists: Vec<PlaylistSummary>,
    pub videos: Vec<crate::api::VideoSummary>,
    pub radios: Vec<crate::api::RadioSummary>,
    pub total_count: u32,
}

/// Local playlist view plus image paths discovered while loading it.
#[derive(Debug, Clone)]
pub struct PlaylistViewPayload {
    pub view: pages::PlaylistView,
    pub images: Vec<(crate::image::ImageKind, u64, PathBuf)>,
}

/// Application messages
#[derive(Clone)]
pub enum Message {
    /// No-op message for event interception (modal backdrop clicks)
    Noop,

    // ============ Navigation ============
    /// Navigation menu item selected
    Navigate(NavItem),
    /// Navigate back in history
    NavigateBack,
    /// Navigate forward in history
    NavigateForward,
    /// Library item selected
    LibrarySelect(LibraryItem),
    /// Search query changed
    SearchChanged(String),
    /// Play hero banner playlist
    PlayHero,
    /// Import local playlist
    ImportLocalPlaylist,
    /// Folder selected from dialog
    FolderSelected(Option<PathBuf>),

    // ============ Window ============
    /// Minimize window
    WindowMinimize,
    /// Maximize window
    WindowMaximize,
    /// Actual maximized state reported by the window system
    WindowMaximized(bool),
    /// Start dragging the window
    WindowDrag,
    /// Start resizing the window from a border/corner
    WindowResize(iced::window::Direction),
    /// Mouse released (for sidebar resize end)
    MouseReleased,
    /// Mouse moved (track cursor position for drag area)
    MouseMoved(iced::Point),
    /// Open settings
    OpenSettings,
    /// Open settings and close lyrics page
    OpenSettingsWithCloseLyrics,
    /// Open audio engine page
    OpenAudioEngine,
    /// Toggle the responsive navigation drawer.
    ToggleSidebarDrawer,
    /// Close the responsive navigation drawer without changing the route.
    CloseSidebarDrawer,

    // ============ Settings ============
    /// Update close behavior
    UpdateCloseBehavior(crate::features::CloseBehavior),
    /// Save settings
    SaveSettings,
    /// Update playback settings
    UpdateFadeInOut(bool),
    UpdateAutomixEnabled(bool),
    UpdateVolumeNormalization(bool),
    UpdateMusicQuality(crate::features::MusicQuality),
    UpdateEqualizerEnabled(bool),
    UpdateEqualizerPreset(crate::features::EqualizerPreset),
    UpdateEqualizerValues([f32; 10]),
    UpdateEqualizerPreamp(f32),
    /// Update spectrum analyzer settings
    UpdateSpectrumDecay(f32),
    UpdateSpectrumBarsMode(bool),
    /// Update display settings
    UpdateDarkMode(bool),
    UpdateAppLanguage(String),
    /// Update power saving mode
    UpdatePowerSavingMode(bool),
    /// Update lyrics panel settings
    UpdateLyricsFontFamily(Option<String>),
    /// Update storage settings
    UpdateMaxCacheMb(u64),
    UpdateDownloadDir(Option<String>),
    /// Open folder dialog for download directory
    UpdateDownloadDirDialog,
    UpdateDownloadQuality(crate::features::MusicQuality),
    ClearCache,
    /// Cache cleared result (files_deleted, bytes_freed)
    CacheCleared(usize, u64),
    /// Refresh cache statistics
    RefreshCacheStats,
    /// Enforce cache size limit
    EnforceCacheLimit,
    /// Update system settings
    UpdateAudioOutputDevice(Option<String>),
    /// Toggle Discord Rich Presence
    UpdateDiscordEnabled(bool),
    /// Update network settings
    UpdateProxyType(crate::features::ProxyType),
    UpdateProxyHost(String),
    UpdateProxyPort(String),
    UpdateProxyUsername(String),
    UpdateProxyPassword(String),
    /// Apply proxy settings to the NCM client
    ApplyProxySettings,
    /// Settings navigation
    ScrollToSection(SettingsSection),
    /// Settings page scrolled (y offset in pixels)
    SettingsScrolled(f32),
    /// Trigger measurement of section widget positions
    MeasureSectionPositions,
    /// Section positions measured from the widget tree
    SectionPositionsMeasured(Vec<(SettingsSection, f32)>),
    /// Start editing a keybinding for an action
    StartEditingKeybinding(Action, crate::features::ShortcutScope),
    /// Cancel keybinding edit
    CancelEditingKeybinding,
    /// Key pressed while editing keybinding
    KeybindingKeyPressed(Key, Modifiers),

    // ============ Database ============
    /// Database initialized
    DatabaseReady(Arc<Database>),
    /// Database error
    DatabaseError(AppError),
    /// Songs loaded from database
    SongsLoaded(Vec<DbSong>),
    /// Playlists loaded from database
    PlaylistsLoaded(Vec<DbPlaylist>),
    /// Playback state loaded
    PlaybackStateLoaded(DbPlaybackState),
    /// Queue restored from database on startup (does not auto-play)
    QueueRestored(Vec<DbSong>),
    /// Download history loaded from database
    DownloadsLoaded(Vec<crate::database::DownloadRow>),
    /// NCM song resolved during app startup restore
    /// (queue_index, resolved_result, saved_position_secs, playback_context)
    SongResolvedForRestore(
        usize,
        Result<crate::app::update::song_resolver::ResolvedSong, AppError>,
        f64,
        PlaybackContext,
    ),
    /// Songs validated - invalid entries removed
    SongsValidated(u32),
    /// Queue loaded from playlist (starts playing)
    QueueLoaded(Vec<DbSong>),
    /// Recently played loaded
    RecentlyPlayedLoaded(Vec<DbSong>),
    /// Watched local library folders restored from database
    WatchedFoldersLoaded(Vec<DbWatchedFolder>),

    // ============ Import ============
    /// Cover cache ready
    CoverCacheReady(Arc<CoverCache>),
    /// Start scanning a folder
    StartScan(PathBuf),
    /// Cancel the active folder scan
    CancelScan,
    /// Scan progress update
    ScanProgressUpdate(ScanProgress),
    /// Import completed and local library playlist was created
    ImportedPlaylistCreated(Result<i64, AppError>),
    /// File watcher event
    WatcherEvent(WatchEvent),
    /// Background watcher mutation completed for a local playlist
    WatchedFolderSyncCompleted(Option<i64>),
    /// Show info toast notification
    ShowInfoToast(String),
    /// Show success toast notification
    ShowSuccessToast(String),
    /// Show warning toast notification
    ShowWarningToast(String),
    /// Show error toast notification
    ShowErrorToast(String),
    /// Hide toast notification
    HideToast,
    /// Clear importing playlist from sidebar
    ClearImportingPlaylist,

    // ============ Playlist page ============
    /// Navigate to playlist detail page
    OpenPlaylist(i64),
    /// Request to delete a local playlist (shows confirmation dialog)
    RequestDeletePlaylist(i64),
    /// Confirm playlist deletion
    ConfirmDeletePlaylist,
    /// Playlist deleted confirmation
    PlaylistDeleted(i64),
    /// Playlist view loaded from database
    PlaylistViewLoaded(PlaylistViewPayload),
    /// NCM playlist songs converted for display.
    NcmPlaylistSongsReady(i64, Vec<crate::ui::pages::PlaylistSongView>),
    /// A batch of NCM playlist songs converted for display.
    /// The boolean marks the final batch so the page can become ready without
    /// waiting for the entire list to be converted in one UI update.
    NcmPlaylistSongsChunk(
        u64,
        i64,
        Vec<crate::api::Track>,
        Vec<crate::ui::pages::PlaylistSongView>,
        bool,
    ),
    /// Play a specific song
    PlaySong(i64),
    /// Hover over a song in playlist
    HoverSong(Option<i64>),
    /// Hover over an icon button
    HoverIcon(Option<IconId>),
    /// Hover over a sidebar item
    HoverSidebar(Option<SidebarId>),
    /// Toggle the "My Playlists" sidebar section
    ToggleMyPlaylistsSection,
    /// Toggle the "Collected Playlists" sidebar section
    ToggleCollectedPlaylistsSection,
    /// Animation tick
    AnimationTick(Instant),
    /// Smooth-scroll input from a native or virtual scroll surface.
    SmoothScroll(SmoothScrollEvent),
    /// Toggle playlist search input expansion
    TogglePlaylistSearch,
    /// Playlist search query changed
    PlaylistSearchChanged(String),
    /// Submit playlist search (Enter key)
    PlaylistSearchSubmit,
    /// Playlist search input lost focus
    PlaylistSearchBlur,
    /// Toggle playlist description expand/collapse
    ToggleDescriptionExpand,

    // ============ Edit dialog ============
    /// Edit playlist (open edit dialog)
    EditPlaylist(i64),
    /// Edit form: name changed
    EditPlaylistNameChanged(String),
    /// Edit form: description changed
    EditPlaylistDescriptionChanged(String),
    /// Edit form: watched library toggle changed
    EditPlaylistWatchEnabledChanged(bool),
    /// Pick cover image
    PickCoverImage,
    /// Cover image picked
    CoverImagePicked(Option<String>),
    /// Save playlist edits
    SavePlaylistEdits,
    /// Playlist updated in database (with playlist id to reload)
    PlaylistUpdated(i64),

    // ============ Lyrics page ============
    /// Open lyrics page
    OpenLyricsPage,
    /// Show the lyric renderer inside the already-open full-screen player
    ShowLyricsContent,
    /// Return from lyric rendering to the artwork-focused player
    ShowLyricsArtwork,
    /// Close lyrics page
    CloseLyricsPage,
    /// Scroll lyrics manually (delta in pixels)
    LyricsScroll(f32),
    /// Real rendered lyrics viewport resized
    LyricsViewportResized(iced::Size),
    /// Window resized (for lyrics viewport calculation)
    WindowResized(iced::Size),
    /// Font system initialized asynchronously (for lyrics text shaping)
    LyricsFontSystemReady(crate::features::lyrics::engine::SharedFontSystem),
    /// Lyrics loaded from online (song_id, lyrics_lines)
    LyricsLoaded(i64, Vec<crate::application::lyrics::LyricLine>),
    /// Lyrics loading failed
    LyricsLoadFailed(i64, AppError),
    /// Start online lyrics fetch for display loading (song_id, ncm_id)
    FetchLyricsOnline(i64, u64),
    /// Warm lyrics cache for a song in the background (song_id, ncm_id)
    WarmLyricsCache(i64, u64),
    /// Background lyrics cache warmup completed
    LyricsWarmupFinished(i64, Result<(), AppError>),
    /// Local/cached lyrics loaded asynchronously (song_id, lyrics_lines)
    LocalLyricsReady(i64, Vec<crate::application::lyrics::LyricLine>),
    /// Engine lines pre-computed asynchronously (song_id, engine_lines)
    LyricsEngineLinesReady(
        i64,
        std::sync::Arc<Vec<crate::features::lyrics::engine::LyricLineData>>,
    ),
    /// 异步预计算的 shaped lines (song_id, shaped_lines, pre_generated_sdf_bitmaps)
    /// 文本布局的唯一数据源，在后台线程计算
    /// 包含预生成的 SDF 位图，避免首次渲染时阻塞主线程
    LyricsShapedLinesReady(
        i64,
        u64,
        std::sync::Arc<Vec<crate::features::lyrics::engine::CachedShapedLine>>,
        std::collections::HashMap<
            cosmic_text::CacheKey,
            crate::features::lyrics::engine::sdf_generator::SdfBitmap,
        >,
        f32,
        f32,
        Option<String>,
    ),
    /// Background colors extracted asynchronously (song_id, cover_path, primary, secondary, tertiary)
    LyricsBackgroundReady(i64, String, [f32; 4], [f32; 4], [f32; 4]),
    /// Album cover image loaded asynchronously for lyrics background (song_id, cover_path, image_data, width, height)
    LyricsCoverImageReady(i64, String, Vec<u8>, u32, u32),

    // ============ Playback controls ============
    /// Toggle play/pause
    TogglePlayback,
    /// Play next song
    NextSong,
    /// Play previous song
    PrevSong,
    /// Update seek preview position while dragging (0.0 to 1.0)
    SeekPreview(f32),
    /// Finish seeking and apply the preview position
    SeekRelease,
    /// Set volume (0.0 to 1.0)
    SetVolume(f32),
    /// Playback tick (for progress updates)
    PlaybackTick,
    /// Toggle queue panel visibility
    ToggleQueue,
    /// Cycle to next play mode
    CyclePlayMode,
    /// Audio preload ready (local file cached) - (queue_index, file_path, direction)
    PreloadReady(
        usize,
        String,
        PreloadDirection,
        Option<crate::app::update::song_resolver::ResolvedAudioQuality>,
        PreloadIdentity,
    ),
    /// Audio preload ready with SharedBuffer for streaming playback
    /// (queue_index, finalized_cache_path, direction, shared_buffer, duration_secs, identity)
    /// `None` means the track is still ring-buffer backed.
    PreloadBufferReady(
        usize,
        Option<String>,
        PreloadDirection,
        crate::audio::SharedBuffer,
        u64,
        Option<crate::app::update::song_resolver::ResolvedAudioQuality>,
        PreloadIdentity,
    ),
    /// Audio preload failed - (queue_index, direction, identity)
    PreloadAudioFailed(usize, PreloadDirection, PreloadIdentity),

    // ============ Queue management ============
    /// Play entire playlist (replace queue with playlist songs)
    PlayPlaylist(i64),
    /// Play a song from queue by index
    PlayQueueIndex(usize),
    /// Song resolved with streaming support (index, finalized_cache_path, cover_path, shared_buffer, duration_secs)
    SongResolvedStreaming(
        usize,
        Option<String>,
        Option<String>,
        Option<crate::audio::SharedBuffer>,
        Option<u64>,
        Option<crate::app::update::song_resolver::ResolvedAudioQuality>,
        PlaybackContext,
    ),
    /// Song resolution failed (playback context, explicit reason)
    SongResolveFailed(PlaybackContext, AppError),
    /// Remove song from queue by index
    RemoveFromQueue(usize),
    /// Clear the entire queue
    ClearQueue,

    // ============ Download ============
    /// Download a single song by song_id
    DownloadSong(i64),
    /// Download URL resolved (song_id, ncm_id, url, metadata)
    DownloadUrlResolved(i64, u64, String, crate::metadata::SongMetadata),
    /// Download all songs in a playlist
    DownloadPlaylist(i64),
    /// Request download playlist with confirmation (playlist_id, name, song_count)
    RequestDownloadPlaylist(i64, String, u32),
    /// Confirm batch download from confirmation dialog
    ConfirmDownloadPlaylist,
    /// Playlist download URLs resolved (vec of (song_id, ncm_id, url, metadata))
    DownloadBatchEnqueue(Vec<(i64, u64, String, crate::metadata::SongMetadata)>),
    /// Cancel a download
    DownloadCancel(i64),
    /// Download progress update (song_id, downloaded_bytes, total_bytes)
    DownloadProgress(i64, u64, u64),
    /// Download completed (song_id, file_path)
    DownloadCompleted(i64, String),
    /// Download failed (song_id, error_message)
    DownloadError(i64, AppError),
    /// Delete a download history entry by song_id
    DeleteDownloadHistory(i64),
    /// Switch download panel tab
    SwitchDownloadTab(crate::app::DownloadTab),

    // ============ Keyboard events ============
    /// Keyboard key pressed
    KeyPressed(Key, Modifiers),
    /// Execute a keybinding action
    ExecuteAction(Action),
    /// Operating-system global shortcut pressed
    GlobalHotkeyPressed(u32),

    // ============ Exit dialog ============
    /// Request to close the window (triggers exit dialog if needed)
    RequestClose,
    /// Confirm exit and close the application
    ConfirmExit,
    /// Minimize to system tray
    MinimizeToTray,
    /// Toggle "remember my choice" checkbox in exit dialog
    ExitDialogRememberChanged(bool),

    // ============ System Tray ============
    /// Initialize the tray after Winit has entered its active event loop.
    InitializeTray,
    /// Tray service started
    TrayStarted(
        std::sync::Arc<
            tokio::sync::Mutex<tokio::sync::mpsc::Receiver<crate::features::TrayCommand>>,
        >,
    ),
    /// Tray initialization or runtime recovery failed.
    TrayUnavailable(String),
    /// Tray command received
    TrayCommand(crate::features::TrayCommand),

    // ============ Media Controls ============
    /// Media controls service started
    MprisStartedWithHandle(
        crate::platform::media_controls::MediaHandle,
        std::sync::Arc<
            tokio::sync::Mutex<
                tokio::sync::mpsc::UnboundedReceiver<crate::platform::media_controls::MediaCommand>,
            >,
        >,
    ),
    /// Media controls command received
    MprisCommand(crate::platform::media_controls::MediaCommand),
    /// Show window from tray
    ShowWindow,
    /// Show hidden window or focus the existing visible window
    ShowOrFocusWindow,
    /// Toggle window visibility
    ToggleWindow,
    /// Initialize native global shortcuts after the main window is open
    InitializeGlobalHotkeys,
    /// Window finished showing and is ready to render
    WindowShown,
    /// Window gained focus
    WindowFocused,
    /// Window lost focus
    WindowUnfocused,
    /// Window operation completed (for debouncing)
    WindowOperationComplete,

    // ============ NCM Login ============
    /// Try to auto-login with saved cookies
    TryAutoLogin(u8),
    /// Auto login result
    AutoLoginResult(Option<LoginInfo>, u8),
    /// Request QR code for login
    RequestQrCode,
    /// QR code generated
    QrCodeReady(PathBuf, String),
    /// Check QR code scan status
    CheckQrStatus(String),
    /// QR code login result
    QrLoginResult(QrLoginStatus),
    /// Login successful
    LoginSuccess(LoginInfo),
    /// Logout
    Logout,
    /// User info loaded
    UserInfoLoaded(UserInfo),
    /// Toggle login popup visibility
    ToggleLoginPopup,
    /// No operation (placeholder)
    NoOp,

    /// Toggle favorite status for a song
    ToggleFavorite(u64),
    /// Favorite status changed
    FavoriteStatusChanged(u64, bool),
    /// Play NCM song
    PlayNcmSong(Track),
    /// Add NCM songs to queue
    AddNcmPlaylist(Vec<Track>, bool),
    AddNcmPlaylistWithSource(Vec<Track>, bool, Option<u64>),
    /// Open NCM playlist detail page
    OpenNcmPlaylist(u64),
    /// Open user detail page
    OpenUser(u64),
    /// Open artist detail page
    OpenArtist(u64),
    /// Open album detail page
    OpenAlbum(u64),
    /// Resolve artist by name then open detail page
    OpenArtistByName(String),
    // ============ Cloud Playlist ============
    /// User playlists loaded
    UserPlaylistsLoaded(Vec<PlaylistSummary>),
    /// NCM playlist detail loaded
    NcmPlaylistDetailLoaded(u64, PlaylistDetail),
    /// Cached NCM playlist snapshot loaded before a refresh.
    NcmPlaylistCacheLoaded(u64, u64, Option<PlaylistDetail>),
    /// NCM playlist metadata and track IDs loaded independently.
    NcmPlaylistPreviewLoaded(u64, PlaylistDetail, Vec<u64>),
    /// Creator details for an NCM playlist, tied to the playlist request
    /// generation for stale-response protection.
    NcmPlaylistCreatorDetailLoaded(u64, i64, UserDetail),
    /// NCM playlist request failed. The generation prevents an old request
    /// from changing a page that has since been opened again.
    NcmPlaylistLoadFailed(u64, i64, AppError),
    /// Artist detail loaded
    ArtistDetailLoaded(ArtistDetail),
    /// Album detail loaded
    AlbumDetailLoaded(AlbumDetail),
    /// Artist albums loaded for artist page
    ArtistAlbumsLoaded(i64, Vec<AlbumSummary>),
    /// User page detail loaded
    UserPageDetailLoaded(i64, UserDetail),
    /// User playlists loaded for user page
    UserPagePlaylistsLoaded(i64, Vec<PlaylistSummary>),
    /// Artist detail loaded for a user page
    UserArtistDetailLoaded(i64, ArtistDetail),

    // ============ Unified Image Pipeline ============
    /// Image download completed and cached locally (generation, scope, kind, id, local_path)
    ImageDownloadReady(
        u64,
        crate::app::state::ImageRequestScope,
        crate::image::ImageKind,
        u64,
        PathBuf,
    ),
    /// Image download failed and should be eligible for retry (generation, scope, kind, id)
    ImageDownloadFailed(
        u64,
        crate::app::state::ImageRequestScope,
        crate::image::ImageKind,
        u64,
    ),
    /// Image references entering a virtual list's visible/overscan range.
    ImageViewportChanged(u64, Vec<(crate::image::ImageKind, u64, String)>),

    /// Toggle playlist subscription (subscribe/unsubscribe)
    TogglePlaylistSubscribe(i64),
    /// Playlist subscription status changed
    PlaylistSubscribeChanged(i64, bool),

    // ============ Discover Page ============
    /// Recommended playlists loaded (for logged-in users)
    RecommendedPlaylistsLoaded(u64, Vec<PlaylistSummary>),
    /// First Daily Recommend track loaded for the feature-card cover.
    DailyRecommendPreviewLoaded(u64, Option<Track>),
    /// Personal FM tracks prefetched for the feature-card cover and first playback.
    PersonalFmPreviewLoaded(u64, Vec<Track>),
    /// Private Radar playlist metadata loaded.
    PrivateRadarLoaded(u64, Option<PlaylistSummary>),
    /// Ordinary hot playlists loaded.
    HotPlaylistsLoaded(u64, Vec<PlaylistSummary>),
    /// Official high-quality playlists loaded.
    OfficialPlaylistsLoaded(u64, Vec<PlaylistSummary>),
    /// Hover over a discover playlist card
    HoverDiscoverPlaylist(Option<u64>),
    /// Play a playlist from discover page
    PlayDiscoverPlaylist(u64),
    /// See all recommended playlists
    SeeAllRecommended,
    /// See all hot playlists
    SeeAllHot,
    /// See all official high-quality playlists
    SeeAllOfficial,

    // ============ Search Page ============
    /// Submit search query (Enter pressed in search bar)
    SearchSubmit,
    /// Change search tab
    SearchTabChanged(crate::app::state::SearchTab),
    /// Search results loaded
    SearchResultsLoaded(SearchResultsPayload),
    /// Search failed
    SearchFailed(SearchErrorPayload),
    /// Change search page (pagination)
    SearchPageChanged(u32),
    /// Hover over search result song
    HoverSearchSong(Option<u64>),
    /// Hover over search result card (album/playlist)
    HoverSearchCard(Option<u64>),
    /// Play search result song
    PlaySearchSong(u64),
    /// Open search result album/playlist
    OpenSearchResult(u64, crate::app::state::SearchTab),
    /// Switch artist page tab
    SwitchArtistTab(crate::ui::pages::playlist::ArtistPageTab),

    // ============ Sidebar Resize ============
    /// Start dragging sidebar resize handle
    SidebarResizeStart,
    /// Stop dragging sidebar resize handle
    SidebarResizeEnd,
    // ============ Player Events (Event-Driven Architecture) ============
    /// Streaming download event (song_id, event)
    StreamingEvent(crate::audio::streaming::StreamingEvent),
    /// Audio thread event
    AudioEvent(crate::audio::AudioEvent),

    // ============ Discord Rich Presence ============
    /// Update Discord presence with current playback state
    DiscordUpdatePresence,
    /// Clear Discord presence
    DiscordClearPresence,

    // ============ Protocol (rustle://) ============
    /// URI received from OS protocol handler or IPC
    UriReceived(String),

    // ============ Context Menu ============
    /// Show context menu for a local song (triggered by right-click)
    RightClickSong(i64),
    /// Close context menu
    CloseContextMenu,
    /// Context menu item selected
    ContextMenuAction(ContextMenuAction, i64),
    /// Confirm adding a song to a playlist from the picker modal (song_id, playlist_id)
    PlaylistPickerConfirm(i64, i64),
    /// Add an NCM song to an NCM online playlist (song_ncm_id, ncm_playlist_id)
    AddToNcmPlaylist(u64, u64),
    /// Result of adding song to NCM playlist
    NcmPlaylistAddResult(u64, u64, Result<(), AppError>),

    // ============ Overlay System (Unified) ============
    /// Dismiss the topmost dismissible overlay
    DismissTopModal,

    // ============ Song Edit ============
    /// Open song edit dialog
    EditSongTags(i64),
    /// Open edit dialog with resolved metadata (DbSong, SongMetadata, cover_path)
    OpenSongEditDialog(
        Box<(
            crate::database::DbSong,
            crate::metadata::SongMetadata,
            Option<PathBuf>,
        )>,
    ),
    /// Song edit field changed
    SongEditFieldChanged {
        song_id: i64,
        field: String,
        value: String,
    },
    /// Song cover replaced
    SongEditCoverReplaced(i64, PathBuf),
    /// Pick a cover image for song edit
    PickSongEditCover(i64),
    /// Save song edits to file
    SaveSongEdits(i64),
    /// Song edits saved successfully
    SongEditsSaved(i64),
    /// Song edits save failed
    SongEditsFailed {
        song_id: i64,
        error: AppError,
    },
}

/// Context menu action types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    PlayNow,
    PlayNext,
    AddToFavorites,
    AddToPlaylist,
    ViewArtist,
    ViewAlbum,
    ShowInFolder,
    EditSongTags,
    Download,
    RemoveFromList,
}

/// Icon identifiers for hover tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconId {
    PlayButton,
    Edit,
    Delete,
    Search,
    Like,
    Download,
}

/// Sidebar item identifiers for hover tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidebarId {
    Nav(usize),        // Navigation items (Home, Discover, Radio)
    Library(usize),    // Library items
    Playlist(i64),     // Playlist by ID
    UserPlaylist(u64), // NCM Playlist by ID
}

/// QR login status
#[derive(Debug, Clone)]
pub enum QrLoginStatus {
    /// Waiting for scan (801)
    WaitingForScan,
    /// Scanned, waiting for confirmation (802)
    WaitingForConfirm,
    /// Expired (800)
    Expired,
    /// Success (803)
    Success,
    /// Error
    Error(AppError),
}

// Manual Debug implementation to avoid slow formatting of large data structures
// This prevents the "Slow Debug implementation" warning from iced_debug
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use a macro to reduce boilerplate for simple variants
        macro_rules! simple {
            ($name:literal) => { write!(f, $name) };
            ($name:literal, $($arg:tt)*) => { write!(f, concat!($name, "({})"), format_args!($($arg)*)) };
        }

        match self {
            // High-frequency messages - keep minimal (no data)
            Self::AnimationTick(_) => simple!("AnimationTick"),
            Self::SmoothScroll(_) => simple!("SmoothScroll"),
            Self::PlaybackTick => simple!("PlaybackTick"),
            Self::Noop => simple!("Noop"),
            Self::NoOp => simple!("NoOp"),

            // Large Vec data - only show count
            Self::SongsLoaded(v) => simple!("SongsLoaded", "{} songs", v.len()),
            Self::PlaylistsLoaded(v) => simple!("PlaylistsLoaded", "{} playlists", v.len()),
            Self::QueueRestored(v) => simple!("QueueRestored", "{} songs", v.len()),
            Self::SongResolvedForRestore(idx, result, pos, context) => {
                simple!(
                    "SongResolvedForRestore",
                    "idx={}, resolved={}, pos={:.1}s, generation={}",
                    idx,
                    result.is_ok(),
                    pos,
                    context.generation.0
                )
            }
            Self::QueueLoaded(v) => simple!("QueueLoaded", "{} songs", v.len()),
            Self::RecentlyPlayedLoaded(v) => simple!("RecentlyPlayedLoaded", "{} songs", v.len()),
            Self::UserPlaylistsLoaded(v) => simple!("UserPlaylistsLoaded", "{} playlists", v.len()),
            Self::AddNcmPlaylist(v, play) => {
                simple!("AddNcmPlaylist", "{} songs, play={}", v.len(), play)
            }
            Self::AddNcmPlaylistWithSource(v, play, source) => {
                simple!(
                    "AddNcmPlaylistWithSource",
                    "{} songs, play={}, source={:?}",
                    v.len(),
                    play,
                    source
                )
            }

            // Arc-wrapped types - just show variant name
            Self::DatabaseReady(_) => simple!("DatabaseReady"),
            Self::CoverCacheReady(_) => simple!("CoverCacheReady"),
            Self::TrayStarted(_) => simple!("TrayStarted"),

            // Complex types - show key identifier only
            Self::NcmPlaylistDetailLoaded(generation, d) => simple!(
                "NcmPlaylistDetailLoaded",
                "id={}, generation={}",
                d.id,
                generation
            ),
            Self::NcmPlaylistCacheLoaded(generation, id, cached) => {
                simple!(
                    "NcmPlaylistCacheLoaded",
                    "id={}, hit={}, generation={}",
                    id,
                    cached.is_some(),
                    generation
                )
            }
            Self::NcmPlaylistPreviewLoaded(generation, d, ids) => simple!(
                "NcmPlaylistPreviewLoaded",
                "id={}, {} track ids, generation={}",
                d.id,
                ids.len(),
                generation
            ),
            Self::NcmPlaylistCreatorDetailLoaded(generation, id, detail) => simple!(
                "NcmPlaylistCreatorDetailLoaded",
                "playlist_id={}, user_id={}, generation={}",
                id,
                detail.user_id,
                generation
            ),
            Self::NcmPlaylistLoadFailed(generation, id, _) => simple!(
                "NcmPlaylistLoadFailed",
                "id={}, generation={}",
                id,
                generation
            ),
            Self::ArtistDetailLoaded(d) => simple!("ArtistDetailLoaded", "id={}", d.id),
            Self::AlbumDetailLoaded(d) => simple!("AlbumDetailLoaded", "id={}", d.id),
            Self::ArtistAlbumsLoaded(id, albums) => {
                simple!(
                    "ArtistAlbumsLoaded",
                    "page_id={}, albums={}",
                    id,
                    albums.len()
                )
            }
            Self::UserPageDetailLoaded(id, d) => {
                simple!(
                    "UserPageDetailLoaded",
                    "page_id={}, user_id={}",
                    id,
                    d.user_id
                )
            }
            Self::UserPagePlaylistsLoaded(id, playlists) => {
                simple!(
                    "UserPagePlaylistsLoaded",
                    "page_id={}, playlists={}",
                    id,
                    playlists.len()
                )
            }
            Self::UserArtistDetailLoaded(id, d) => {
                simple!(
                    "UserArtistDetailLoaded",
                    "page_id={}, artist_id={}",
                    id,
                    d.id
                )
            }
            Self::PlaylistViewLoaded(payload) => {
                simple!("PlaylistViewLoaded", "id={}", payload.view.id)
            }
            Self::NcmPlaylistSongsReady(id, songs) => {
                simple!("NcmPlaylistSongsReady", "id={}, {} songs", id, songs.len())
            }
            Self::NcmPlaylistSongsChunk(generation, id, tracks, songs, last) => simple!(
                "NcmPlaylistSongsChunk",
                "id={}, {} tracks, {} songs, last={}, generation={}",
                id,
                tracks.len(),
                songs.len(),
                last,
                generation
            ),
            Self::PlaybackStateLoaded(_) => simple!("PlaybackStateLoaded"),
            Self::ScanProgressUpdate(_) => simple!("ScanProgressUpdate"),
            Self::WatchedFoldersLoaded(v) => {
                simple!("WatchedFoldersLoaded", "{} folders", v.len())
            }
            Self::LoginSuccess(_) => simple!("LoginSuccess"),
            Self::UserInfoLoaded(_) => simple!("UserInfoLoaded"),
            Self::AutoLoginResult(r, retry) => simple!(
                "AutoLoginResult",
                "success={}, retry={}",
                r.is_some(),
                retry
            ),
            Self::PlayNcmSong(s) => simple!("PlayNcmSong", "id={}", s.id),

            // Navigation
            Self::Navigate(nav) => simple!("Navigate", "{:?}", nav),
            Self::NavigateBack => simple!("NavigateBack"),
            Self::NavigateForward => simple!("NavigateForward"),
            Self::LibrarySelect(item) => simple!("LibrarySelect", "{:?}", item),
            Self::SearchChanged(_) => simple!("SearchChanged"),
            Self::PlayHero => simple!("PlayHero"),
            Self::ImportLocalPlaylist => simple!("ImportLocalPlaylist"),
            Self::FolderSelected(p) => simple!("FolderSelected", "{:?}", p.as_ref().map(|_| "...")),

            // Window
            Self::WindowMinimize => simple!("WindowMinimize"),
            Self::WindowMaximize => simple!("WindowMaximize"),
            Self::WindowMaximized(maximized) => simple!("WindowMaximized", "{}", maximized),
            Self::WindowDrag => simple!("WindowDrag"),
            Self::WindowResize(direction) => simple!("WindowResize", "{:?}", direction),
            Self::MouseReleased => simple!("MouseReleased"),
            Self::MouseMoved(_) => simple!("MouseMoved"),
            Self::OpenSettings => simple!("OpenSettings"),
            Self::OpenSettingsWithCloseLyrics => simple!("OpenSettingsWithCloseLyrics"),
            Self::ToggleSidebarDrawer => simple!("ToggleSidebarDrawer"),
            Self::CloseSidebarDrawer => simple!("CloseSidebarDrawer"),
            Self::OpenAudioEngine => simple!("OpenAudioEngine"),

            // Settings - most are simple
            Self::UpdateCloseBehavior(b) => simple!("UpdateCloseBehavior", "{:?}", b),
            Self::SaveSettings => simple!("SaveSettings"),
            Self::UpdateFadeInOut(b) => simple!("UpdateFadeInOut", "{}", b),
            Self::UpdateAutomixEnabled(b) => simple!("UpdateAutomixEnabled", "{}", b),
            Self::UpdateVolumeNormalization(b) => simple!("UpdateVolumeNormalization", "{}", b),
            Self::UpdateMusicQuality(q) => simple!("UpdateMusicQuality", "{:?}", q),
            Self::UpdateEqualizerEnabled(b) => simple!("UpdateEqualizerEnabled", "{}", b),
            Self::UpdateEqualizerPreset(p) => simple!("UpdateEqualizerPreset", "{:?}", p),
            Self::UpdateEqualizerValues(_) => simple!("UpdateEqualizerValues"),
            Self::UpdateEqualizerPreamp(v) => simple!("UpdateEqualizerPreamp", "{:.1}", v),
            Self::UpdateSpectrumDecay(v) => simple!("UpdateSpectrumDecay", "{:.2}", v),
            Self::UpdateSpectrumBarsMode(b) => simple!("UpdateSpectrumBarsMode", "{}", b),
            Self::UpdateDarkMode(b) => simple!("UpdateDarkMode", "{}", b),
            Self::UpdateAppLanguage(l) => simple!("UpdateAppLanguage", "{}", l),
            Self::UpdatePowerSavingMode(b) => simple!("UpdatePowerSavingMode", "{}", b),
            Self::UpdateLyricsFontFamily(f) => {
                simple!("UpdateLyricsFontFamily", "{:?}", f)
            }
            Self::UpdateMaxCacheMb(m) => simple!("UpdateMaxCacheMb", "{}", m),
            Self::UpdateDownloadQuality(q) => simple!("UpdateDownloadQuality", "{:?}", q),
            Self::UpdateDownloadDir(d) => simple!("UpdateDownloadDir", "{:?}", d),
            Self::UpdateDownloadDirDialog => simple!("UpdateDownloadDirDialog"),
            Self::ClearCache => simple!("ClearCache"),
            Self::CacheCleared(n, b) => simple!("CacheCleared", "{} files, {} bytes", n, b),
            Self::RefreshCacheStats => simple!("RefreshCacheStats"),
            Self::EnforceCacheLimit => simple!("EnforceCacheLimit"),
            Self::UpdateAudioOutputDevice(_) => simple!("UpdateAudioOutputDevice"),
            Self::UpdateDiscordEnabled(b) => simple!("UpdateDiscordEnabled", "{}", b),
            Self::UpdateProxyType(t) => simple!("UpdateProxyType", "{:?}", t),
            Self::UpdateProxyHost(_) => simple!("UpdateProxyHost"),
            Self::UpdateProxyPort(_) => simple!("UpdateProxyPort"),
            Self::UpdateProxyUsername(_) => simple!("UpdateProxyUsername"),
            Self::UpdateProxyPassword(_) => simple!("UpdateProxyPassword"),
            Self::ApplyProxySettings => simple!("ApplyProxySettings"),
            Self::ScrollToSection(s) => simple!("ScrollToSection", "{:?}", s),
            Self::SettingsScrolled(y) => simple!("SettingsScrolled", "{:.0}", y),
            Self::MeasureSectionPositions => simple!("MeasureSectionPositions"),
            Self::SectionPositionsMeasured(p) => {
                simple!("SectionPositionsMeasured", "{} positions", p.len())
            }
            Self::StartEditingKeybinding(a, scope) => {
                simple!("StartEditingKeybinding", "{:?} ({:?})", a, scope)
            }
            Self::CancelEditingKeybinding => simple!("CancelEditingKeybinding"),
            Self::KeybindingKeyPressed(_, _) => simple!("KeybindingKeyPressed"),

            // Database
            Self::DatabaseError(e) => simple!("DatabaseError", "{}", e),
            Self::SongsValidated(n) => simple!("SongsValidated", "{}", n),

            // Import
            Self::StartScan(_) => simple!("StartScan"),
            Self::CancelScan => simple!("CancelScan"),
            Self::ImportedPlaylistCreated(result) => {
                simple!("ImportedPlaylistCreated", "success={}", result.is_ok())
            }
            Self::WatcherEvent(_) => simple!("WatcherEvent"),
            Self::WatchedFolderSyncCompleted(id) => {
                simple!("WatchedFolderSyncCompleted", "{:?}", id)
            }
            Self::ShowInfoToast(_) => simple!("ShowInfoToast"),
            Self::ShowSuccessToast(_) => simple!("ShowSuccessToast"),
            Self::ShowWarningToast(_) => simple!("ShowWarningToast"),
            Self::ShowErrorToast(_) => simple!("ShowErrorToast"),
            Self::HideToast => simple!("HideToast"),
            Self::ClearImportingPlaylist => simple!("ClearImportingPlaylist"),

            // Playlist page
            Self::OpenPlaylist(id) => simple!("OpenPlaylist", "{}", id),
            Self::RequestDeletePlaylist(id) => simple!("RequestDeletePlaylist", "{}", id),
            Self::ConfirmDeletePlaylist => simple!("ConfirmDeletePlaylist"),
            Self::PlaylistDeleted(id) => simple!("PlaylistDeleted", "{}", id),
            Self::PlaySong(id) => simple!("PlaySong", "{}", id),
            Self::HoverSong(id) => simple!("HoverSong", "{:?}", id),
            Self::HoverIcon(id) => simple!("HoverIcon", "{:?}", id),
            Self::HoverSidebar(id) => simple!("HoverSidebar", "{:?}", id),
            Self::ToggleMyPlaylistsSection => simple!("ToggleMyPlaylistsSection"),
            Self::ToggleCollectedPlaylistsSection => {
                simple!("ToggleCollectedPlaylistsSection")
            }
            Self::TogglePlaylistSearch => simple!("TogglePlaylistSearch"),
            Self::PlaylistSearchChanged(_) => simple!("PlaylistSearchChanged"),
            Self::PlaylistSearchSubmit => simple!("PlaylistSearchSubmit"),
            Self::PlaylistSearchBlur => simple!("PlaylistSearchBlur"),
            Self::ToggleDescriptionExpand => simple!("ToggleDescriptionExpand"),

            // Edit dialog
            Self::EditPlaylist(id) => simple!("EditPlaylist", "{}", id),
            Self::EditPlaylistNameChanged(_) => simple!("EditPlaylistNameChanged"),
            Self::EditPlaylistDescriptionChanged(_) => simple!("EditPlaylistDescriptionChanged"),
            Self::EditPlaylistWatchEnabledChanged(enabled) => {
                simple!("EditPlaylistWatchEnabledChanged", "{}", enabled)
            }
            Self::PickCoverImage => simple!("PickCoverImage"),
            Self::CoverImagePicked(_) => simple!("CoverImagePicked"),
            Self::SavePlaylistEdits => simple!("SavePlaylistEdits"),
            Self::PlaylistUpdated(id) => simple!("PlaylistUpdated", "{}", id),

            // Lyrics
            Self::OpenLyricsPage => simple!("OpenLyricsPage"),
            Self::ShowLyricsContent => simple!("ShowLyricsContent"),
            Self::ShowLyricsArtwork => simple!("ShowLyricsArtwork"),
            Self::CloseLyricsPage => simple!("CloseLyricsPage"),
            Self::LyricsScroll(d) => simple!("LyricsScroll", "{:.1}", d),
            Self::LyricsViewportResized(size) => {
                simple!("LyricsViewportResized", "{}x{}", size.width, size.height)
            }
            Self::WindowResized(size) => simple!("WindowResized", "{}x{}", size.width, size.height),
            Self::LyricsFontSystemReady(_) => simple!("LyricsFontSystemReady"),
            Self::LyricsLoaded(id, lines) => {
                simple!("LyricsLoaded", "id={}, {} lines", id, lines.len())
            }
            Self::LyricsLoadFailed(id, _) => simple!("LyricsLoadFailed", "id={}", id),
            Self::FetchLyricsOnline(id, _) => simple!("FetchLyricsOnline", "id={}", id),
            Self::WarmLyricsCache(id, _) => simple!("WarmLyricsCache", "id={}", id),
            Self::LyricsWarmupFinished(id, result) => simple!(
                "LyricsWarmupFinished",
                "id={}, status={}",
                id,
                if result.is_ok() { "ok" } else { "err" }
            ),
            Self::LocalLyricsReady(id, lines) => {
                simple!("LocalLyricsReady", "id={}, {} lines", id, lines.len())
            }
            Self::LyricsEngineLinesReady(id, lines) => {
                simple!("LyricsEngineLinesReady", "id={}, {} lines", id, lines.len())
            }
            Self::LyricsShapedLinesReady(id, generation, lines, bitmaps, _, _, _) => simple!(
                "LyricsShapedLinesReady",
                "id={}, gen={}, {} lines, {} bitmaps",
                id,
                generation,
                lines.len(),
                bitmaps.len()
            ),
            Self::LyricsBackgroundReady(id, _, _, _, _) => {
                simple!("LyricsBackgroundReady", "id={}", id)
            }
            Self::LyricsCoverImageReady(id, _, _, w, h) => {
                simple!("LyricsCoverImageReady", "id={}, {}x{}", id, w, h)
            }

            // Playback controls
            Self::TogglePlayback => simple!("TogglePlayback"),
            Self::NextSong => simple!("NextSong"),
            Self::PrevSong => simple!("PrevSong"),
            Self::SeekPreview(p) => simple!("SeekPreview", "{:.2}", p),
            Self::SeekRelease => simple!("SeekRelease"),
            Self::SetVolume(v) => simple!("SetVolume", "{:.2}", v),
            Self::ToggleQueue => simple!("ToggleQueue"),
            Self::CyclePlayMode => simple!("CyclePlayMode"),
            Self::PreloadReady(idx, _, direction, _, _) => {
                simple!("PreloadReady", "idx={}, direction={}", idx, direction)
            }
            Self::PreloadBufferReady(idx, _, direction, buffer, duration, _, _) => {
                simple!(
                    "PreloadBufferReady",
                    "idx={}, direction={}, downloaded={}, duration={}s",
                    idx,
                    direction,
                    buffer.downloaded(),
                    duration
                )
            }
            Self::PreloadAudioFailed(idx, direction, _) => {
                simple!("PreloadAudioFailed", "idx={}, direction={}", idx, direction)
            }

            // Queue management
            Self::PlayPlaylist(id) => simple!("PlayPlaylist", "{}", id),
            Self::PlayQueueIndex(i) => simple!("PlayQueueIndex", "{}", i),
            Self::SongResolvedStreaming(i, _, _, buffer, _, _, _) => {
                simple!(
                    "SongResolvedStreaming",
                    "idx={}, buffer={}",
                    i,
                    buffer.is_some()
                )
            }
            Self::SongResolveFailed(_, reason) => simple!("SongResolveFailed", "{}", reason),
            Self::RemoveFromQueue(i) => simple!("RemoveFromQueue", "{}", i),
            Self::ClearQueue => simple!("ClearQueue"),

            // Keyboard
            Self::KeyPressed(_, _) => simple!("KeyPressed"),
            Self::ExecuteAction(a) => simple!("ExecuteAction", "{:?}", a),
            Self::GlobalHotkeyPressed(id) => simple!("GlobalHotkeyPressed", "{}", id),

            // Exit dialog
            Self::RequestClose => simple!("RequestClose"),
            Self::ConfirmExit => simple!("ConfirmExit"),
            Self::MinimizeToTray => simple!("MinimizeToTray"),
            Self::ExitDialogRememberChanged(b) => simple!("ExitDialogRememberChanged", "{}", b),

            // Tray
            Self::InitializeTray => simple!("InitializeTray"),
            Self::TrayUnavailable(error) => simple!("TrayUnavailable", "{}", error),
            Self::TrayCommand(c) => simple!("TrayCommand", "{:?}", c),

            // Media Controls
            Self::MprisCommand(c) => simple!("MprisCommand", "{:?}", c),
            Self::MprisStartedWithHandle(_, _) => simple!("MprisStartedWithHandle"),
            Self::ShowWindow => simple!("ShowWindow"),
            Self::ShowOrFocusWindow => simple!("ShowOrFocusWindow"),
            Self::ToggleWindow => simple!("ToggleWindow"),
            Self::InitializeGlobalHotkeys => simple!("InitializeGlobalHotkeys"),
            Self::WindowShown => simple!("WindowShown"),
            Self::WindowFocused => simple!("WindowFocused"),
            Self::WindowUnfocused => simple!("WindowUnfocused"),
            Self::WindowOperationComplete => simple!("WindowOperationComplete"),

            // NCM Login
            Self::TryAutoLogin(retry) => simple!("TryAutoLogin", "retry={}", retry),
            Self::RequestQrCode => simple!("RequestQrCode"),
            Self::QrCodeReady(_, _) => simple!("QrCodeReady"),
            Self::CheckQrStatus(_) => simple!("CheckQrStatus"),
            Self::QrLoginResult(s) => simple!("QrLoginResult", "{:?}", s),
            Self::Logout => simple!("Logout"),
            Self::ToggleLoginPopup => simple!("ToggleLoginPopup"),

            // NCM playback and navigation
            Self::ToggleFavorite(id) => simple!("ToggleFavorite", "{}", id),
            Self::FavoriteStatusChanged(id, s) => simple!("FavoriteStatusChanged", "{}, {}", id, s),
            Self::OpenNcmPlaylist(id) => simple!("OpenNcmPlaylist", "{}", id),
            Self::OpenUser(id) => simple!("OpenUser", "{}", id),
            Self::OpenArtist(id) => simple!("OpenArtist", "{}", id),
            Self::OpenAlbum(id) => simple!("OpenAlbum", "{}", id),
            Self::OpenArtistByName(name) => simple!("OpenArtistByName", "{}", name),

            // Cloud Playlist
            Self::ImageDownloadReady(generation, scope, kind, id, _path) => {
                simple!(
                    "ImageDownloadReady",
                    "generation={}, {:?}, {:?}, {}",
                    generation,
                    scope,
                    kind,
                    id
                )
            }
            Self::ImageDownloadFailed(generation, scope, kind, id) => {
                simple!(
                    "ImageDownloadFailed",
                    "generation={}, {:?}, {:?}, {}",
                    generation,
                    scope,
                    kind,
                    id
                )
            }
            Self::ImageViewportChanged(generation, images) => {
                simple!(
                    "ImageViewportChanged",
                    "generation={}, {} images",
                    generation,
                    images.len()
                )
            }
            Self::TogglePlaylistSubscribe(id) => simple!("TogglePlaylistSubscribe", "{}", id),
            Self::PlaylistSubscribeChanged(id, s) => {
                simple!("PlaylistSubscribeChanged", "{}, {}", id, s)
            }
            // Discover Page
            Self::RecommendedPlaylistsLoaded(generation, v) => {
                simple!(
                    "RecommendedPlaylistsLoaded",
                    "generation={}, {} playlists",
                    generation,
                    v.len()
                )
            }
            Self::DailyRecommendPreviewLoaded(generation, track) => {
                simple!(
                    "DailyRecommendPreviewLoaded",
                    "generation={}, loaded={}",
                    generation,
                    track.is_some()
                )
            }
            Self::PersonalFmPreviewLoaded(generation, tracks) => {
                simple!(
                    "PersonalFmPreviewLoaded",
                    "generation={}, {} tracks",
                    generation,
                    tracks.len()
                )
            }
            Self::PrivateRadarLoaded(generation, playlist) => {
                simple!(
                    "PrivateRadarLoaded",
                    "generation={}, loaded={}",
                    generation,
                    playlist.is_some()
                )
            }
            Self::HotPlaylistsLoaded(generation, v) => {
                simple!(
                    "HotPlaylistsLoaded",
                    "generation={}, {} playlists",
                    generation,
                    v.len()
                )
            }
            Self::OfficialPlaylistsLoaded(generation, v) => {
                simple!(
                    "OfficialPlaylistsLoaded",
                    "generation={}, {} playlists",
                    generation,
                    v.len()
                )
            }
            Self::HoverDiscoverPlaylist(id) => simple!("HoverDiscoverPlaylist", "{:?}", id),
            Self::PlayDiscoverPlaylist(id) => simple!("PlayDiscoverPlaylist", "{}", id),
            Self::SeeAllRecommended => simple!("SeeAllRecommended"),
            Self::SeeAllHot => simple!("SeeAllHot"),
            Self::SeeAllOfficial => simple!("SeeAllOfficial"),

            // Search Page
            Self::SearchSubmit => simple!("SearchSubmit"),
            Self::SearchTabChanged(tab) => simple!("SearchTabChanged", "{:?}", tab),
            Self::SearchResultsLoaded(payload) => {
                simple!(
                    "SearchResultsLoaded",
                    "tab={:?}, tracks={}, artists={}, albums={}, playlists={}",
                    payload.context.tab,
                    payload.tracks.len(),
                    payload.artists.len(),
                    payload.albums.len(),
                    payload.playlists.len()
                )
            }
            Self::SearchFailed(e) => simple!("SearchFailed", "{}", e.error),
            Self::SearchPageChanged(page) => simple!("SearchPageChanged", "{}", page),
            Self::HoverSearchSong(id) => simple!("HoverSearchSong", "{:?}", id),
            Self::HoverSearchCard(id) => simple!("HoverSearchCard", "{:?}", id),
            Self::PlaySearchSong(id) => simple!("PlaySearchSong", "id={}", id),
            Self::OpenSearchResult(id, tab) => {
                simple!("OpenSearchResult", "id={}, tab={:?}", id, tab)
            }
            Self::SwitchArtistTab(tab) => simple!("SwitchArtistTab", "{:?}", tab),

            // Sidebar resize
            Self::SidebarResizeStart => simple!("SidebarResizeStart"),
            Self::SidebarResizeEnd => simple!("SidebarResizeEnd"),

            // Streaming
            Self::StreamingEvent(_) => simple!("StreamingEvent"),

            // Audio events
            Self::AudioEvent(event) => simple!("AudioEvent", "{:?}", event),

            // Discord
            Self::DiscordUpdatePresence => simple!("DiscordUpdatePresence"),
            Self::DiscordClearPresence => simple!("DiscordClearPresence"),

            // Protocol
            Self::UriReceived(uri) => simple!("UriReceived", "{}", uri),

            // Context Menu
            Self::RightClickSong(id) => simple!("RightClickSong", "{}", id),
            Self::CloseContextMenu => simple!("CloseContextMenu"),
            Self::ContextMenuAction(action, id) => {
                simple!("ContextMenuAction", "{:?}, {}", action, id)
            }
            Self::PlaylistPickerConfirm(song_id, playlist_id) => {
                simple!(
                    "PlaylistPickerConfirm",
                    "song={}, playlist={}",
                    song_id,
                    playlist_id
                )
            }
            Self::AddToNcmPlaylist(song_id, playlist_id) => {
                simple!(
                    "AddToNcmPlaylist",
                    "song={}, playlist={}",
                    song_id,
                    playlist_id
                )
            }
            Self::NcmPlaylistAddResult(song_id, playlist_id, result) => {
                simple!(
                    "NcmPlaylistAddResult",
                    "song={}, playlist={}, ok={}",
                    song_id,
                    playlist_id,
                    result.is_ok()
                )
            }

            // Overlay System
            Self::DismissTopModal => simple!("DismissTopModal"),

            Self::EditSongTags(id) => simple!("EditSongTags", "{}", id),
            Self::OpenSongEditDialog(_) => simple!("OpenSongEditDialog"),
            Self::SongEditFieldChanged {
                song_id,
                field,
                value: _,
            } => simple!("SongEditFieldChanged", "{}, {}", song_id, field),
            Self::SongEditCoverReplaced(id, _) => simple!("SongEditCoverReplaced", "{}", id),
            Self::PickSongEditCover(id) => simple!("PickSongEditCover", "{}", id),
            Self::SaveSongEdits(id) => simple!("SaveSongEdits", "{}", id),
            Self::SongEditsSaved(id) => simple!("SongEditsSaved", "{}", id),
            Self::SongEditsFailed { song_id, error: _ } => {
                simple!("SongEditsFailed", "{}", song_id)
            }

            // Download
            Self::DownloadSong(id) => simple!("DownloadSong", "{}", id),
            Self::DownloadUrlResolved(sid, nid, ..) => {
                simple!("DownloadUrlResolved", "{}, ncm={}", sid, nid)
            }
            Self::DownloadPlaylist(id) => simple!("DownloadPlaylist", "{}", id),
            Self::DownloadCancel(id) => simple!("DownloadCancel", "{}", id),
            Self::DownloadProgress(sid, dl, total) => {
                simple!("DownloadProgress", "{}, {}/{}", sid, dl, total)
            }
            Self::DownloadCompleted(sid, path) => {
                simple!("DownloadCompleted", "{}, {}", sid, path)
            }
            Self::DownloadError(sid, e) => simple!("DownloadError", "{}, {}", sid, e),
            Self::DeleteDownloadHistory(sid) => simple!("DeleteDownloadHistory", "{}", sid),
            Self::SwitchDownloadTab(_) => simple!("SwitchDownloadTab"),
            Self::DownloadBatchEnqueue(items) => {
                simple!("DownloadBatchEnqueue", "{} items", items.len())
            }
            Self::RequestDownloadPlaylist(id, name, count) => {
                simple!("RequestDownloadPlaylist", "{}, {}, {}", id, name, count)
            }
            Self::ConfirmDownloadPlaylist => simple!("ConfirmDownloadPlaylist"),
            Self::DownloadsLoaded(v) => simple!("DownloadsLoaded", "{} downloads", v.len()),
        }
    }
}
