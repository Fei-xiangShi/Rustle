//! Internationalization (i18n) support for Rustle
//! Supports multiple languages with easy extensibility
//!
//! Structure:
//! - mod.rs: Core types (Language, Key, Locale) and translation lookup
//! - en.rs: English translations
//! - zh.rs: Chinese translations

mod en;
mod zh;

use std::collections::HashMap;

/// Supported languages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    #[default]
    English,
    Chinese,
}

impl Language {
    /// Get language display name
    pub fn display_name(&self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Chinese => "简体中文",
        }
    }

    /// Get language code
    pub fn code(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Chinese => "zh",
        }
    }

    /// Get a language by code
    pub fn from_code(code: &str) -> Option<Self> {
        Self::all()
            .iter()
            .copied()
            .find(|language| language.code() == code)
    }

    /// All available languages
    pub fn all() -> &'static [Language] {
        &[Language::English, Language::Chinese]
    }
}

/// Translation keys
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    // App
    AppName,

    // Navigation
    NavHome,
    NavDiscover,
    NavRadio,
    NavDownloads,
    NavSettings,
    NavAudioEngine,
    NavigationMenu,

    // Library - Local
    LibraryTitle,
    LibraryRecentlyPlayed,
    ImportLocalPlaylist,
    // Library - Cloud
    CloudPlaylistsTitle,
    CollectedPlaylistsTitle,
    CloudPlaylistsNotLoggedIn,

    // User
    GuestUser,
    NotLoggedIn,
    ClickToLogin,
    FreeAccount,
    ProfileStatsFallback,
    ProfileNoBio,
    ProfileNoPlaylists,

    // Search
    SearchPlaceholder,
    SearchRelated,
    SearchTabSongs,
    SearchTabArtists,
    SearchTabAlbums,
    SearchTabPlaylists,
    SearchTabVideos,
    SearchTabRadios,

    // Hero Banner
    HeroTitle,
    HeroSubtitle,
    PlayButton,

    // Trending
    TrendingSongs,
    SeeAll,

    // Recently Played
    RecentlyPlayed,
    RecentlyPlayedDescription,
    RecentlyPlayedList,

    // Window Controls
    Minimize,
    Maximize,
    Restore,
    Close,
    Settings,

    // Navigation Controls
    Back,
    Forward,

    // Settings Page - Tabs
    SettingsTitle,
    SettingsTabPlayback,
    SettingsTabDisplay,
    SettingsTabSystem,
    SettingsTabStorage,
    SettingsTabShortcuts,
    SettingsTabAbout,
    SettingsTabAccount,

    // Settings - Playback Section
    SettingsPlaybackTitle,
    SettingsMusicQuality,
    SettingsMusicQualityDesc,
    SettingsFadeInOut,
    SettingsFadeInOutDesc,
    SettingsAutomix,
    SettingsAutomixDesc,
    SettingsVolumeNormalization,
    SettingsVolumeNormalizationDesc,
    SettingsEqualizer,
    SettingsEqualizerDesc,

    // Audio Engine Page
    AudioEngineTitle,
    AudioEngineDesc,
    AudioEngineEqualizer,
    AudioEngineEqualizerDesc,
    AudioEngineVolumeVisualization,
    AudioEngineVolumeVisualizationDesc,
    AudioEngineWaveform,
    AudioEngineWaveformDesc,

    // Settings - Account Section
    SettingsAccountTitle,
    SettingsAccountNotLoggedIn,
    SettingsAccountLoggedInAs,
    SettingsAccountVipStatus,
    SettingsAccountLogout,

    // Settings - Display Section
    SettingsDisplayTitle,
    SettingsDarkMode,
    SettingsLanguage,
    SettingsPowerSavingMode,
    SettingsPowerSavingModeDesc,
    SettingsCloseBehavior,
    SettingsCloseBehaviorAsk,
    SettingsCloseBehaviorExit,
    SettingsCloseBehaviorMinimize,
    SettingsLyricsFontFamily,
    SettingsLyricsFontFamilyAuto,

    // Settings - System Section
    SettingsSystemTitle,
    SettingsAudioDevice,
    SettingsAudioBuffer,
    SettingsAudioBufferDesc,
    SettingsDefaultDevice,

    // Settings - Network Section
    SettingsNetworkTitle,
    SettingsTabNetwork,
    SettingsProxyType,
    SettingsProxyHost,
    SettingsProxyPort,
    SettingsProxyUsername,
    SettingsProxyPassword,
    SettingsProxyNone,
    SettingsProxySystem,

    // Settings - Storage Section
    SettingsStorageTitle,
    SettingsCacheLocation,
    SettingsCacheSize,
    SettingsMaxCache,
    SettingsClearCache,
    SettingsClearCacheDesc,
    SettingsClearButton,
    SettingsDiscordRichPresence,
    SettingsDiscordRichPresenceDesc,

    // Settings - Shortcuts Section
    SettingsShortcutsTitle,
    SettingsShortcutsPlayback,
    SettingsShortcutsNavigation,
    SettingsShortcutsUI,
    SettingsShortcutsGeneral,
    SettingsShortcutFunction,
    SettingsShortcutLocal,
    SettingsShortcutGlobal,
    SettingsShortcutRecording,
    SettingsGlobalShortcutError,

    // Settings - About Section
    SettingsAboutTitle,
    SettingsAppName,
    SettingsVersion,
    SettingsDeveloper,
    SettingsDescription,

    // Shortcut Actions
    ActionPlayPause,
    ActionNextTrack,
    ActionPrevTrack,
    ActionVolumeUp,
    ActionVolumeDown,
    ActionVolumeMute,
    ActionSeekForward,
    ActionSeekBackward,
    ActionGoHome,
    ActionGoSearch,
    ActionGoQueue,
    ActionGoSettings,
    ActionToggleQueue,
    ActionToggleSidebar,
    ActionToggleFullscreen,
    ActionEscape,
    ActionDelete,
    ActionSelectAll,

    // Playlist Page
    PlaylistTypeLabel,
    AlbumTypeLabel,
    UserTypeLabel,
    ArtistTypeLabel,
    PlaylistLikes,
    PlaylistSongCount,
    PlaylistPlayAll,
    PlaylistSubscribe,
    PlaylistUnsubscribe,
    PlaylistDownload,
    PlaylistHeaderNumber,
    PlaylistHeaderTitle,
    PlaylistHeaderAlbum,
    PlaylistHeaderAddedDate,

    // Discover Page
    DiscoverRecommended,
    DiscoverHot,
    DiscoverOfficialPicks,
    DiscoverSeeAll,
    DiscoverDailyRecommend,
    DiscoverDailyRecommendDesc,
    DiscoverDailyRecommendCreator,
    DiscoverPrivateRadar,
    DiscoverPrivateRadarDesc,
    DiscoverPersonalFm,
    DiscoverPersonalFmDesc,
    DiscoverLoadFailed,
    DiscoverPlaylistLoadFailed,

    // Common UI
    Loading,
    Cancel,
    Save,
    Delete,
    Refresh,

    // Lyrics Page
    LyricsNoLyrics,
    LyricsPureMusic,

    // Artist/User detail pages
    ArtistPopularWorks,
    ArtistTopSongs,
    ArtistAlbums,
    ArtistNoAlbums,

    // Audio Engine
    AudioEngineEqualizerDisabled,
    AudioEngineSpectrum,

    // Queue Panel
    QueueTitle,
    QueueSongCount,
    QueueEmpty,

    // Playlist View
    PlaylistNoSongs,
    ExpandDescription,
    CollapseDescription,

    // Login Popup
    LoginScanQr,
    LoginGeneratingQr,
    LoginRefreshQr,
    LoginLoggedIn,
    LoginLogout,

    // Delete Playlist Dialog
    DeletePlaylistTitle,
    DeletePlaylistConfirm,

    // Download Playlist Dialog
    DownloadPlaylistTitle,
    DownloadPlaylistConfirm,
    DownloadPlaylistConfirmBtn,

    // Edit Playlist Dialog
    EditPlaylistTitle,
    EditPlaylistChangeCover,
    EditPlaylistName,
    EditPlaylistNamePlaceholder,
    EditPlaylistDesc,
    EditPlaylistDescPlaceholder,
    EditPlaylistWatchLibrary,
    EditPlaylistWatchLibraryDesc,

    // Exit Dialog
    ExitDialogTitle,
    ExitDialogMessage,
    ExitDialogExit,
    ExitDialogMinimize,

    // System tray
    TrayPlay,
    TrayPause,
    TrayPrevious,
    TrayNext,
    TrayFavorite,
    TrayUnfavorite,
    TrayPlayMode,
    TraySequential,
    TrayLoopAll,
    TrayLoopOne,
    TrayShuffle,
    TrayToggleWindow,
    TrayQuit,
    TrayNotPlaying,
    TrayUnavailable,

    // Context Menu
    ContextMenuEditTags,

    // Song Info Dialog
    SongInfoTitle,
    SongInfoBasicSection,
    SongInfoDetailSection,
    SongInfoFileSection,
    SongInfoStatsSection,
    SongInfoTrackNumber,
    SongInfoYear,
    SongInfoGenre,
    SongInfoDuration,
    SongInfoFormat,
    SongInfoFilePath,
    SongInfoFileSize,
    SongInfoPlayCount,
    SongInfoLastPlayed,
    SongInfoEditButton,
    SongInfoCloseButton,

    // Song Edit Dialog
    SongEditTitle,
    SongEditLabelTitle,
    SongEditLabelArtist,
    SongEditLabelAlbum,
    SongEditCoverReplace,
    SongEditCancel,
    SongEditSave,
    SongEditSaved,
    SongEditFailed,
    SongAddedToPlaylist,

    // Context Menu Items
    ContextMenuPlayNow,
    ContextMenuPlayNext,
    ContextMenuAddFavorites,
    ContextMenuRemoveFavorites,
    ContextMenuAddPlaylist,
    ContextMenuViewArtist,
    ContextMenuViewAlbum,
    ContextMenuShowInFolder,
    ContextMenuRemoveFromList,
    ContextMenuDownload,

    // Playlist Picker
    PlaylistPickerTitle,
    PlaylistPickerConfirm,
    PlaylistPickerCancel,

    // Download
    DownloadPanelTitle,
    DownloadActive,
    DownloadCompleted,
    DownloadPending,
    DownloadNoActive,
    DownloadNoCompleted,
    DownloadNoFailed,
    DownloadSpeed,
    DownloadCancel,
    DownloadRetry,
    DownloadQueued,
    DownloadFailed,

    // Settings - Download
    SettingsDownloadLocation,
    SettingsDownloadQuality,
    SettingsDownloadLocationDesc,
    SettingsDownloadChange,
}

/// Get translation for a key in the specified language
pub fn t(lang: Language, key: Key) -> &'static str {
    let translations: &HashMap<Key, &'static str> = match lang {
        Language::English => en::translations(),
        Language::Chinese => zh::translations(),
    };

    translations.get(&key).copied().unwrap_or("???")
}

/// Localization context that can be passed around
#[derive(Debug, Clone, Copy, Default)]
pub struct Locale {
    pub language: Language,
}

impl Locale {
    pub fn new(language: Language) -> Self {
        Self { language }
    }

    /// Get translation for a key
    pub fn get(&self, key: Key) -> &'static str {
        t(self.language, key)
    }
}

#[cfg(test)]
mod tests {
    use super::{Key, Language, t};

    #[test]
    fn chinese_queue_song_count_separates_number_and_unit() {
        assert_eq!(t(Language::Chinese, Key::QueueSongCount), "{} 首");
    }
}
