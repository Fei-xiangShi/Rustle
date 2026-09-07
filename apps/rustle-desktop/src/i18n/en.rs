//! English translations

use super::Key;
use std::collections::HashMap;
use std::sync::LazyLock;

static TRANSLATIONS: LazyLock<HashMap<Key, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // App
    m.insert(Key::AppName, "Rustle");

    // Navigation
    m.insert(Key::NavHome, "Home");
    m.insert(Key::NavDiscover, "Discover");
    m.insert(Key::NavRadio, "Radio");
    m.insert(Key::NavSettings, "Settings");
    m.insert(Key::NavAudioEngine, "Audio Engine");
    m.insert(Key::NavigationMenu, "Navigation menu");

    // Library - Local
    m.insert(Key::LibraryTitle, "Library");
    m.insert(Key::LibraryRecentlyPlayed, "Recently Played");
    m.insert(Key::ImportLocalPlaylist, "Import Local Playlist");
    // Library - Cloud
    m.insert(Key::CloudPlaylistsTitle, "My Playlists");
    m.insert(Key::CollectedPlaylistsTitle, "Collected Playlists");
    m.insert(
        Key::CloudPlaylistsNotLoggedIn,
        "Login to view cloud playlists",
    );

    // User
    m.insert(Key::GuestUser, "Guest");
    m.insert(Key::NotLoggedIn, "Not logged in");
    m.insert(Key::ClickToLogin, "Click to login");
    m.insert(Key::FreeAccount, "Free Account");
    m.insert(Key::ProfileStatsFallback, "Following 0 · Followers 0");
    m.insert(Key::ProfileNoBio, "No biography available");
    m.insert(Key::ProfileNoPlaylists, "No playlists available");

    // Search
    m.insert(Key::SearchPlaceholder, "Search songs, artists, albums...");
    m.insert(Key::SearchRelated, "related searches");
    m.insert(Key::SearchTabSongs, "Songs");
    m.insert(Key::SearchTabArtists, "Artists");
    m.insert(Key::SearchTabAlbums, "Albums");
    m.insert(Key::SearchTabPlaylists, "Playlists");
    m.insert(Key::SearchTabVideos, "Videos");
    m.insert(Key::SearchTabRadios, "Podcasts");

    // Hero Banner
    m.insert(Key::HeroTitle, "Global Hits 2024");
    m.insert(Key::HeroSubtitle, "The biggest songs from around the world");
    m.insert(Key::PlayButton, "Play");

    // Trending
    m.insert(Key::TrendingSongs, "Trending Songs");
    m.insert(Key::SeeAll, "See All");

    // Recently Played
    m.insert(Key::RecentlyPlayed, "Recently Played");
    m.insert(Key::RecentlyPlayedDescription, "Last 200 songs played");
    m.insert(Key::RecentlyPlayedList, "Recently Played");

    // Window Controls
    m.insert(Key::Minimize, "Minimize");
    m.insert(Key::Maximize, "Maximize");
    m.insert(Key::Restore, "Restore Down");
    m.insert(Key::Close, "Close");
    m.insert(Key::Settings, "Settings");

    // Navigation Controls
    m.insert(Key::Back, "Back");
    m.insert(Key::Forward, "Forward");

    // Settings Page - Tabs
    m.insert(Key::SettingsTitle, "Settings");
    m.insert(Key::SettingsTabAccount, "Account");
    m.insert(Key::SettingsTabPlayback, "Playback");
    m.insert(Key::SettingsTabDisplay, "Display");
    m.insert(Key::SettingsTabSystem, "System");
    m.insert(Key::SettingsTabStorage, "Storage");
    m.insert(Key::SettingsTabShortcuts, "Shortcuts");
    m.insert(Key::SettingsTabAbout, "About");

    // Settings - Playback Section
    m.insert(Key::SettingsPlaybackTitle, "Playback Settings");
    m.insert(Key::SettingsMusicQuality, "Music Quality");
    m.insert(
        Key::SettingsMusicQualityDesc,
        "Select audio quality for online streaming",
    );
    m.insert(Key::SettingsFadeInOut, "Fade In/Out");
    m.insert(
        Key::SettingsFadeInOutDesc,
        "Smooth volume transition when playing/pausing",
    );
    m.insert(Key::SettingsAutomix, "Automix");
    m.insert(
        Key::SettingsAutomixDesc,
        "Analyze adjacent tracks for beat, loudness, and bass-aware transitions",
    );
    m.insert(Key::SettingsVolumeNormalization, "Volume Normalization");
    m.insert(
        Key::SettingsVolumeNormalizationDesc,
        "Auto-adjust volume for consistent playback",
    );
    m.insert(Key::SettingsEqualizer, "Equalizer");
    m.insert(Key::SettingsEqualizerDesc, "Enable audio equalizer");

    // Audio Engine Page
    m.insert(Key::AudioEngineTitle, "Rustle Audio Engine");
    m.insert(
        Key::AudioEngineDesc,
        "Advanced audio processing and visualization",
    );
    m.insert(Key::AudioEngineEqualizer, "Equalizer");
    m.insert(
        Key::AudioEngineEqualizerDesc,
        "10-band parametric equalizer",
    );
    m.insert(Key::AudioEngineVolumeVisualization, "Volume Visualization");
    m.insert(
        Key::AudioEngineVolumeVisualizationDesc,
        "Real-time volume level display",
    );
    m.insert(Key::AudioEngineWaveform, "Waveform Display");
    m.insert(
        Key::AudioEngineWaveformDesc,
        "Real-time audio waveform visualization",
    );

    // Settings - Account Section
    m.insert(Key::SettingsAccountTitle, "Account Settings");
    m.insert(Key::SettingsAccountNotLoggedIn, "Not logged in");
    m.insert(Key::SettingsAccountLoggedInAs, "Logged in as");
    m.insert(Key::SettingsAccountVipStatus, "VIP Status");
    m.insert(Key::SettingsAccountLogout, "Log Out");

    // Settings - Display Section
    m.insert(Key::SettingsDisplayTitle, "Display & Interface");
    m.insert(Key::SettingsDarkMode, "Dark Mode");
    m.insert(Key::SettingsLanguage, "Language");
    m.insert(Key::SettingsPowerSavingMode, "Power Saving Mode");
    m.insert(
        Key::SettingsPowerSavingModeDesc,
        "Disable animations and effects to reduce CPU usage",
    );
    m.insert(Key::SettingsCloseBehavior, "Close Button Behavior");
    m.insert(Key::SettingsCloseBehaviorAsk, "Ask");
    m.insert(Key::SettingsCloseBehaviorExit, "Exit");
    m.insert(Key::SettingsCloseBehaviorMinimize, "Minimize to Tray");
    m.insert(Key::SettingsLyricsFontFamily, "Lyrics Font");
    m.insert(
        Key::SettingsLyricsFontFamilyAuto,
        "System Default (Auto Detect)",
    );

    // Settings - System Section
    m.insert(Key::SettingsSystemTitle, "System Settings");
    m.insert(Key::SettingsAudioDevice, "Audio Output Device");
    m.insert(Key::SettingsAudioBuffer, "Audio Buffer");
    m.insert(
        Key::SettingsAudioBufferDesc,
        "Larger buffer reduces audio stuttering",
    );
    m.insert(Key::SettingsDefaultDevice, "Default Device");

    // Settings - Network Section
    m.insert(Key::SettingsNetworkTitle, "Network Settings");
    m.insert(Key::SettingsTabNetwork, "Network");
    m.insert(Key::SettingsProxyType, "Proxy Type");
    m.insert(Key::SettingsProxyHost, "Proxy Host");
    m.insert(Key::SettingsProxyPort, "Proxy Port");
    m.insert(Key::SettingsProxyUsername, "Username");
    m.insert(Key::SettingsProxyPassword, "Password");
    m.insert(Key::SettingsProxyNone, "No Proxy");
    m.insert(Key::SettingsProxySystem, "System Proxy");

    // Settings - Storage Section
    m.insert(Key::SettingsStorageTitle, "Storage Settings");
    m.insert(Key::SettingsCacheLocation, "Cache Location");
    m.insert(Key::SettingsCacheSize, "Current Cache Size");
    m.insert(Key::SettingsMaxCache, "Max Cache Size");
    m.insert(Key::SettingsClearCache, "Clear Cache");
    m.insert(Key::SettingsClearCacheDesc, "Delete all cached audio files");
    m.insert(Key::SettingsClearButton, "Clear");
    m.insert(Key::SettingsDiscordRichPresence, "Discord Rich Presence");
    m.insert(
        Key::SettingsDiscordRichPresenceDesc,
        "Show playback status on Discord",
    );

    // Settings - Shortcuts Section
    m.insert(Key::SettingsShortcutsTitle, "Keyboard Shortcuts");
    m.insert(Key::SettingsShortcutsPlayback, "Playback Controls");
    m.insert(Key::SettingsShortcutsNavigation, "Navigation");
    m.insert(Key::SettingsShortcutsUI, "Interface");
    m.insert(Key::SettingsShortcutsGeneral, "General");
    m.insert(Key::SettingsShortcutFunction, "Function");
    m.insert(Key::SettingsShortcutLocal, "Shortcut");
    m.insert(Key::SettingsShortcutGlobal, "Global Shortcut");
    m.insert(Key::SettingsShortcutRecording, "Press key...");
    m.insert(
        Key::SettingsGlobalShortcutError,
        "Unable to update global shortcut",
    );

    // Settings - About Section
    m.insert(Key::SettingsAboutTitle, "About");
    m.insert(Key::SettingsAppName, "App Name");
    m.insert(Key::SettingsVersion, "Version");
    m.insert(Key::SettingsDeveloper, "Developer");
    m.insert(
        Key::SettingsDescription,
        "A modern local music player built with Rust",
    );

    // Shortcut Actions
    m.insert(Key::ActionPlayPause, "Play/Pause");
    m.insert(Key::ActionNextTrack, "Next Track");
    m.insert(Key::ActionPrevTrack, "Previous Track");
    m.insert(Key::ActionVolumeUp, "Volume Up");
    m.insert(Key::ActionVolumeDown, "Volume Down");
    m.insert(Key::ActionVolumeMute, "Mute");
    m.insert(Key::ActionSeekForward, "Seek Forward");
    m.insert(Key::ActionSeekBackward, "Seek Backward");
    m.insert(Key::ActionGoHome, "Go Home");
    m.insert(Key::ActionGoSearch, "Search");
    m.insert(Key::ActionGoQueue, "Playlist");
    m.insert(Key::ActionGoSettings, "Settings");
    m.insert(Key::ActionToggleQueue, "Toggle Playlist");
    m.insert(Key::ActionToggleSidebar, "Toggle Sidebar");
    m.insert(Key::ActionToggleFullscreen, "Fullscreen");
    m.insert(Key::ActionEscape, "Cancel/Close");
    m.insert(Key::ActionDelete, "Delete");
    m.insert(Key::ActionSelectAll, "Select All");

    // Playlist Page
    m.insert(Key::PlaylistTypeLabel, "Playlist");
    m.insert(Key::AlbumTypeLabel, "Album");
    m.insert(Key::UserTypeLabel, "User");
    m.insert(Key::ArtistTypeLabel, "Artist");
    m.insert(Key::PlaylistLikes, "{} likes");
    m.insert(Key::PlaylistSongCount, "{} songs");
    m.insert(Key::PlaylistPlayAll, "Play All");
    m.insert(Key::PlaylistSubscribe, "Collect");
    m.insert(Key::PlaylistUnsubscribe, "Uncollect");
    m.insert(Key::PlaylistDownload, "Download");
    m.insert(Key::PlaylistHeaderNumber, "#");
    m.insert(Key::PlaylistHeaderTitle, "Title");
    m.insert(Key::PlaylistHeaderAlbum, "Album");
    m.insert(Key::PlaylistHeaderAddedDate, "Added Date");

    // Discover Page
    m.insert(Key::DiscoverRecommended, "Recommended Playlists");
    m.insert(Key::DiscoverHot, "Hot Playlists");
    m.insert(Key::DiscoverOfficialPicks, "Official Picks");
    m.insert(Key::DiscoverSeeAll, "See All");
    m.insert(Key::DiscoverDailyRecommend, "Daily Recommend");
    m.insert(
        Key::DiscoverDailyRecommendDesc,
        "Personalized for you, updated daily at 6:00",
    );
    m.insert(Key::DiscoverDailyRecommendCreator, "NetEase Music");
    m.insert(Key::DiscoverPrivateRadar, "Private Radar");
    m.insert(
        Key::DiscoverPrivateRadarDesc,
        "A playlist tuned to your recent taste",
    );
    m.insert(Key::DiscoverPersonalFm, "Personal FM");
    m.insert(
        Key::DiscoverPersonalFmDesc,
        "Start a stream selected just for you",
    );
    m.insert(Key::DiscoverLoadFailed, "Failed to load daily recommend");
    m.insert(Key::DiscoverPlaylistLoadFailed, "Failed to load playlist");

    // Common UI
    m.insert(Key::Loading, "Loading...");
    m.insert(Key::Cancel, "Cancel");
    m.insert(Key::Save, "Save");
    m.insert(Key::Delete, "Delete");
    m.insert(Key::Refresh, "Refresh");

    // Lyrics Page
    m.insert(Key::LyricsNoLyrics, "No lyrics available");
    m.insert(Key::LyricsPureMusic, "Instrumental");

    // Artist/User detail pages
    m.insert(Key::ArtistPopularWorks, "Popular works");
    m.insert(Key::ArtistTopSongs, "Top Songs");
    m.insert(Key::ArtistAlbums, "Albums");
    m.insert(Key::ArtistNoAlbums, "No albums available");

    // Audio Engine
    m.insert(Key::AudioEngineEqualizerDisabled, "Equalizer disabled");
    m.insert(Key::AudioEngineSpectrum, "Spectrum");

    // Queue Panel
    m.insert(Key::QueueTitle, "Playlist");
    m.insert(Key::QueueSongCount, "{} songs");
    m.insert(Key::QueueEmpty, "Playlist is empty");

    // Playlist View
    m.insert(Key::PlaylistNoSongs, "No songs");
    m.insert(Key::ExpandDescription, "More ▼");
    m.insert(Key::CollapseDescription, "Less ▲");

    // Login Popup
    m.insert(Key::LoginScanQr, "Scan QR to Login");
    m.insert(Key::LoginGeneratingQr, "Generating QR code...");
    m.insert(Key::LoginRefreshQr, "Refresh QR Code");
    m.insert(Key::LoginLoggedIn, "Logged In");
    m.insert(Key::LoginLogout, "Log Out");

    // Delete Playlist Dialog
    m.insert(Key::DeletePlaylistTitle, "Delete Playlist");
    m.insert(
        Key::DeletePlaylistConfirm,
        "Are you sure you want to delete this playlist?",
    );

    // Edit Playlist Dialog
    m.insert(Key::EditPlaylistTitle, "Edit Playlist");
    m.insert(Key::EditPlaylistChangeCover, "Change Cover");
    m.insert(Key::EditPlaylistName, "Playlist Name");
    m.insert(Key::EditPlaylistNamePlaceholder, "Enter playlist name...");
    m.insert(Key::EditPlaylistDesc, "Description");
    m.insert(
        Key::EditPlaylistDescPlaceholder,
        "Enter description (optional)...",
    );
    m.insert(Key::EditPlaylistWatchLibrary, "Auto-sync this library");
    m.insert(
        Key::EditPlaylistWatchLibraryDesc,
        "Keep this local library playlist updated when files change in the folder below.",
    );

    // Exit Dialog
    m.insert(Key::ExitDialogTitle, "Exit Application");
    m.insert(
        Key::ExitDialogMessage,
        "Do you want to exit or minimize to system tray?",
    );
    m.insert(Key::ExitDialogExit, "Exit");
    m.insert(Key::ExitDialogMinimize, "Minimize to Tray");

    // System tray
    m.insert(Key::TrayPlay, "Play");
    m.insert(Key::TrayPause, "Pause");
    m.insert(Key::TrayPrevious, "Previous Track");
    m.insert(Key::TrayNext, "Next Track");
    m.insert(Key::TrayFavorite, "Add to Favorites");
    m.insert(Key::TrayUnfavorite, "Remove from Favorites");
    m.insert(Key::TrayPlayMode, "Playback Mode");
    m.insert(Key::TraySequential, "Sequential");
    m.insert(Key::TrayLoopAll, "Repeat All");
    m.insert(Key::TrayLoopOne, "Repeat One");
    m.insert(Key::TrayShuffle, "Shuffle");
    m.insert(Key::TrayToggleWindow, "Show/Hide Rustle");
    m.insert(Key::TrayQuit, "Quit Rustle");
    m.insert(Key::TrayNotPlaying, "Not Playing");
    m.insert(
        Key::TrayUnavailable,
        "The system tray is unavailable. Rustle will keep this window visible.",
    );

    // Context Menu
    m.insert(Key::ContextMenuEditTags, "Edit Tags");

    // Song Info Dialog
    m.insert(Key::SongInfoTitle, "Song Info");
    m.insert(Key::SongInfoBasicSection, "Basic Info");
    m.insert(Key::SongInfoDetailSection, "Details");
    m.insert(Key::SongInfoFileSection, "File Info");
    m.insert(Key::SongInfoStatsSection, "Statistics");
    m.insert(Key::SongInfoTrackNumber, "Track No.");
    m.insert(Key::SongInfoYear, "Year");
    m.insert(Key::SongInfoGenre, "Genre");
    m.insert(Key::SongInfoDuration, "Duration");
    m.insert(Key::SongInfoFormat, "Format");
    m.insert(Key::SongInfoFilePath, "File Path");
    m.insert(Key::SongInfoFileSize, "File Size");
    m.insert(Key::SongInfoPlayCount, "Plays");
    m.insert(Key::SongInfoLastPlayed, "Last Played");
    m.insert(Key::SongInfoEditButton, "Edit Tags");
    m.insert(Key::SongInfoCloseButton, "Close");

    // Song Edit Dialog
    m.insert(Key::SongEditTitle, "Edit Song Tags");
    m.insert(Key::SongEditLabelTitle, "Title");
    m.insert(Key::SongEditLabelArtist, "Artist");
    m.insert(Key::SongEditLabelAlbum, "Album");
    m.insert(Key::SongEditCoverReplace, "Replace Cover");
    m.insert(Key::SongEditCancel, "Cancel");
    m.insert(Key::SongEditSave, "Save");
    m.insert(Key::SongEditSaved, "Tags saved");
    m.insert(Key::SongEditFailed, "Save failed");
    m.insert(Key::SongAddedToPlaylist, "Song added to playlist");

    // Context Menu Items
    m.insert(Key::ContextMenuPlayNow, "Play Now");
    m.insert(Key::ContextMenuPlayNext, "Play Next");
    m.insert(Key::ContextMenuAddFavorites, "Add to Favorites");
    m.insert(Key::ContextMenuRemoveFavorites, "Remove from Favorites");
    m.insert(Key::ContextMenuAddPlaylist, "Add to Playlist...");
    m.insert(Key::ContextMenuViewArtist, "View Artist");
    m.insert(Key::ContextMenuViewAlbum, "View Album");
    m.insert(Key::ContextMenuShowInFolder, "Show in Folder");
    m.insert(Key::ContextMenuRemoveFromList, "Remove from List");
    m.insert(Key::ContextMenuDownload, "Download");

    // Playlist Picker
    m.insert(Key::PlaylistPickerTitle, "Select Playlist");
    m.insert(Key::PlaylistPickerConfirm, "Confirm");
    m.insert(Key::PlaylistPickerCancel, "Cancel");

    // Navigation
    m.insert(Key::NavDownloads, "Downloads");

    // Download
    m.insert(Key::DownloadPanelTitle, "Download Manager");
    m.insert(Key::DownloadActive, "Active");
    m.insert(Key::DownloadCompleted, "Completed");
    m.insert(Key::DownloadPending, "Pending");
    m.insert(Key::DownloadNoActive, "No active downloads");
    m.insert(Key::DownloadNoCompleted, "No completed downloads");
    m.insert(Key::DownloadNoFailed, "No failed downloads");
    m.insert(Key::DownloadSpeed, "Speed");
    m.insert(Key::DownloadCancel, "Cancel");
    m.insert(Key::DownloadRetry, "Retry");
    m.insert(Key::DownloadQueued, "Added to download queue");
    m.insert(Key::DownloadFailed, "Download failed");

    // Download Playlist Dialog
    m.insert(Key::DownloadPlaylistTitle, "Download Playlist");
    m.insert(
        Key::DownloadPlaylistConfirm,
        "Download {count} songs from \"{name}\"?",
    );
    m.insert(Key::DownloadPlaylistConfirmBtn, "Download");

    // Settings - Download
    m.insert(Key::SettingsDownloadLocation, "Download Location");
    m.insert(Key::SettingsDownloadQuality, "Download Quality");
    m.insert(
        Key::SettingsDownloadLocationDesc,
        "Downloaded songs will be saved here",
    );
    m.insert(Key::SettingsDownloadChange, "Change");

    m
});

pub fn translations() -> &'static HashMap<Key, &'static str> {
    &TRANSLATIONS
}
