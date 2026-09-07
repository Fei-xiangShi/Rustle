//! Chinese translations (简体中文)

use super::Key;
use std::collections::HashMap;
use std::sync::LazyLock;

static TRANSLATIONS: LazyLock<HashMap<Key, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // App
    m.insert(Key::AppName, "Rustle");

    // Navigation
    m.insert(Key::NavHome, "首页");
    m.insert(Key::NavDiscover, "发现");
    m.insert(Key::NavRadio, "电台");
    m.insert(Key::NavSettings, "设置");
    m.insert(Key::NavAudioEngine, "音频引擎");
    m.insert(Key::NavigationMenu, "导航菜单");

    // Library - Local
    m.insert(Key::LibraryTitle, "音乐库");
    m.insert(Key::LibraryRecentlyPlayed, "最近播放");
    m.insert(Key::ImportLocalPlaylist, "导入本地歌单");
    // Library - Cloud
    m.insert(Key::CloudPlaylistsTitle, "我的歌单");
    m.insert(Key::CollectedPlaylistsTitle, "收藏的歌单");
    m.insert(Key::CloudPlaylistsNotLoggedIn, "登录以查看云端歌单");

    // User
    m.insert(Key::GuestUser, "访客");
    m.insert(Key::NotLoggedIn, "未登录");
    m.insert(Key::ClickToLogin, "点击登录");
    m.insert(Key::FreeAccount, "免费账户");
    m.insert(Key::ProfileStatsFallback, "关注 0 · 粉丝 0");
    m.insert(Key::ProfileNoBio, "暂无简介");
    m.insert(Key::ProfileNoPlaylists, "暂无歌单");

    // Search
    m.insert(Key::SearchPlaceholder, "搜索歌曲、艺术家、专辑...");
    m.insert(Key::SearchRelated, "的相关搜索");
    m.insert(Key::SearchTabSongs, "单曲");
    m.insert(Key::SearchTabArtists, "歌手");
    m.insert(Key::SearchTabAlbums, "专辑");
    m.insert(Key::SearchTabPlaylists, "歌单");
    m.insert(Key::SearchTabVideos, "视频");
    m.insert(Key::SearchTabRadios, "播客");

    // Hero Banner
    m.insert(Key::HeroTitle, "2024 全球热门");
    m.insert(Key::HeroSubtitle, "来自世界各地的热门歌曲");
    m.insert(Key::PlayButton, "播放");

    // Trending
    m.insert(Key::TrendingSongs, "热门歌曲");
    m.insert(Key::SeeAll, "查看全部");

    // Recently Played
    m.insert(Key::RecentlyPlayed, "最近播放");
    m.insert(Key::RecentlyPlayedDescription, "最近播放的200首歌曲");
    m.insert(Key::RecentlyPlayedList, "最近播放");

    // Window Controls
    m.insert(Key::Minimize, "最小化");
    m.insert(Key::Maximize, "最大化");
    m.insert(Key::Restore, "向下还原");
    m.insert(Key::Close, "关闭");
    m.insert(Key::Settings, "设置");

    // Navigation Controls
    m.insert(Key::Back, "后退");
    m.insert(Key::Forward, "前进");

    // Settings Page - Tabs
    m.insert(Key::SettingsTitle, "设置");
    m.insert(Key::SettingsTabAccount, "账号");
    m.insert(Key::SettingsTabPlayback, "播放");
    m.insert(Key::SettingsTabDisplay, "界面");
    m.insert(Key::SettingsTabSystem, "系统");
    m.insert(Key::SettingsTabStorage, "存储");
    m.insert(Key::SettingsTabShortcuts, "快捷键");
    m.insert(Key::SettingsTabAbout, "关于");

    // Settings - Playback Section
    m.insert(Key::SettingsPlaybackTitle, "播放设置");
    m.insert(Key::SettingsMusicQuality, "音质选择");
    m.insert(Key::SettingsMusicQualityDesc, "选择在线播放的音频质量");
    m.insert(Key::SettingsFadeInOut, "淡入淡出");
    m.insert(Key::SettingsFadeInOutDesc, "播放和暂停时平滑过渡音量");
    m.insert(Key::SettingsAutomix, "Automix");
    m.insert(
        Key::SettingsAutomixDesc,
        "智能分析相邻歌曲并进行节拍、响度与低频衔接",
    );
    m.insert(Key::SettingsVolumeNormalization, "音量标准化");
    m.insert(
        Key::SettingsVolumeNormalizationDesc,
        "自动调整音量使所有歌曲音量一致",
    );
    m.insert(Key::SettingsEqualizer, "均衡器");
    m.insert(Key::SettingsEqualizerDesc, "启用音频均衡器调节");

    // Audio Engine Page
    m.insert(Key::AudioEngineTitle, "Rustle 音频引擎");
    m.insert(Key::AudioEngineDesc, "高级音频处理与可视化");
    m.insert(Key::AudioEngineEqualizer, "均衡器");
    m.insert(Key::AudioEngineEqualizerDesc, "10 频段参数均衡器");
    m.insert(Key::AudioEngineVolumeVisualization, "音量可视化");
    m.insert(Key::AudioEngineVolumeVisualizationDesc, "实时音量电平显示");
    m.insert(Key::AudioEngineWaveform, "波形显示");
    m.insert(Key::AudioEngineWaveformDesc, "音频波形实时可视化");

    // Settings - Account Section
    m.insert(Key::SettingsAccountTitle, "账号设置");
    m.insert(Key::SettingsAccountNotLoggedIn, "当前未登录");
    m.insert(Key::SettingsAccountLoggedInAs, "当前登录账号");
    m.insert(Key::SettingsAccountVipStatus, "VIP 状态");
    m.insert(Key::SettingsAccountLogout, "退出登录");

    // Settings - Display Section
    m.insert(Key::SettingsDisplayTitle, "界面与显示");
    m.insert(Key::SettingsDarkMode, "深色模式");
    m.insert(Key::SettingsLanguage, "应用语言");
    m.insert(Key::SettingsPowerSavingMode, "省电模式");
    m.insert(
        Key::SettingsPowerSavingModeDesc,
        "关闭动画和特效，降低 CPU 占用",
    );
    m.insert(Key::SettingsCloseBehavior, "关闭按钮行为");
    m.insert(Key::SettingsCloseBehaviorAsk, "询问");
    m.insert(Key::SettingsCloseBehaviorExit, "退出");
    m.insert(Key::SettingsCloseBehaviorMinimize, "最小化到托盘");
    m.insert(Key::SettingsLyricsFontFamily, "歌词字体");
    m.insert(Key::SettingsLyricsFontFamilyAuto, "系统默认 (自动检测)");

    // Settings - System Section
    m.insert(Key::SettingsSystemTitle, "系统设置");
    m.insert(Key::SettingsAudioDevice, "音频输出设备");
    m.insert(Key::SettingsAudioBuffer, "音频缓冲区");
    m.insert(Key::SettingsAudioBufferDesc, "较大的缓冲区可减少音频卡顿");
    m.insert(Key::SettingsDefaultDevice, "默认设备");

    // Settings - Network Section
    m.insert(Key::SettingsNetworkTitle, "网络设置");
    m.insert(Key::SettingsTabNetwork, "网络");
    m.insert(Key::SettingsProxyType, "代理类型");
    m.insert(Key::SettingsProxyHost, "代理地址");
    m.insert(Key::SettingsProxyPort, "代理端口");
    m.insert(Key::SettingsProxyUsername, "用户名");
    m.insert(Key::SettingsProxyPassword, "密码");
    m.insert(Key::SettingsProxyNone, "无代理");
    m.insert(Key::SettingsProxySystem, "系统代理");

    // Settings - Storage Section
    m.insert(Key::SettingsStorageTitle, "存储设置");
    m.insert(Key::SettingsCacheLocation, "缓存位置");
    m.insert(Key::SettingsCacheSize, "当前缓存大小");
    m.insert(Key::SettingsMaxCache, "最大缓存占用");
    m.insert(Key::SettingsClearCache, "清除缓存");
    m.insert(Key::SettingsClearCacheDesc, "删除所有缓存的音频文件");
    m.insert(Key::SettingsClearButton, "清除");
    m.insert(Key::SettingsDiscordRichPresence, "Discord Rich Presence");
    m.insert(
        Key::SettingsDiscordRichPresenceDesc,
        "在 Discord 上显示当前播放状态",
    );

    // Settings - Shortcuts Section
    m.insert(Key::SettingsShortcutsTitle, "快捷键设置");
    m.insert(Key::SettingsShortcutsPlayback, "播放控制");
    m.insert(Key::SettingsShortcutsNavigation, "导航");
    m.insert(Key::SettingsShortcutsUI, "界面");
    m.insert(Key::SettingsShortcutsGeneral, "通用");
    m.insert(Key::SettingsShortcutFunction, "功能");
    m.insert(Key::SettingsShortcutLocal, "快捷键");
    m.insert(Key::SettingsShortcutGlobal, "全局快捷键");
    m.insert(Key::SettingsShortcutRecording, "请按快捷键...");
    m.insert(Key::SettingsGlobalShortcutError, "无法更新全局快捷键");

    // Settings - About Section
    m.insert(Key::SettingsAboutTitle, "关于");
    m.insert(Key::SettingsAppName, "应用名称");
    m.insert(Key::SettingsVersion, "版本");
    m.insert(Key::SettingsDeveloper, "开发者");
    m.insert(
        Key::SettingsDescription,
        "一个基于 Rust 的现代化本地音乐播放器",
    );

    // Shortcut Actions
    m.insert(Key::ActionPlayPause, "播放/暂停");
    m.insert(Key::ActionNextTrack, "下一首");
    m.insert(Key::ActionPrevTrack, "上一首");
    m.insert(Key::ActionVolumeUp, "增加音量");
    m.insert(Key::ActionVolumeDown, "减少音量");
    m.insert(Key::ActionVolumeMute, "静音");
    m.insert(Key::ActionSeekForward, "快进");
    m.insert(Key::ActionSeekBackward, "快退");
    m.insert(Key::ActionGoHome, "返回首页");
    m.insert(Key::ActionGoSearch, "搜索");
    m.insert(Key::ActionGoQueue, "播放列表");
    m.insert(Key::ActionGoSettings, "设置");
    m.insert(Key::ActionToggleQueue, "显示/隐藏播放列表");
    m.insert(Key::ActionToggleSidebar, "显示/隐藏侧边栏");
    m.insert(Key::ActionToggleFullscreen, "全屏");
    m.insert(Key::ActionEscape, "取消/关闭");
    m.insert(Key::ActionDelete, "删除");
    m.insert(Key::ActionSelectAll, "全选");

    // Playlist Page
    m.insert(Key::PlaylistTypeLabel, "歌单");
    m.insert(Key::AlbumTypeLabel, "专辑");
    m.insert(Key::UserTypeLabel, "用户");
    m.insert(Key::ArtistTypeLabel, "歌手");
    m.insert(Key::PlaylistLikes, "{} 次点赞");
    m.insert(Key::PlaylistSongCount, "{} 首歌曲");
    m.insert(Key::PlaylistPlayAll, "播放全部");
    m.insert(Key::PlaylistSubscribe, "收藏");
    m.insert(Key::PlaylistUnsubscribe, "取消收藏");
    m.insert(Key::PlaylistDownload, "下载");
    m.insert(Key::PlaylistHeaderNumber, "#");
    m.insert(Key::PlaylistHeaderTitle, "标题");
    m.insert(Key::PlaylistHeaderAlbum, "专辑");
    m.insert(Key::PlaylistHeaderAddedDate, "添加日期");

    // Discover Page
    m.insert(Key::DiscoverRecommended, "推荐歌单");
    m.insert(Key::DiscoverHot, "热门歌单");
    m.insert(Key::DiscoverOfficialPicks, "官方精选");
    m.insert(Key::DiscoverSeeAll, "查看全部");
    m.insert(Key::DiscoverDailyRecommend, "每日推荐");
    m.insert(
        Key::DiscoverDailyRecommendDesc,
        "根据你的口味生成，每天6:00更新",
    );
    m.insert(Key::DiscoverDailyRecommendCreator, "网易云音乐");
    m.insert(Key::DiscoverPrivateRadar, "私人雷达");
    m.insert(Key::DiscoverPrivateRadarDesc, "捕捉你的近期音乐偏好");
    m.insert(Key::DiscoverPersonalFm, "私人 FM");
    m.insert(Key::DiscoverPersonalFmDesc, "即刻播放为你挑选的音乐");
    m.insert(Key::DiscoverLoadFailed, "无法加载每日推荐");
    m.insert(Key::DiscoverPlaylistLoadFailed, "无法加载歌单");

    // Common UI
    m.insert(Key::Loading, "加载中...");
    m.insert(Key::Cancel, "取消");
    m.insert(Key::Save, "保存");
    m.insert(Key::Delete, "删除");
    m.insert(Key::Refresh, "刷新");

    // Lyrics Page
    m.insert(Key::LyricsNoLyrics, "暂无歌词");
    m.insert(Key::LyricsPureMusic, "纯音乐，请欣赏");

    // Artist/User detail pages
    m.insert(Key::ArtistPopularWorks, "热门作品");
    m.insert(Key::ArtistTopSongs, "热门单曲");
    m.insert(Key::ArtistAlbums, "专辑");
    m.insert(Key::ArtistNoAlbums, "暂无专辑数据");

    // Audio Engine
    m.insert(Key::AudioEngineEqualizerDisabled, "均衡器已关闭");
    m.insert(Key::AudioEngineSpectrum, "频谱");

    // Queue Panel
    m.insert(Key::QueueTitle, "播放列表");
    m.insert(Key::QueueSongCount, "{} 首");
    m.insert(Key::QueueEmpty, "播放列表为空");

    // Playlist View
    m.insert(Key::PlaylistNoSongs, "暂无歌曲");
    m.insert(Key::ExpandDescription, "展开 ▼");
    m.insert(Key::CollapseDescription, "收起 ▲");

    // Login Popup
    m.insert(Key::LoginScanQr, "扫码登录");
    m.insert(Key::LoginGeneratingQr, "生成二维码中...");
    m.insert(Key::LoginRefreshQr, "刷新二维码");
    m.insert(Key::LoginLoggedIn, "已登录");
    m.insert(Key::LoginLogout, "退出登录");

    // Delete Playlist Dialog
    m.insert(Key::DeletePlaylistTitle, "删除歌单");
    m.insert(Key::DeletePlaylistConfirm, "确定要删除这个歌单吗？");

    // Edit Playlist Dialog
    m.insert(Key::EditPlaylistTitle, "编辑歌单");
    m.insert(Key::EditPlaylistChangeCover, "更换封面");
    m.insert(Key::EditPlaylistName, "歌单名称");
    m.insert(Key::EditPlaylistNamePlaceholder, "输入歌单名称...");
    m.insert(Key::EditPlaylistDesc, "歌单描述");
    m.insert(Key::EditPlaylistDescPlaceholder, "输入歌单描述（可选）...");
    m.insert(Key::EditPlaylistWatchLibrary, "持续监听本地媒体库");
    m.insert(
        Key::EditPlaylistWatchLibraryDesc,
        "监听下面这个文件夹的变化，并自动同步到这个本地歌单。",
    );

    // Exit Dialog
    m.insert(Key::ExitDialogTitle, "退出应用");
    m.insert(
        Key::ExitDialogMessage,
        "你想要关闭应用还是最小化到系统托盘？",
    );
    m.insert(Key::ExitDialogExit, "退出");
    m.insert(Key::ExitDialogMinimize, "最小化到托盘");

    // 系统托盘
    m.insert(Key::TrayPlay, "播放");
    m.insert(Key::TrayPause, "暂停");
    m.insert(Key::TrayPrevious, "上一首");
    m.insert(Key::TrayNext, "下一首");
    m.insert(Key::TrayFavorite, "收藏");
    m.insert(Key::TrayUnfavorite, "取消收藏");
    m.insert(Key::TrayPlayMode, "播放模式");
    m.insert(Key::TraySequential, "顺序播放");
    m.insert(Key::TrayLoopAll, "列表循环");
    m.insert(Key::TrayLoopOne, "单曲循环");
    m.insert(Key::TrayShuffle, "随机播放");
    m.insert(Key::TrayToggleWindow, "显示/隐藏 Rustle");
    m.insert(Key::TrayQuit, "退出 Rustle");
    m.insert(Key::TrayNotPlaying, "未在播放");
    m.insert(
        Key::TrayUnavailable,
        "系统托盘当前不可用，Rustle 将保持窗口可见。",
    );

    // Context Menu
    m.insert(Key::ContextMenuEditTags, "编辑标签");

    // Song Info Dialog
    m.insert(Key::SongInfoTitle, "歌曲信息");
    m.insert(Key::SongInfoBasicSection, "基本信息");
    m.insert(Key::SongInfoDetailSection, "详细信息");
    m.insert(Key::SongInfoFileSection, "文件信息");
    m.insert(Key::SongInfoStatsSection, "统计信息");
    m.insert(Key::SongInfoTrackNumber, "曲目号");
    m.insert(Key::SongInfoYear, "年份");
    m.insert(Key::SongInfoGenre, "流派");
    m.insert(Key::SongInfoDuration, "时长");
    m.insert(Key::SongInfoFormat, "格式");
    m.insert(Key::SongInfoFilePath, "文件路径");
    m.insert(Key::SongInfoFileSize, "文件大小");
    m.insert(Key::SongInfoPlayCount, "播放次数");
    m.insert(Key::SongInfoLastPlayed, "最后播放");
    m.insert(Key::SongInfoEditButton, "编辑标签");
    m.insert(Key::SongInfoCloseButton, "关闭");

    // Song Edit Dialog
    m.insert(Key::SongEditTitle, "编辑歌曲标签");
    m.insert(Key::SongEditLabelTitle, "标题");
    m.insert(Key::SongEditLabelArtist, "艺人");
    m.insert(Key::SongEditLabelAlbum, "专辑");
    m.insert(Key::SongEditCoverReplace, "替换封面");
    m.insert(Key::SongEditCancel, "取消");
    m.insert(Key::SongEditSave, "保存");
    m.insert(Key::SongEditSaved, "标签已保存");
    m.insert(Key::SongEditFailed, "保存失败");
    m.insert(Key::SongAddedToPlaylist, "已添加到歌单");

    // Context Menu Items
    m.insert(Key::ContextMenuPlayNow, "立即播放");
    m.insert(Key::ContextMenuPlayNext, "下一首播放");
    m.insert(Key::ContextMenuAddFavorites, "收藏到喜欢");
    m.insert(Key::ContextMenuRemoveFavorites, "取消喜欢");
    m.insert(Key::ContextMenuAddPlaylist, "添加到歌单...");
    m.insert(Key::ContextMenuViewArtist, "查看歌手");
    m.insert(Key::ContextMenuViewAlbum, "查看专辑");
    m.insert(Key::ContextMenuShowInFolder, "在文件夹中显示");
    m.insert(Key::ContextMenuRemoveFromList, "从列表中移除");
    m.insert(Key::ContextMenuDownload, "下载");

    // Playlist Picker
    m.insert(Key::PlaylistPickerTitle, "选择歌单");
    m.insert(Key::PlaylistPickerConfirm, "确定");
    m.insert(Key::PlaylistPickerCancel, "取消");

    // Navigation
    m.insert(Key::NavDownloads, "下载");

    // Download
    m.insert(Key::DownloadPanelTitle, "下载管理");
    m.insert(Key::DownloadActive, "下载中");
    m.insert(Key::DownloadCompleted, "已完成");
    m.insert(Key::DownloadPending, "等待中");
    m.insert(Key::DownloadNoActive, "没有正在下载的任务");
    m.insert(Key::DownloadNoCompleted, "暂无已下载的文件");
    m.insert(Key::DownloadNoFailed, "暂无失败的任务");
    m.insert(Key::DownloadSpeed, "速度");
    m.insert(Key::DownloadCancel, "取消");
    m.insert(Key::DownloadRetry, "重试");
    m.insert(Key::DownloadQueued, "已加入下载队列");
    m.insert(Key::DownloadFailed, "下载失败");

    // Download Playlist Dialog
    m.insert(Key::DownloadPlaylistTitle, "下载歌单");
    m.insert(
        Key::DownloadPlaylistConfirm,
        "确定要下载歌单「{name}」中的 {count} 首歌曲吗？",
    );
    m.insert(Key::DownloadPlaylistConfirmBtn, "下载");

    // Settings - Download
    m.insert(Key::SettingsDownloadLocation, "下载位置");
    m.insert(Key::SettingsDownloadQuality, "下载音质");
    m.insert(
        Key::SettingsDownloadLocationDesc,
        "下载的歌曲将保存在此目录",
    );
    m.insert(Key::SettingsDownloadChange, "修改");

    m
});

pub fn translations() -> &'static HashMap<Key, &'static str> {
    &TRANSLATIONS
}
