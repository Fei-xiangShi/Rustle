//! NCM (Netease Cloud Music) related message handlers

use iced::Task;
use std::time::Duration;
use tracing::{debug, error};

use crate::api::{ArtistSummary, LoginInfo, NcmClient, PRIVATE_RADAR_PLAYLIST_ID, PlaylistSummary};
use crate::app::message::QrLoginStatus;
use crate::app::state::UserInfo;
use crate::app::{App, Message, Route};
use crate::i18n::Key;

use super::discover::is_login_only_feature_playlist;

/// Return `Some(toast_warning)` if user is not logged in, `None` if logged in.
macro_rules! require_logged_in {
    ($self:expr) => {
        if !$self.core.is_logged_in {
            return Some(Self::toast_warning(
                $self.core.locale.get(Key::NotLoggedIn).to_string(),
            ));
        }
    };
}

/// Return `Some(toast_warning)` if NCM client is not available, `None` if available.
macro_rules! require_ncm_client {
    ($self:expr) => {
        if $self.core.ncm_client.is_none() {
            return Some(Self::toast_warning(
                $self.core.locale.get(Key::NotLoggedIn).to_string(),
            ));
        }
    };
}

fn artist_page_id(artist_id: u64) -> i64 {
    i64::MIN + artist_id as i64
}

fn user_page_id(user_id: u64) -> i64 {
    (i64::MIN / 2) + user_id as i64
}

fn album_page_id(album_id: u64) -> i64 {
    (i64::MIN / 4) + album_id as i64
}

fn format_social_count(value: u64) -> String {
    if value >= 10_000 {
        format!("{:.1}万", value as f64 / 10_000.0)
    } else {
        value.to_string()
    }
}

fn exact_artist_id(keyword: &str, artists: &[ArtistSummary]) -> Option<u64> {
    let keyword = keyword.trim();
    artists
        .iter()
        .find(|artist| artist.name.trim() == keyword)
        .map(|artist| artist.id)
}

const NCM_PLAYLIST_BATCH_SIZE: usize = 120;

fn convert_ncm_playlist_chunks(
    playlist_id: i64,
    generation: u64,
    tracks: Vec<crate::api::Track>,
    cache_detail: Option<crate::api::PlaylistDetail>,
) -> Task<Message> {
    let batch_count = tracks.len().div_ceil(NCM_PLAYLIST_BATCH_SIZE);
    Task::run(
        async_stream::stream! {
            let mut all_tracks = Vec::with_capacity(tracks.len());
            let mut cache_detail = cache_detail;
            if tracks.is_empty() {
                if let Some(mut detail) = cache_detail.take() {
                    detail.tracks.clear();
                    crate::cache::save_ncm_playlist_cache(&detail).await;
                }
                yield Message::NcmPlaylistSongsChunk(
                    generation,
                    playlist_id,
                    Vec::new(),
                    Vec::new(),
                    true,
                );
            } else {
                for batch_index in 0..batch_count {
                    let start = batch_index * NCM_PLAYLIST_BATCH_SIZE;
                    let end = (start + NCM_PLAYLIST_BATCH_SIZE).min(tracks.len());
                    let batch = tracks[start..end].to_vec();
                    let batch_for_message = batch.clone();
                    all_tracks.extend(batch.iter().cloned());
                    let converted = tokio::task::spawn_blocking(move || {
                        crate::app::update::page_loader::convert_ncm_tracks_to_views_with_offset(
                            &batch,
                            start,
                        )
                    })
                    .await
                    .unwrap_or_default();
                    let is_last = batch_index + 1 == batch_count;
                    if is_last
                        && let Some(mut detail) = cache_detail.take() {
                            detail.tracks = all_tracks.clone();
                            crate::cache::save_ncm_playlist_cache(&detail).await;
                        }
                    yield Message::NcmPlaylistSongsChunk(
                        generation,
                        playlist_id,
                        batch_for_message,
                        converted,
                        is_last,
                    );
                }
            }
        },
        |message| message,
    )
}

fn fetch_ncm_playlist_chunks(
    client: NcmClient,
    playlist_id: i64,
    generation: u64,
    track_ids: Vec<u64>,
    cache_detail: Option<crate::api::PlaylistDetail>,
) -> Task<Message> {
    let batch_count = track_ids.len().div_ceil(NCM_PLAYLIST_BATCH_SIZE);
    Task::run(
        async_stream::stream! {
            let mut all_tracks = Vec::new();
            let mut cache_detail = cache_detail;
            if track_ids.is_empty() {
                if let Some(mut detail) = cache_detail.take() {
                    detail.tracks.clear();
                    crate::cache::save_ncm_playlist_cache(&detail).await;
                }
                yield Message::NcmPlaylistSongsChunk(
                    generation,
                    playlist_id,
                    Vec::new(),
                    Vec::new(),
                    true,
                );
            } else {
                for batch_index in 0..batch_count {
                    let start = batch_index * NCM_PLAYLIST_BATCH_SIZE;
                    let end = (start + NCM_PLAYLIST_BATCH_SIZE).min(track_ids.len());
                    let tracks = match client.track_detail(&track_ids[start..end]).await {
                        Ok(tracks) => tracks,
                        Err(error) => {
                            error!("Failed to load NCM playlist tracks: {:?}", error);
                            yield Message::NcmPlaylistLoadFailed(
                                generation,
                                playlist_id,
                                error.into(),
                            );
                            return;
                        }
                    };
                    let batch_for_message = tracks.clone();
                    all_tracks.extend(tracks.iter().cloned());
                    let batch_for_conversion = tracks.clone();
                    let converted = tokio::task::spawn_blocking(move || {
                        crate::app::update::page_loader::convert_ncm_tracks_to_views_with_offset(
                            &batch_for_conversion,
                            start,
                        )
                    })
                    .await
                    .unwrap_or_default();
                    let is_last = batch_index + 1 == batch_count;
                    if is_last
                        && let Some(mut detail) = cache_detail.take() {
                            detail.tracks = all_tracks.clone();
                            crate::cache::save_ncm_playlist_cache(&detail).await;
                        }
                    yield Message::NcmPlaylistSongsChunk(
                        generation,
                        playlist_id,
                        batch_for_message,
                        converted,
                        is_last,
                    );
                }
            }
        },
        |message| message,
    )
}

impl App {
    pub(crate) fn ncm_track_to_db_song(track: &crate::api::Track) -> crate::database::DbSong {
        let metadata = crate::metadata::SongMetadata::from(track);
        let mut db_song = metadata.to_db_song(-(track.id as i64));
        if db_song.format.is_none() {
            db_song.format = Some("mp3".to_string());
        }
        db_song
    }

    pub(super) fn set_ncm_scrobble_source(&mut self, source_id: Option<u64>) {
        self.playback.ncm_scrobble_source_id = source_id;
    }

    pub(super) fn current_route_ncm_scrobble_source(&self) -> Option<u64> {
        match self.ui.current_route {
            Route::NcmPlaylist(playlist_id) => (playlist_id != 0).then_some(playlist_id),
            Route::Album(album_id) => (album_id != 0).then_some(album_id),
            _ => None,
        }
    }

    fn handle_ncm_playlist_queue(
        &mut self,
        tracks: &[crate::api::Track],
        play_now: bool,
        source_id: Option<u64>,
    ) -> Option<Task<Message>> {
        debug!(
            "Adding {} NCM tracks to playlist, play_now: {}",
            tracks.len(),
            play_now
        );
        self.ui.home.current_ncm_playlist_songs = tracks.to_vec();

        let source_id = if self.is_fm_mode() { None } else { source_id };
        self.set_ncm_scrobble_source(source_id);

        let db_songs: Vec<crate::database::DbSong> =
            tracks.iter().map(Self::ncm_track_to_db_song).collect();

        if self.is_fm_mode() && !play_now {
            debug!("FM mode: appending {} songs to queue", db_songs.len());
            self.extend_queue_artist_metadata(tracks);
            self.clear_shuffle_cache();
            self.playback.queue.extend(db_songs);
            self.persist_queue_snapshot();
            self.cache_shuffle_indices();
            self.refresh_preload_window();
            return Some(self.preload_adjacent_tracks_with_ncm());
        }

        if play_now {
            self.replace_queue_artist_metadata(tracks);
            self.clear_shuffle_cache();
            self.playback.queue = db_songs;
            self.playback.current_index = Some(0);
            self.persist_queue_snapshot();
            Some(self.update(Message::PlayQueueIndex(0)))
        } else {
            self.extend_queue_artist_metadata(tracks);
            self.clear_shuffle_cache();
            self.playback.queue.extend(db_songs);
            self.persist_queue_snapshot();
            self.cache_shuffle_indices();
            self.refresh_preload_window();
            Some(self.preload_adjacent_tracks_with_ncm())
        }
    }

    /// Set the NCM client and sync request settings before its first request.
    fn set_ncm_client(&mut self, client: NcmClient) {
        client.set_quality(self.core.settings.playback.music_quality.to_api_rate());
        client.set_overseas_compatibility(self.core.settings.network.overseas_compatibility);
        self.core.ncm_client = Some(client);
    }

    pub(super) fn start_personal_fm_route(&mut self) -> Task<Message> {
        debug!("Starting Personal FM");

        self.enter_fm_mode();

        let prefetched_tracks = std::mem::take(&mut self.ui.discover.personal_fm_prefetched_tracks);
        if !prefetched_tracks.is_empty() {
            return Task::done(Message::AddNcmPlaylist(prefetched_tracks, true));
        }

        if let Some(client) = &self.core.ncm_client {
            let client = client.clone();
            let not_logged_in_msg = self.core.locale.get(Key::NotLoggedIn).to_string();

            Task::perform(
                async move {
                    match client.personal_fm_tracks().await {
                        Ok(tracks) if !tracks.is_empty() => Some(tracks),
                        Ok(_) => None,
                        Err(e) => {
                            error!("Failed to get personal FM: {}", e);
                            None
                        }
                    }
                },
                move |songs_opt| {
                    if let Some(songs) = songs_opt {
                        Message::AddNcmPlaylist(songs, true)
                    } else {
                        Message::ShowWarningToast(not_logged_in_msg)
                    }
                },
            )
        } else {
            self.exit_fm_mode();
            let msg = self.core.locale.get(Key::NotLoggedIn).to_string();
            Self::toast_warning(msg)
        }
    }

    pub(super) fn open_ncm_playlist_route(&mut self, playlist_id: u64) -> Task<Message> {
        let is_daily_recommend = playlist_id == 0;

        if !is_daily_recommend && self.is_viewing_ncm_playlist(playlist_id) {
            if let Some(ref mut playlist) = self.ui.playlist_page.current {
                for song in &mut playlist.songs {
                    song.source = crate::utils::compute_source(
                        "",
                        song.id,
                        Some(&song.artist),
                        Some(&song.title),
                    );
                }
            }
            return Task::none();
        }

        debug!("Opening NCM playlist: {}", playlist_id);
        if is_daily_recommend {
            self.invalidate_daily_recommend_cover();
        }
        self.reset_playlist_page_state();
        let generation = self.ui.playlist_page.ncm_load_generation;

        let (name, owner) = if is_daily_recommend {
            let locale = &self.core.locale;
            (
                locale
                    .get(crate::i18n::Key::DiscoverDailyRecommend)
                    .to_string(),
                locale
                    .get(crate::i18n::Key::DiscoverDailyRecommendCreator)
                    .to_string(),
            )
        } else {
            self.ui
                .home
                .user_playlists
                .iter()
                .find(|p| p.id == playlist_id)
                .map(|p| (p.name.clone(), p.creator.nickname.clone()))
                .unwrap_or_else(|| ("加载中...".to_string(), String::new()))
        };

        let internal_id = if is_daily_recommend {
            0
        } else {
            -(playlist_id as i64)
        };

        let skeleton_view = crate::ui::pages::PlaylistView {
            kind: crate::ui::pages::playlist::DetailPageKind::Playlist,
            id: internal_id,
            name,
            description: None,
            profile_stats: None,
            artist_tab: crate::ui::pages::playlist::ArtistPageTab::TopSongs,
            artist_albums: Vec::new(),
            user_playlists: Vec::new(),
            cover_path: None,
            owner,
            owner_artist_id: None,
            owner_avatar_path: None,
            creator_id: 0,
            song_count: 0,
            total_duration: String::new(),
            like_count: String::new(),
            songs: Vec::new(),
            palette: None,
            is_local: false,
            is_subscribed: false,
            watched_folder_path: None,
            watch_enabled: false,
        };

        self.ui.playlist_page.current = Some(skeleton_view);
        self.ui
            .playlist_page
            .begin_online_tracks(internal_id, generation);
        self.ui.playlist_page.ncm_replace_songs_on_chunk = true;
        self.ui.playlist_page.load_state =
            crate::app::update::page_loader::PlaylistLoadState::Loading;

        let Some(client) = &self.core.ncm_client else {
            self.ui.playlist_page.load_state =
                crate::app::update::page_loader::PlaylistLoadState::Idle;
            return Self::toast_warning(self.core.locale.get(Key::NotLoggedIn).to_string());
        };

        let client = client.clone();
        if is_daily_recommend {
            let locale = &self.core.locale;
            let name = locale
                .get(crate::i18n::Key::DiscoverDailyRecommend)
                .to_string();
            let desc = locale
                .get(crate::i18n::Key::DiscoverDailyRecommendDesc)
                .to_string();
            let creator = locale
                .get(crate::i18n::Key::DiscoverDailyRecommendCreator)
                .to_string();
            Task::perform(
                async move {
                    match client.recommend_tracks().await {
                        Ok(tracks) => {
                            let cover_url = tracks
                                .first()
                                .map(|track| track.cover_url().to_string())
                                .unwrap_or_default();
                            Ok(crate::api::PlaylistDetail {
                                id: 0,
                                name,
                                cover_url,
                                description: desc,
                                create_time: 0,
                                track_update_time: 0,
                                creator: crate::api::UserSummary {
                                    id: 0,
                                    nickname: creator,
                                    avatar_url: String::new(),
                                    vip: crate::api::VipInfo::default(),
                                },
                                track_count: tracks.len() as u64,
                                subscribed: false,
                                tracks,
                            })
                        }
                        Err(e) => {
                            error!("Failed to load daily recommend: {:?}", e);
                            Err(crate::error::AppError::from(e))
                        }
                    }
                },
                move |result| match result {
                    Ok(detail) => Message::NcmPlaylistDetailLoaded(generation, detail),
                    Err(message) => {
                        Message::NcmPlaylistLoadFailed(generation, internal_id, message)
                    }
                },
            )
        } else {
            Task::perform(
                async move { crate::cache::load_ncm_playlist_cache(playlist_id).await },
                move |cached| Message::NcmPlaylistCacheLoaded(generation, playlist_id, cached),
            )
        }
    }

    pub(super) fn open_album_route(&mut self, album_id: u64) -> Task<Message> {
        let route = Route::Album(album_id);
        if self.ui.current_route != route {
            return self.navigate_to_route(route, true);
        }

        let page_id = album_page_id(album_id);
        if self.ui.playlist_page.current.as_ref().map(|page| page.id) == Some(page_id)
            && !matches!(
                self.ui.playlist_page.load_state,
                crate::app::update::page_loader::PlaylistLoadState::Loading
            )
        {
            debug!("Already viewing album {}, skipping load", album_id);
            return Task::none();
        }

        if matches!(
            self.ui.playlist_page.load_state,
            crate::app::update::page_loader::PlaylistLoadState::Loading
        ) {
            debug!("Album page already loading, skipping");
            return Task::none();
        }

        let preview = self
            .ui
            .playlist_page
            .current
            .as_ref()
            .and_then(|page| {
                page.artist_albums
                    .iter()
                    .find(|album| album.id == album_id)
                    .map(|album| {
                        (
                            album.name.clone(),
                            album.artist_names(),
                            page.owner_artist_id,
                        )
                    })
            })
            .or_else(|| {
                self.ui
                    .search
                    .albums
                    .iter()
                    .find(|album| album.id == album_id)
                    .map(|album| {
                        (
                            album.name.clone(),
                            album.artist_names(),
                            album.primary_artist().map(|artist| artist.id),
                        )
                    })
            });

        let (name, owner, owner_artist_id) =
            preview.unwrap_or_else(|| ("加载中...".to_string(), String::new(), None));

        debug!("Opening album page: {}", album_id);
        self.reset_playlist_page_state();
        let generation = self.ui.playlist_page.ncm_load_generation;

        let skeleton_view = crate::ui::pages::PlaylistView {
            kind: crate::ui::pages::playlist::DetailPageKind::Album,
            id: page_id,
            name,
            description: None,
            profile_stats: None,
            artist_tab: crate::ui::pages::playlist::ArtistPageTab::TopSongs,
            artist_albums: Vec::new(),
            user_playlists: Vec::new(),
            cover_path: None,
            owner,
            owner_artist_id,
            owner_avatar_path: None,
            creator_id: 0,
            song_count: 0,
            total_duration: String::new(),
            like_count: String::new(),
            songs: Vec::new(),
            palette: None,
            is_local: false,
            is_subscribed: false,
            watched_folder_path: None,
            watch_enabled: false,
        };

        self.ui.playlist_page.current = Some(skeleton_view);
        self.ui
            .playlist_page
            .begin_online_tracks(page_id, generation);
        self.ui.playlist_page.load_state =
            crate::app::update::page_loader::PlaylistLoadState::Loading;

        if let Some(client) = &self.core.ncm_client {
            let client = client.clone();
            return Task::perform(
                async move { client.album_detail(album_id).await.ok() },
                move |result| {
                    if let Some(detail) = result {
                        Message::AlbumDetailLoaded(generation, detail)
                    } else {
                        Message::ShowErrorToast("加载专辑失败".to_string())
                    }
                },
            );
        }

        Self::toast_warning(self.core.locale.get(Key::NotLoggedIn).to_string())
    }

    pub(super) fn open_artist_route(&mut self, artist_id: u64) -> Task<Message> {
        let route = Route::Artist(artist_id);
        if self.ui.current_route != route {
            return self.navigate_to_route(route, true);
        }

        if matches!(
            self.ui.playlist_page.load_state,
            crate::app::update::page_loader::PlaylistLoadState::Loading
        ) {
            debug!("Artist page already loading, skipping");
            return Task::none();
        }

        debug!("Opening artist page: {}", artist_id);
        self.reset_playlist_page_state();
        let generation = self.ui.playlist_page.ncm_load_generation;

        let internal_id = artist_page_id(artist_id);
        let skeleton_view = crate::ui::pages::PlaylistView {
            kind: crate::ui::pages::playlist::DetailPageKind::Artist,
            id: internal_id,
            name: "加载中...".to_string(),
            description: None,
            profile_stats: Some("歌手".to_string()),
            artist_tab: crate::ui::pages::playlist::ArtistPageTab::TopSongs,
            artist_albums: Vec::new(),
            user_playlists: Vec::new(),
            cover_path: None,
            owner: "网易云音乐".to_string(),
            owner_artist_id: Some(artist_id),
            owner_avatar_path: None,
            creator_id: 0,
            song_count: 0,
            total_duration: String::new(),
            like_count: String::new(),
            songs: Vec::new(),
            palette: None,
            is_local: false,
            is_subscribed: false,
            watched_folder_path: None,
            watch_enabled: false,
        };

        self.ui.playlist_page.current = Some(skeleton_view);
        self.ui
            .playlist_page
            .begin_online_tracks(internal_id, generation);
        self.ui.playlist_page.load_state =
            crate::app::update::page_loader::PlaylistLoadState::Loading;

        if let Some(client) = &self.core.ncm_client {
            let client = client.clone();
            return Task::perform(
                async move { client.artist_detail(artist_id).await.ok() },
                move |result| {
                    if let Some(detail) = result {
                        Message::ArtistDetailLoaded(generation, detail)
                    } else {
                        Message::ShowErrorToast("加载歌手失败".to_string())
                    }
                },
            );
        }

        Self::toast_warning(self.core.locale.get(Key::NotLoggedIn).to_string())
    }

    pub(super) fn open_user_route(&mut self, user_id: u64) -> Task<Message> {
        let route = Route::User(user_id);
        if self.ui.current_route != route {
            return self.navigate_to_route(route, true);
        }

        if matches!(
            self.ui.playlist_page.load_state,
            crate::app::update::page_loader::PlaylistLoadState::Loading
        ) {
            debug!("User page already loading, skipping");
            return Task::none();
        }

        debug!("Opening user page: {}", user_id);
        self.reset_playlist_page_state();
        let generation = self.ui.playlist_page.ncm_load_generation;

        let page_id = user_page_id(user_id);
        let skeleton_view = crate::ui::pages::PlaylistView {
            kind: crate::ui::pages::playlist::DetailPageKind::User,
            id: page_id,
            name: "加载中...".to_string(),
            description: None,
            profile_stats: Some("关注 0 · 粉丝 0".to_string()),
            artist_tab: crate::ui::pages::playlist::ArtistPageTab::TopSongs,
            artist_albums: Vec::new(),
            user_playlists: Vec::new(),
            cover_path: None,
            owner: "网易云用户".to_string(),
            owner_artist_id: None,
            owner_avatar_path: None,
            creator_id: user_id,
            song_count: 0,
            total_duration: String::new(),
            like_count: String::new(),
            songs: Vec::new(),
            palette: None,
            is_local: false,
            is_subscribed: false,
            watched_folder_path: None,
            watch_enabled: false,
        };

        self.ui.playlist_page.current = Some(skeleton_view);
        self.ui
            .playlist_page
            .begin_online_tracks(page_id, generation);
        self.ui.playlist_page.load_state =
            crate::app::update::page_loader::PlaylistLoadState::Loading;

        if let Some(client) = &self.core.ncm_client {
            let client = client.clone();
            return Task::perform(
                async move {
                    client
                        .user_detail(user_id)
                        .await
                        .ok()
                        .map(|detail| (page_id, detail))
                },
                move |result| {
                    if let Some((page_id, detail)) = result {
                        Message::UserPageDetailLoaded(generation, page_id, detail)
                    } else {
                        Message::ShowErrorToast("加载用户失败".to_string())
                    }
                },
            );
        }

        Self::toast_warning(self.core.locale.get(Key::NotLoggedIn).to_string())
    }

    /// Handle NCM-related messages
    pub fn handle_ncm(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::TryAutoLogin(retry_count) => {
                let retry = *retry_count;
                let proxy_url = self.core.settings.network.proxy_url();
                let client = match NcmClient::from_session_file_with_proxy(proxy_url.clone()) {
                    Ok(Some(client)) => client,
                    Ok(None) => NcmClient::with_proxy(proxy_url),
                    Err(error) => {
                        tracing::warn!(
                            error_code = %error.code().as_str(),
                            "ncm_session_load_failed"
                        );
                        NcmClient::with_proxy(proxy_url)
                    }
                };
                let has_authenticated_session = client.has_authenticated_session();
                self.set_ncm_client(client.clone());

                if has_authenticated_session {
                    Some(Task::perform(
                        async move {
                            let login = client
                                .authenticated_login_status()
                                .await
                                .map_err(crate::error::AppError::from)?;
                            if let Err(error) = client.refresh_login().await {
                                tracing::warn!(
                                    error_code = %error.code().as_str(),
                                    "ncm_auto_login_refresh_failed"
                                );
                            }
                            Ok(login)
                        },
                        move |result| Message::AutoLoginResult(result, retry),
                    ))
                } else {
                    Some(Task::perform(
                        async move {
                            client
                                .ensure_anonymous_session()
                                .await
                                .map_err(crate::error::AppError::from)
                        },
                        Message::AnonymousSessionReady,
                    ))
                }
            }

            Message::AnonymousSessionReady(result) => {
                if let Err(error) = result {
                    tracing::warn!(
                        error_code = %error.code(),
                        "ncm_anonymous_session_bootstrap_failed"
                    );
                }
                Some(self.load_discover_data())
            }

            Message::AutoLoginResult(login_result, retry_count) => match login_result {
                Ok(login_info) => {
                    debug!("Auto login successful: {:?}", login_info);
                    self.core.is_logged_in = true;

                    if let Some(client) = &self.core.ncm_client {
                        client.save_cookie_to_file();
                    }

                    let mut user_info =
                        UserInfo::new(login_info.user_id, login_info.nickname.clone());
                    user_info.vip_type = login_info.vip_type;
                    user_info.vip = login_info.vip.clone();
                    self.core.user_info = Some(user_info);

                    let client = self.core.ncm_client.clone();
                    let user_id = login_info.user_id;

                    Some(Task::batch([
                        self.load_discover_data(),
                        Task::perform(
                            {
                                let client = client.clone();
                                async move {
                                    if let Some(client) = client
                                        && let Ok(song_ids) =
                                            client.user_song_id_list(user_id).await
                                    {
                                        let mut user_info = UserInfo::new(user_id, String::new());
                                        user_info.like_songs = song_ids.into_iter().collect();
                                        return user_info;
                                    }
                                    UserInfo::new(user_id, String::new())
                                }
                            },
                            Message::UserInfoLoaded,
                        ),
                        self.load_user_playlists(),
                    ]))
                }
                Err(error) => {
                    if error.code() == crate::error::ErrorCode::AuthenticationRequired {
                        self.core.is_logged_in = false;
                        self.core.user_info = None;
                        let Some(client) = self.core.ncm_client.clone() else {
                            return Some(self.load_discover_data());
                        };
                        return Some(Task::perform(
                            async move {
                                client
                                    .invalidate_authenticated_session()
                                    .map_err(crate::error::AppError::from)?;
                                client
                                    .ensure_anonymous_session()
                                    .await
                                    .map_err(crate::error::AppError::from)
                            },
                            Message::AnonymousSessionReady,
                        ));
                    }
                    // Auto login failed - retry up to 3 times
                    const MAX_RETRIES: u8 = 3;
                    let retry = *retry_count;
                    if retry < MAX_RETRIES {
                        tracing::warn!(
                            attempt = retry + 1,
                            max_attempts = MAX_RETRIES + 1,
                            error_code = %error.code(),
                            "ncm_auto_login_retrying"
                        );
                        // Wait 1 seconds before retry
                        Some(Task::perform(
                            async move {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            },
                            move |_| Message::TryAutoLogin(retry + 1),
                        ))
                    } else {
                        tracing::warn!(
                            attempts = MAX_RETRIES + 1,
                            error_code = %error.code(),
                            "ncm_auto_login_retry_exhausted"
                        );
                        Some(self.load_discover_data())
                    }
                }
            },

            Message::RequestQrCode => {
                self.ui.home.login_popup_open = true;
                self.ui.home.qr_status = Some("正在生成二维码...".to_string());
                // Clear old QR code data to force refresh
                self.ui.home.qr_code_path = None;
                self.ui.home.qr_unikey = None;

                let client = self.core.ncm_client.clone().unwrap_or_default();

                Some(Task::perform(
                    async move {
                        match client.create_qrcode().await {
                            Ok((path, unikey)) => Some((path, unikey)),
                            Err(e) => {
                                error!("Failed to create QR code: {:?}", e);
                                None
                            }
                        }
                    },
                    |result| {
                        if let Some((path, unikey)) = result {
                            Message::QrCodeReady(path, unikey)
                        } else {
                            Message::ShowErrorToast("生成二维码失败".to_string())
                        }
                    },
                ))
            }

            Message::QrCodeReady(path, unikey) => {
                self.ui.home.qr_code_path = Some(path.clone());
                self.ui.home.qr_unikey = Some(unikey.clone());
                self.ui.home.qr_status = Some("请使用网易云音乐App扫码登录".to_string());

                let unikey = unikey.clone();
                Some(Task::done(Message::CheckQrStatus(unikey)))
            }

            Message::CheckQrStatus(unikey) => {
                let current_unikey = self.ui.home.qr_unikey.clone();
                if current_unikey.as_ref() != Some(unikey) {
                    return Some(Task::none());
                }

                let client = self.core.ncm_client.clone().unwrap_or_default();
                let unikey = unikey.clone();

                Some(Task::perform(
                    async move {
                        match client.login_qr_check(unikey.clone()).await {
                            Ok(msg) => match msg.code {
                                800 => QrLoginStatus::Expired,
                                801 => QrLoginStatus::WaitingForScan,
                                802 => QrLoginStatus::WaitingForConfirm,
                                803 => QrLoginStatus::Success,
                                _ => QrLoginStatus::Error(
                                    crate::error::AppError::with_message_source(
                                        crate::error::ErrorCode::ProtocolInvalidResponse,
                                        "The login service returned an unknown status",
                                        format!("Unknown QR login code: {}", msg.code),
                                    ),
                                ),
                            },
                            Err(e) => QrLoginStatus::Error(e.into()),
                        }
                    },
                    Message::QrLoginResult,
                ))
            }

            Message::QrLoginResult(status) => match status {
                QrLoginStatus::WaitingForScan => {
                    self.ui.home.qr_status = Some("等待扫码...".to_string());
                    let unikey = self.ui.home.qr_unikey.clone();
                    if let Some(unikey) = unikey {
                        Some(Task::perform(
                            async move {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                unikey
                            },
                            Message::CheckQrStatus,
                        ))
                    } else {
                        Some(Task::none())
                    }
                }
                QrLoginStatus::WaitingForConfirm => {
                    self.ui.home.qr_status = Some("已扫码，请在App中确认登录".to_string());
                    let unikey = self.ui.home.qr_unikey.clone();
                    if let Some(unikey) = unikey {
                        Some(Task::perform(
                            async move {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                unikey
                            },
                            Message::CheckQrStatus,
                        ))
                    } else {
                        Some(Task::none())
                    }
                }
                QrLoginStatus::Expired => {
                    self.ui.home.qr_status = Some("二维码已过期，请刷新".to_string());
                    self.ui.home.login_popup_open = false;
                    Some(Self::toast_error("二维码已过期".to_string()))
                }
                QrLoginStatus::Success => {
                    self.ui.home.qr_status = Some("登录成功！".to_string());

                    if let Some(client) = &self.core.ncm_client {
                        let client = client.clone();
                        return Some(Task::perform(
                            async move {
                                match client.login_status().await {
                                    Ok(login_info) => {
                                        client.save_cookie_to_file();
                                        login_info
                                    }
                                    Err(e) => {
                                        error!("Failed to get login status: {:?}", e);
                                        LoginInfo::default()
                                    }
                                }
                            },
                            Message::LoginSuccess,
                        ));
                    }
                    Some(Task::none())
                }
                QrLoginStatus::Error(err) => {
                    self.ui.home.qr_status = Some(format!("登录错误: {}", err));
                    self.ui.home.login_popup_open = false;
                    Some(Self::toast_error(format!("登录失败: {}", err)))
                }
            },

            Message::LoginSuccess(login_info) => {
                debug!("Login successful: {:?}", login_info);
                self.core.is_logged_in = true;
                self.ui.home.login_popup_open = false;

                let mut user_info = UserInfo::new(login_info.user_id, login_info.nickname.clone());
                user_info.vip_type = login_info.vip_type;
                user_info.vip = login_info.vip.clone();
                self.core.user_info = Some(user_info);

                Some(Task::batch([
                    Self::toast_success("登录成功！".to_string()),
                    self.load_discover_data(),
                    self.load_user_playlists(),
                ]))
            }

            Message::Logout => {
                if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    return Some(Task::perform(
                        async move { client.logout().await.map_err(crate::error::AppError::from) },
                        Message::LogoutCompleted,
                    ));
                }

                Some(Task::done(Message::LogoutCompleted(Ok(()))))
            }

            Message::LogoutCompleted(result) => {
                self.core.is_logged_in = false;
                self.core.user_info = None;
                self.ui.home.user_playlists.clear();
                let mut tasks = vec![self.load_discover_data()];
                match result {
                    Ok(()) => tasks.push(Self::toast_success("已退出登录".to_string())),
                    Err(error) => {
                        tracing::warn!(
                            error_code = %error.code(),
                            "ncm_logout_session_finalize_failed"
                        );
                        tasks.push(Self::toast_warning(
                            "已退出登录，但本地会话状态保存失败".to_string(),
                        ));
                    }
                }
                Some(Task::batch(tasks))
            }

            Message::UserInfoLoaded(user_info) => {
                if let Some(existing) = &mut self.core.user_info {
                    existing.like_songs = user_info.like_songs.clone();
                } else {
                    self.core.user_info = Some(user_info.clone());
                }
                Some(Task::none())
            }

            Message::ToggleLoginPopup => {
                self.ui.home.login_popup_open = !self.ui.home.login_popup_open;
                if self.ui.home.login_popup_open && self.ui.home.qr_code_path.is_none() {
                    Some(Task::done(Message::RequestQrCode))
                } else {
                    Some(Task::none())
                }
            }

            Message::ToggleFavorite(song_id) => {
                require_logged_in!(self);

                if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    let is_liked = if let Some(ref user_info) = self.core.user_info {
                        user_info.like_songs.contains(song_id)
                    } else {
                        false
                    };
                    let song_id = *song_id;

                    Some(Task::perform(
                        async move {
                            match client.like_song(song_id, !is_liked).await {
                                Ok(result) => Some(result.liked),
                                Err(error) => {
                                    tracing::warn!(
                                        song_id,
                                        error_code = %error.code().as_str(),
                                        "ncm_like_mutation_failed"
                                    );
                                    None
                                }
                            }
                        },
                        move |result| {
                            if let Some(liked) = result {
                                Message::FavoriteStatusChanged(song_id, liked)
                            } else {
                                Message::ShowErrorToast("操作失败".to_string())
                            }
                        },
                    ))
                } else {
                    Some(Task::none())
                }
            }

            Message::FavoriteStatusChanged(song_id, liked) => {
                if let Some(ref mut user_info) = self.core.user_info {
                    if *liked {
                        user_info.like_songs.insert(*song_id);
                    } else {
                        user_info.like_songs.remove(song_id);
                    }
                }

                // Update tray state if this is the current song
                if let Some(current) = &self.playback.current_song
                    && current.id < 0
                    && (-current.id) as u64 == *song_id
                {
                    let is_playing = self.playback_is_playing();
                    crate::app::helpers::update_tray_state_with_favorite(
                        is_playing,
                        Some(current.title.clone()),
                        Some(current.artist.clone()),
                        self.core.settings.play_mode,
                        Some(*song_id),
                        *liked,
                        self.core.locale.language,
                    );
                }

                Some(Task::none())
            }

            Message::PlayNcmSong(song_info) => {
                debug!("Playing NCM song: {}", song_info.title);

                require_ncm_client!(self);

                self.exit_fm_mode();
                self.ui.home.current_ncm_playlist_songs = vec![song_info.clone()];
                self.set_ncm_scrobble_source(None);
                self.replace_queue_artist_metadata(std::slice::from_ref(song_info));
                self.clear_shuffle_cache();
                self.playback.queue.clear();
                self.playback
                    .queue
                    .push(Self::ncm_track_to_db_song(song_info));
                self.persist_queue_snapshot();

                Some(self.play_song_at_index(0))
            }

            Message::AddNcmPlaylist(songs, play_now) => {
                self.handle_ncm_playlist_queue(songs, *play_now, None)
            }

            Message::AddNcmPlaylistWithSource(songs, play_now, source_id) => {
                self.handle_ncm_playlist_queue(songs, *play_now, *source_id)
            }

            Message::UserPlaylistsLoaded(playlists) => {
                self.ui.home.user_playlists = playlists.clone();
                Some(Task::none())
            }

            Message::OpenNcmPlaylist(playlist_id) => {
                if !self.core.is_logged_in && is_login_only_feature_playlist(*playlist_id) {
                    return Some(Self::toast_warning(
                        self.core.locale.get(Key::NotLoggedIn).to_string(),
                    ));
                }

                let route = Route::NcmPlaylist(*playlist_id);
                if self.ui.current_route != route {
                    return Some(self.navigate_to_route(route, true));
                }

                Some(self.open_ncm_playlist_route(*playlist_id))
            }

            Message::OpenArtist(artist_id) => Some(self.open_artist_route(*artist_id)),

            Message::SwitchArtistTab(tab) => {
                if let Some(playlist) = &mut self.ui.playlist_page.current {
                    playlist.artist_tab = *tab;
                }
                self.ui.smooth_scroll.cancel_all();
                self.ui.playlist_page.scroll_state.borrow_mut().jump_to(0.0);
                Some(iced::widget::operation::snap_to(
                    iced::widget::Id::new("playlist_scroll"),
                    iced::widget::scrollable::RelativeOffset { x: 0.0, y: 0.0 },
                ))
            }

            Message::OpenArtistByName(name) => {
                let keyword = name.trim().to_string();
                if keyword.is_empty() {
                    return Some(Task::none());
                }

                if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    Some(Task::perform(
                        async move {
                            client
                                .search(&keyword, crate::api::SearchType::Artists, 20, 0)
                                .await
                                .ok()
                                .and_then(|response| exact_artist_id(&keyword, &response.artists))
                        },
                        |result| {
                            if let Some(artist_id) = result {
                                Message::OpenArtist(artist_id)
                            } else {
                                Message::ShowWarningToast("未找到对应歌手".to_string())
                            }
                        },
                    ))
                } else {
                    Some(Self::toast_warning(
                        self.core.locale.get(Key::NotLoggedIn).to_string(),
                    ))
                }
            }

            Message::NcmPlaylistCacheLoaded(generation, playlist_id, cached) => {
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !matches!(self.ui.current_route, Route::NcmPlaylist(id) if id == *playlist_id)
                {
                    return Some(Task::none());
                }
                let generation = *generation;
                let internal_id = -(*playlist_id as i64);
                let refresh_id = *playlist_id;
                let Some(client) = &self.core.ncm_client else {
                    return Some(Self::toast_warning(
                        self.core.locale.get(Key::NotLoggedIn).to_string(),
                    ));
                };

                if let Some(detail) = cached {
                    let total_secs: u64 = detail.tracks.iter().map(|s| s.duration_ms / 1000).sum();
                    let total_mins = total_secs / 60;
                    let total_duration = if total_mins / 60 > 0 {
                        format!("约 {} 小时 {} 分钟", total_mins / 60, total_mins % 60)
                    } else {
                        format!("{} 分钟", total_mins)
                    };
                    if let Some(playlist) = &mut self.ui.playlist_page.current
                        && playlist.id == internal_id
                    {
                        playlist.name = detail.name.clone();
                        playlist.description =
                            (!detail.description.is_empty()).then(|| detail.description.clone());
                        playlist.owner = detail.creator.nickname.clone();
                        playlist.creator_id = detail.creator.id;
                        playlist.song_count = detail.track_count as u32;
                        playlist.total_duration = total_duration;
                        playlist.is_subscribed = detail.subscribed;
                    }
                    self.ui.playlist_page.ncm_cache_baseline = Some(detail.clone());
                    self.ui.playlist_page.ncm_replace_songs_on_chunk = false;
                    let mut cached_views =
                        crate::app::update::page_loader::convert_ncm_tracks_to_views(
                            &detail.tracks,
                        );
                    for song in &mut cached_views {
                        song.source = crate::utils::compute_source(
                            "",
                            song.id,
                            Some(&song.artist),
                            Some(&song.title),
                        );
                    }
                    if let Some(playlist) = &mut self.ui.playlist_page.current
                        && playlist.id == internal_id
                    {
                        playlist.songs = cached_views;
                    }
                    self.ui.playlist_page.replace_online_tracks(
                        internal_id,
                        generation,
                        detail.tracks.clone(),
                    );
                    self.ui.home.current_ncm_playlist_songs = detail.tracks.clone();
                    self.ui.playlist_page.load_state =
                        crate::app::update::page_loader::PlaylistLoadState::Ready;
                    let refresh_client = client.clone();
                    let refresh_task = Task::perform(
                        async move { refresh_client.playlist_detail_preview(refresh_id).await },
                        move |result| match result {
                            Ok((detail, track_ids)) => {
                                Message::NcmPlaylistPreviewLoaded(generation, detail, track_ids)
                            }
                            Err(error) => {
                                error!("Failed to refresh NCM playlist metadata: {:?}", error);
                                Message::NoOp
                            }
                        },
                    );
                    Some(refresh_task)
                } else {
                    let client = client.clone();
                    Some(Task::perform(
                        async move { client.playlist_detail_preview(refresh_id).await },
                        move |result| match result {
                            Ok((detail, track_ids)) => {
                                Message::NcmPlaylistPreviewLoaded(generation, detail, track_ids)
                            }
                            Err(error) => {
                                error!("Failed to load NCM playlist metadata: {:?}", error);
                                Message::NcmPlaylistLoadFailed(
                                    generation,
                                    internal_id,
                                    error.into(),
                                )
                            }
                        },
                    ))
                }
            }

            Message::NcmPlaylistPreviewLoaded(generation, detail, track_ids) => {
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !matches!(self.ui.current_route, Route::NcmPlaylist(id) if id == detail.id)
                {
                    return Some(Task::none());
                }
                let generation = *generation;
                let playlist_id = -(detail.id as i64);
                let baseline = self.ui.playlist_page.ncm_cache_baseline.take();
                let unchanged = baseline.as_ref().is_some_and(|cached| {
                    cached.track_update_time == detail.track_update_time
                        && cached.track_count == detail.track_count
                });

                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == playlist_id
                {
                    playlist.name = detail.name.clone();
                    playlist.description =
                        (!detail.description.is_empty()).then(|| detail.description.clone());
                    playlist.owner = if detail.creator.nickname.is_empty() {
                        "网易云音乐".to_string()
                    } else {
                        detail.creator.nickname.clone()
                    };
                    playlist.creator_id = detail.creator.id;
                    playlist.song_count = detail.track_count as u32;
                    playlist.is_subscribed = detail.subscribed;
                }

                if unchanged {
                    self.ui.playlist_page.load_state =
                        crate::app::update::page_loader::PlaylistLoadState::Ready;
                    return Some(Task::none());
                }

                self.ui.playlist_page.ncm_replace_songs_on_chunk = true;
                self.ui.home.current_ncm_playlist_songs.clear();
                self.ui.playlist_page.load_state =
                    crate::app::update::page_loader::PlaylistLoadState::Loading;

                let Some(client) = &self.core.ncm_client else {
                    return Some(Self::toast_warning(
                        self.core.locale.get(Key::NotLoggedIn).to_string(),
                    ));
                };
                let track_task = fetch_ncm_playlist_chunks(
                    client.clone(),
                    playlist_id,
                    generation,
                    track_ids.clone(),
                    Some(detail.clone()),
                );
                let creator_task = if detail.creator.id != 0 {
                    let creator_client = client.clone();
                    let creator_id = detail.creator.id;
                    Task::perform(
                        async move { creator_client.user_detail(creator_id).await.ok() },
                        move |result| {
                            result
                                .map(|detail| {
                                    Message::NcmPlaylistCreatorDetailLoaded(
                                        generation,
                                        playlist_id,
                                        detail,
                                    )
                                })
                                .unwrap_or(Message::NoOp)
                        },
                    )
                } else {
                    Task::none()
                };
                Some(Task::batch([track_task, creator_task]))
            }

            Message::NcmPlaylistDetailLoaded(generation, detail) => {
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !matches!(self.ui.current_route, Route::NcmPlaylist(id) if id == detail.id)
                {
                    return Some(Task::none());
                }
                let generation = *generation;
                debug!(
                    "NCM playlist detail loaded: {} with {} tracks",
                    detail.name,
                    detail.tracks.len()
                );

                let playlist_id = -(detail.id as i64);

                // Calculate total duration
                let total_secs: u64 = detail.tracks.iter().map(|s| s.duration_ms / 1000).sum();
                let total_mins = total_secs / 60;
                let total_hours = total_mins / 60;
                let remaining_mins = total_mins % 60;
                let total_duration = if total_hours > 0 {
                    format!("约 {} 小时 {} 分钟", total_hours, remaining_mins)
                } else {
                    format!("{} 分钟", total_mins)
                };

                // Update existing PlaylistView with full details (keep cover_path if already loaded)
                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == playlist_id
                {
                    playlist.name = detail.name.clone();
                    playlist.description = if detail.description.is_empty() {
                        None
                    } else {
                        Some(detail.description.clone())
                    };
                    playlist.owner = if detail.creator.nickname.is_empty() {
                        "网易云音乐".to_string()
                    } else {
                        detail.creator.nickname.clone()
                    };
                    playlist.creator_id = detail.creator.id;
                    playlist.song_count = detail.track_count as u32;
                    playlist.total_duration = total_duration;
                    playlist.is_subscribed = detail.subscribed;
                }

                // Raw tracks are appended as conversion batches arrive. This
                // keeps playback state and the visible list on the same load
                // lifecycle.
                self.ui.home.current_ncm_playlist_songs.clear();

                let creator_detail_task = if detail.creator.id != 0 {
                    if let Some(client) = &self.core.ncm_client {
                        let client = client.clone();
                        let creator_id = detail.creator.id;
                        let internal_id = playlist_id;
                        Task::perform(
                            async move { client.user_detail(creator_id).await.ok() },
                            move |result| {
                                if let Some(user_detail) = result {
                                    Message::NcmPlaylistCreatorDetailLoaded(
                                        generation,
                                        internal_id,
                                        user_detail,
                                    )
                                } else {
                                    Message::NoOp
                                }
                            },
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                };

                let tracks_task = convert_ncm_playlist_chunks(
                    playlist_id,
                    generation,
                    detail.tracks.clone(),
                    None,
                );

                Some(Task::batch([tracks_task, creator_detail_task]))
            }

            Message::NcmPlaylistLoadFailed(generation, playlist_id, message) => {
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !self
                        .ui
                        .playlist_page
                        .current
                        .as_ref()
                        .is_some_and(|playlist| playlist.id == *playlist_id)
                {
                    return Some(Task::none());
                }
                self.ui.playlist_page.load_state =
                    crate::app::update::page_loader::PlaylistLoadState::Idle;
                Some(Self::toast_error(message.user_summary().to_owned()))
            }

            Message::NcmPlaylistSongsChunk(
                generation,
                playlist_id,
                tracks,
                song_views,
                is_last,
            ) => {
                if *generation != self.ui.playlist_page.ncm_load_generation {
                    return Some(Task::none());
                }
                let is_current = self
                    .ui
                    .playlist_page
                    .current
                    .as_ref()
                    .is_some_and(|playlist| playlist.id == *playlist_id);
                if !is_current {
                    return Some(Task::none());
                }
                if self.ui.playlist_page.ncm_replace_songs_on_chunk {
                    if !self.ui.playlist_page.replace_online_tracks(
                        *playlist_id,
                        *generation,
                        tracks.clone(),
                    ) {
                        tracing::warn!(
                            page_id = *playlist_id,
                            generation = *generation,
                            "playlist_track_owner_rejected_replacement"
                        );
                        return Some(Task::none());
                    }
                    if let Some(playlist) = &mut self.ui.playlist_page.current
                        && playlist.id == *playlist_id
                    {
                        playlist.songs.clear();
                    }
                    self.ui.home.current_ncm_playlist_songs.clear();
                    self.ui.playlist_page.ncm_replace_songs_on_chunk = false;
                } else if !self.ui.playlist_page.append_online_tracks(
                    *playlist_id,
                    *generation,
                    tracks,
                ) {
                    tracing::warn!(
                        page_id = *playlist_id,
                        generation = *generation,
                        "playlist_track_owner_rejected_append"
                    );
                    return Some(Task::none());
                }
                self.ui
                    .home
                    .current_ncm_playlist_songs
                    .extend(tracks.iter().cloned());
                let mut song_views = song_views.clone();
                debug!(
                    "NCM playlist songs chunk ready: {} tracks / {} songs (last={})",
                    tracks.len(),
                    song_views.len(),
                    is_last
                );

                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == *playlist_id
                {
                    for song in &mut song_views {
                        song.source = crate::utils::compute_source(
                            "",
                            song.id,
                            Some(&song.artist),
                            Some(&song.title),
                        );
                    }
                    playlist.songs.extend(song_views);
                    if *is_last {
                        self.ui.playlist_page.load_state =
                            crate::app::update::page_loader::PlaylistLoadState::Ready;
                    }
                }

                if *is_last {
                    self.ui.playlist_page.scroll_state.borrow_mut().jump_to(0.0);
                    Some(Task::none())
                } else {
                    Some(Task::none())
                }
            }

            Message::NcmPlaylistSongsReady(generation, playlist_id, song_views) => {
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !self
                        .ui
                        .playlist_page
                        .current
                        .as_ref()
                        .is_some_and(|playlist| playlist.id == *playlist_id)
                {
                    return Some(Task::none());
                }
                let mut song_views = song_views.clone();

                debug!("NCM playlist songs ready: {} songs", song_views.len());

                // Update existing playlist view with songs
                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == *playlist_id
                {
                    // Recompute sources with real download dir (async callers used None)
                    for song in &mut song_views {
                        song.source = crate::utils::compute_source(
                            "",
                            song.id,
                            Some(&song.artist),
                            Some(&song.title),
                        );
                    }
                    playlist.songs = song_views.clone();
                }

                // Update load state
                self.ui.playlist_page.load_state =
                    crate::app::update::page_loader::PlaylistLoadState::Ready;

                // Scroll to top
                self.ui.playlist_page.scroll_state.borrow_mut().jump_to(0.0);
                Some(Task::none())
            }

            Message::AlbumDetailLoaded(generation, detail) => {
                let page_id = album_page_id(detail.id);
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !self
                        .ui
                        .playlist_page
                        .current
                        .as_ref()
                        .is_some_and(|playlist| playlist.id == page_id)
                {
                    return Some(Task::none());
                }
                let generation = *generation;
                debug!(
                    "Album detail loaded: {} with {} tracks",
                    detail.name,
                    detail.tracks.len()
                );

                let total_secs: u64 = detail
                    .tracks
                    .iter()
                    .map(|song| song.duration_ms / 1000)
                    .sum();
                let total_mins = total_secs / 60;
                let total_hours = total_mins / 60;
                let remaining_mins = total_mins % 60;
                let total_duration = if total_hours > 0 {
                    format!("约 {} 小时 {} 分钟", total_hours, remaining_mins)
                } else {
                    format!("{} 分钟", total_mins)
                };

                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == page_id
                {
                    playlist.kind = crate::ui::pages::playlist::DetailPageKind::Album;
                    playlist.name = detail.name.clone();
                    playlist.description = (!detail.description.trim().is_empty())
                        .then_some(detail.description.clone());
                    let artist_name = detail.artist_names();
                    playlist.owner = if artist_name.trim().is_empty() {
                        "网易云音乐".to_string()
                    } else {
                        artist_name
                    };
                    playlist.owner_artist_id = detail.primary_artist().map(|artist| artist.id);
                    playlist.song_count = detail.track_count.max(detail.tracks.len() as u32);
                    playlist.total_duration = total_duration;
                    playlist.like_count.clear();
                }

                self.ui.playlist_page.replace_online_tracks(
                    page_id,
                    generation,
                    detail.tracks.clone(),
                );
                self.ui.home.current_ncm_playlist_songs = detail.tracks.clone();

                let tracks = detail.tracks.clone();
                let tracks_task = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::app::update::page_loader::convert_ncm_tracks_to_views(&tracks)
                        })
                        .await
                        .unwrap_or_default()
                    },
                    move |song_views| {
                        Message::NcmPlaylistSongsReady(generation, page_id, song_views)
                    },
                );

                Some(tracks_task)
            }

            Message::ArtistDetailLoaded(generation, detail) => {
                let page_id = artist_page_id(detail.id);
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !self
                        .ui
                        .playlist_page
                        .current
                        .as_ref()
                        .is_some_and(|playlist| playlist.id == page_id)
                {
                    return Some(Task::none());
                }
                let generation = *generation;
                debug!(
                    "Artist detail loaded: {} with {} tracks",
                    detail.name,
                    detail.top_tracks.len()
                );

                let total_secs: u64 = detail.top_tracks.iter().map(|s| s.duration_ms / 1000).sum();
                let total_mins = total_secs / 60;
                let total_hours = total_mins / 60;
                let remaining_mins = total_mins % 60;
                let total_duration = if total_hours > 0 {
                    format!("约 {} 小时 {} 分钟", total_hours, remaining_mins)
                } else {
                    format!("{} 分钟", total_mins)
                };

                let description = if detail.description.trim().is_empty() {
                    Some(format!(
                        "{} 首热门单曲 · {} 张专辑",
                        detail.track_count, detail.album_count
                    ))
                } else {
                    Some(detail.description.clone())
                };

                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == page_id
                {
                    playlist.kind = crate::ui::pages::playlist::DetailPageKind::Artist;
                    playlist.name = detail.name.clone();
                    playlist.description = description;
                    playlist.profile_stats = Some(format!(
                        "{} 首热门单曲 · {} 张专辑",
                        detail.track_count, detail.album_count
                    ));
                    playlist.owner = "歌手热门作品".to_string();
                    playlist.owner_artist_id = Some(detail.id);
                    playlist.song_count = detail.top_tracks.len() as u32;
                    playlist.total_duration = total_duration;
                    playlist.like_count = format!("{} 张专辑", detail.album_count);
                }

                self.ui.playlist_page.replace_online_tracks(
                    page_id,
                    generation,
                    detail.top_tracks.clone(),
                );
                self.ui.home.current_ncm_playlist_songs = detail.top_tracks.clone();

                let tracks = detail.top_tracks.clone();
                let tracks_task = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::app::update::page_loader::convert_ncm_tracks_to_views(&tracks)
                        })
                        .await
                        .unwrap_or_default()
                    },
                    move |song_views| {
                        Message::NcmPlaylistSongsReady(generation, page_id, song_views)
                    },
                );

                let albums_task = if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    let artist_id = detail.id;
                    Task::perform(
                        async move {
                            client
                                .artist_albums(artist_id, 50)
                                .await
                                .ok()
                                .map(|mut albums| {
                                    albums.sort_by(|a, b| a.name.cmp(&b.name));
                                    (page_id, albums)
                                })
                        },
                        move |result| {
                            if let Some((page_id, albums)) = result {
                                Message::ArtistAlbumsLoaded(page_id, albums)
                            } else {
                                Message::ArtistAlbumsLoaded(page_id, Vec::new())
                            }
                        },
                    )
                } else {
                    Task::none()
                };

                Some(Task::batch([tracks_task, albums_task]))
            }

            Message::ArtistAlbumsLoaded(page_id, albums) => {
                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == *page_id
                {
                    playlist.artist_albums = albums.clone();
                }
                Some(Task::none())
            }

            Message::UserPageDetailLoaded(generation, page_id, detail) => {
                let page_id = *page_id;
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !self
                        .ui
                        .playlist_page
                        .current
                        .as_ref()
                        .is_some_and(|playlist| playlist.id == page_id)
                {
                    return Some(Task::none());
                }
                let generation = *generation;
                let description = if !detail.signature.trim().is_empty() {
                    Some(detail.signature.clone())
                } else if detail.artist_id != 0 {
                    Some("网易云音乐人".to_string())
                } else {
                    Some(format!("网易云用户 {}", detail.nickname))
                };

                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == page_id
                {
                    playlist.kind = crate::ui::pages::playlist::DetailPageKind::User;
                    playlist.name = detail.nickname.clone();
                    playlist.description = description;
                    playlist.profile_stats = Some(format!(
                        "关注 {} · 粉丝 {}",
                        format_social_count(detail.follows),
                        format_social_count(detail.followeds)
                    ));
                    playlist.owner = if detail.artist_name.trim().is_empty() {
                        "网易云用户".to_string()
                    } else {
                        detail.artist_name.clone()
                    };
                    playlist.owner_artist_id = (detail.artist_id != 0).then_some(detail.artist_id);
                    playlist.creator_id = detail.user_id;
                    playlist.like_count.clear();
                }

                let artist_task = if detail.artist_id != 0 {
                    if let Some(client) = &self.core.ncm_client {
                        let client = client.clone();
                        let artist_id = detail.artist_id;
                        Task::perform(
                            async move {
                                client
                                    .artist_detail(artist_id)
                                    .await
                                    .ok()
                                    .map(|detail| (page_id, detail))
                            },
                            move |result| {
                                if let Some((page_id, detail)) = result {
                                    Message::UserArtistDetailLoaded(generation, page_id, detail)
                                } else {
                                    Message::NoOp
                                }
                            },
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    self.ui.playlist_page.load_state =
                        crate::app::update::page_loader::PlaylistLoadState::Ready;
                    Task::none()
                };

                let playlist_task = if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    let user_id = detail.user_id;
                    Task::perform(
                        async move { client.user_playlists(user_id, 0, 30).await.ok() },
                        move |result| {
                            if let Some(playlists) = result {
                                Message::UserPagePlaylistsLoaded(page_id, playlists)
                            } else {
                                Message::UserPagePlaylistsLoaded(page_id, Vec::new())
                            }
                        },
                    )
                } else {
                    Task::none()
                };

                Some(Task::batch([artist_task, playlist_task]))
            }

            Message::UserPagePlaylistsLoaded(page_id, playlists) => {
                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == *page_id
                {
                    playlist.user_playlists = playlists.clone();
                }

                Some(Task::none())
            }

            Message::UserArtistDetailLoaded(generation, page_id, detail) => {
                let page_id = *page_id;
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !self
                        .ui
                        .playlist_page
                        .current
                        .as_ref()
                        .is_some_and(|playlist| playlist.id == page_id)
                {
                    return Some(Task::none());
                }
                let generation = *generation;
                let total_secs: u64 = detail.top_tracks.iter().map(|s| s.duration_ms / 1000).sum();
                let total_mins = total_secs / 60;
                let total_hours = total_mins / 60;
                let remaining_mins = total_mins % 60;
                let total_duration = if total_hours > 0 {
                    format!("约 {} 小时 {} 分钟", total_hours, remaining_mins)
                } else {
                    format!("{} 分钟", total_mins)
                };

                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == page_id
                {
                    if playlist
                        .description
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                    {
                        playlist.description = Some(format!(
                            "{} 首热门单曲 · {} 张专辑",
                            detail.track_count, detail.album_count
                        ));
                    }
                    if playlist
                        .profile_stats
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                    {
                        playlist.profile_stats = Some(format!(
                            "{} 首热门单曲 · {} 张专辑",
                            detail.track_count, detail.album_count
                        ));
                    }
                    playlist.owner = "热门作品".to_string();
                    playlist.owner_artist_id = Some(detail.id);
                    playlist.song_count = detail.top_tracks.len() as u32;
                    playlist.total_duration = total_duration;
                    playlist.like_count = format!("{} 张专辑", detail.album_count);
                }

                self.ui.playlist_page.replace_online_tracks(
                    page_id,
                    generation,
                    detail.top_tracks.clone(),
                );
                self.ui.home.current_ncm_playlist_songs = detail.top_tracks.clone();
                self.ui.playlist_page.load_state =
                    crate::app::update::page_loader::PlaylistLoadState::Ready;

                let tracks = detail.top_tracks.clone();
                let tracks_task = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::app::update::page_loader::convert_ncm_tracks_to_views(&tracks)
                        })
                        .await
                        .unwrap_or_default()
                    },
                    move |song_views| {
                        Message::NcmPlaylistSongsReady(generation, page_id, song_views)
                    },
                );

                Some(tracks_task)
            }

            Message::NcmPlaylistCreatorDetailLoaded(generation, playlist_id, detail) => {
                if *generation != self.ui.playlist_page.ncm_load_generation
                    || !matches!(self.ui.current_route, Route::NcmPlaylist(id) if -(id as i64) == *playlist_id)
                {
                    return Some(Task::none());
                }
                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == *playlist_id
                    && detail.artist_id != 0
                {
                    playlist.owner_artist_id = Some(detail.artist_id);
                }
                Some(Task::none())
            }

            Message::TogglePlaylistSubscribe(playlist_id) => {
                require_logged_in!(self);

                // Get current subscription status
                let is_subscribed = self
                    .ui
                    .playlist_page
                    .current
                    .as_ref()
                    .map(|p| p.is_subscribed)
                    .unwrap_or(false);

                if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    let playlist_id = *playlist_id;
                    let new_status = !is_subscribed;

                    Some(Task::perform(
                        async move {
                            // NCM playlist IDs are stored as negative in our system
                            let ncm_id = (-playlist_id) as u64;
                            match client.playlist_subscribe(new_status, ncm_id).await {
                                Ok(_) => Some((playlist_id, new_status)),
                                Err(e) => {
                                    error!("Failed to toggle playlist subscription: {}", e);
                                    None
                                }
                            }
                        },
                        |result| {
                            if let Some((id, subscribed)) = result {
                                Message::PlaylistSubscribeChanged(id, subscribed)
                            } else {
                                Message::ShowErrorToast("操作失败".to_string())
                            }
                        },
                    ))
                } else {
                    Some(Task::none())
                }
            }

            Message::PlaylistSubscribeChanged(playlist_id, subscribed) => {
                // Update the subscription status in the current playlist view
                if let Some(playlist) = &mut self.ui.playlist_page.current
                    && playlist.id == *playlist_id
                {
                    playlist.is_subscribed = *subscribed;
                }
                let msg = if *subscribed {
                    "已收藏歌单"
                } else {
                    "已取消收藏"
                };
                Some(Self::toast_success(msg.to_string()))
            }

            Message::AddToNcmPlaylist(song_id, playlist_id) => {
                require_logged_in!(self);
                // If targeting the liked songs playlist (always first in user_playlists),
                // toggle the like status instead of adding to the playlist.
                let is_liked_songs_playlist = self
                    .ui
                    .home
                    .user_playlists
                    .first()
                    .map(|pl| pl.id == *playlist_id)
                    .unwrap_or(false);
                if is_liked_songs_playlist {
                    self.ui.overlay_stack.pop();
                    return Some(Task::done(Message::ToggleFavorite(*song_id)));
                }
                if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    let sid = *song_id;
                    let pid = *playlist_id;
                    return Some(Task::perform(
                        async move {
                            client
                                .mutate_playlist_tracks(
                                    pid,
                                    &[sid],
                                    crate::api::PlaylistTrackOperation::Add,
                                )
                                .await
                        },
                        move |result| match result {
                            Ok(_) => Message::NcmPlaylistAddResult(sid, pid, Ok(())),
                            Err(e) => Message::NcmPlaylistAddResult(sid, pid, Err(e.into())),
                        },
                    ));
                }
                Some(Task::none())
            }

            Message::NcmPlaylistAddResult(_song_id, _playlist_id, result) => {
                self.ui.overlay_stack.pop();
                match result {
                    Ok(()) => {
                        let msg = self.core.locale.get(crate::i18n::Key::SongAddedToPlaylist);
                        Some(Self::toast_success(msg.to_string()))
                    }
                    Err(e) => Some(Self::toast_error(format!(
                        "{}: {}",
                        self.core.locale.get(crate::i18n::Key::SongEditFailed),
                        e
                    ))),
                }
            }

            _ => None,
        }
    }

    /// Load user playlists (liked songs + collected playlists)
    fn load_user_playlists(&self) -> Task<Message> {
        let client = self.core.ncm_client.clone();
        let uid = self.core.user_info.as_ref().map(|u| u.user_id).unwrap_or(0);
        let nickname = self
            .core
            .user_info
            .as_ref()
            .map(|u| u.nickname.clone())
            .unwrap_or_default();

        if uid == 0 {
            return Task::none();
        }

        Task::perform(
            async move {
                if let Some(client) = client {
                    match client.user_playlists(uid, 0, 100).await {
                        Ok(mut playlists) => {
                            // First playlist is "liked songs", rename it
                            if let Some(first) = playlists.first_mut() {
                                first.name = format!("{} 喜欢的音乐", nickname);
                            }
                            playlists
                        }
                        Err(e) => {
                            error!("Failed to load user playlists: {:?}", e);
                            Vec::new()
                        }
                    }
                } else {
                    Vec::new()
                }
            },
            Message::UserPlaylistsLoaded,
        )
    }

    /// Load the redesigned home data.
    pub fn load_discover_data(&mut self) -> Task<Message> {
        self.ui.discover.data_loaded = true;
        self.ui.discover.load_generation = self.ui.discover.load_generation.wrapping_add(1);
        let generation = self.ui.discover.load_generation;

        self.invalidate_daily_recommend_cover();

        let client = self.core.ncm_client.clone();
        let is_logged_in = self.core.is_logged_in;
        self.ui.discover.recommended_loading = is_logged_in;
        self.ui.discover.private_radar_loading = is_logged_in;
        self.ui.discover.hot_loading = true;
        self.ui.discover.official_loading = true;

        let mut tasks = Vec::new();

        if is_logged_in {
            tasks.push(Task::perform(
                {
                    let client = client.clone();
                    async move {
                        if let Some(client) = client {
                            match client.recommend_playlists().await {
                                Ok(playlists) => playlists,
                                Err(e) => {
                                    error!("Failed to load recommended playlists: {:?}", e);
                                    Vec::new()
                                }
                            }
                        } else {
                            Vec::new()
                        }
                    }
                },
                move |playlists| Message::RecommendedPlaylistsLoaded(generation, playlists),
            ));

            tasks.push(Task::perform(
                {
                    let client = client.clone();
                    async move {
                        let client = client?;
                        match client.recommend_tracks().await {
                            Ok(tracks) => tracks.into_iter().next(),
                            Err(e) => {
                                error!("Failed to load Daily Recommend preview: {:?}", e);
                                None
                            }
                        }
                    }
                },
                move |track| Message::DailyRecommendPreviewLoaded(generation, track),
            ));

            tasks.push(Task::perform(
                {
                    let client = client.clone();
                    async move {
                        let Some(client) = client else {
                            return Vec::new();
                        };
                        match client.personal_fm_tracks().await {
                            Ok(tracks) => tracks,
                            Err(e) => {
                                error!("Failed to load Personal FM preview: {:?}", e);
                                Vec::new()
                            }
                        }
                    }
                },
                move |tracks| Message::PersonalFmPreviewLoaded(generation, tracks),
            ));

            tasks.push(Task::perform(
                {
                    let client = client.clone();
                    async move {
                        let client = client?;
                        match client
                            .playlist_detail_preview(PRIVATE_RADAR_PLAYLIST_ID)
                            .await
                        {
                            Ok((detail, _)) => Some(PlaylistSummary {
                                id: detail.id,
                                name: detail.name,
                                cover_url: detail.cover_url,
                                creator: detail.creator,
                                subscribed: detail.subscribed,
                            }),
                            Err(e) => {
                                error!("Failed to load Private Radar metadata: {:?}", e);
                                None
                            }
                        }
                    }
                },
                move |playlist| Message::PrivateRadarLoaded(generation, playlist),
            ));
        } else {
            self.ui.discover.recommended_playlists.clear();
            self.ui.discover.daily_recommend_preview = None;
            self.ui.discover.personal_fm_preview = None;
            self.ui.discover.personal_fm_prefetched_tracks.clear();
            self.ui.discover.private_radar = None;
        }

        tasks.push(Task::perform(
            {
                let client = client.clone();
                async move {
                    if let Some(client) = client {
                        match client.hot_playlists("全部", 0, 30).await {
                            Ok(playlists) => playlists,
                            Err(e) => {
                                error!("Failed to load hot playlists: {:?}", e);
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    }
                }
            },
            move |playlists| Message::HotPlaylistsLoaded(generation, playlists),
        ));

        tasks.push(Task::perform(
            {
                let client = client.clone();
                async move {
                    if let Some(client) = client {
                        match client.high_quality_playlists("全部", None, 30).await {
                            Ok(playlists) => playlists,
                            Err(e) => {
                                error!("Failed to load official playlists: {:?}", e);
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    }
                }
            },
            move |playlists| Message::OfficialPlaylistsLoaded(generation, playlists),
        ));

        Task::batch(tasks)
    }

    /// Daily Recommend is backed by a synthetic playlist ID (`0`), but its
    /// cover is derived from the current first recommendation and can change
    /// on every refresh. Never reuse the stable playlist cache for it.
    fn invalidate_daily_recommend_cover(&mut self) {
        let kind = crate::image::ImageKind::PlaylistCover;
        let id = 0;
        self.ui.image_state.invalidate(kind, id);
        crate::utils::remove_cached_image(&kind.cache_dir(), &kind.file_stem(id));
    }
}

#[cfg(test)]
mod tests {
    use super::exact_artist_id;
    use crate::api::ArtistSummary;

    #[test]
    fn artist_name_resolution_prefers_an_exact_match() {
        let artists = vec![
            ArtistSummary {
                id: 1,
                name: "Artist A Tribute".to_string(),
                image_url: String::new(),
            },
            ArtistSummary {
                id: 2,
                name: "Artist A".to_string(),
                image_url: String::new(),
            },
        ];

        assert_eq!(exact_artist_id(" Artist A ", &artists), Some(2));
        assert_eq!(exact_artist_id("Artist", &artists), None);
    }
}
