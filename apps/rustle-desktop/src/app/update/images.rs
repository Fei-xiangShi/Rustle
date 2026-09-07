//! Unified image pipeline handler.
//!
//! Queues remote image requests, runs a bounded number of downloads, then stores
//! the resulting handle in `ImageState` on `ImageDownloadReady`.

use iced::Task;

use crate::app::state::{ImageRequest, ImageRequestScope};
use crate::app::{App, Message, Route};
use crate::image::{ImageKind, ImageResult};

const MAX_IMAGE_DOWNLOADS: usize = 6;

impl App {
    // ── Main handler ──

    /// Handle unified image-pipeline messages.
    pub fn handle_image(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::ImageDownloadReady(generation, scope, kind, id, path) => {
                if !self
                    .ui
                    .image_state
                    .is_current_inflight(*kind, *id, *generation, *scope)
                {
                    return Some(Task::none());
                }
                self.store_image_handle(*kind, *id, path.clone());
                Some(Task::batch([
                    self.after_image_ready(*kind, *id, path),
                    self.pump_image_downloads(),
                ]))
            }

            Message::ImageDownloadFailed(generation, scope, kind, id) => {
                if !self
                    .ui
                    .image_state
                    .is_current_inflight(*kind, *id, *generation, *scope)
                {
                    return Some(Task::none());
                }
                self.ui.image_state.clear_inflight(*kind, *id);
                Some(self.pump_image_downloads())
            }

            Message::ImageViewportChanged(generation, images) => {
                if *generation != self.ui.image_state.generation {
                    return Some(Task::none());
                }
                let desired = images
                    .iter()
                    .map(|(kind, id, _)| (*kind, *id))
                    .collect::<std::collections::HashSet<_>>();
                self.ui.image_state.reconcile_viewport_requests(&desired);
                Some(Task::none())
            }

            _ => None,
        }
    }

    /// Schedule all image work implied by the handled message and current app state.
    pub(super) fn collect_image_tasks_after_message(&mut self, message: &Message) -> Task<Message> {
        if matches!(
            message,
            Message::ImageDownloadReady(..) | Message::ImageDownloadFailed(..)
        ) {
            return Task::none();
        }

        Task::batch([
            self.collect_image_tasks_for_message(message),
            self.collect_current_song_image_task(),
        ])
    }

    /// Collect image references carried by a business message.
    fn collect_image_tasks_for_message(&mut self, message: &Message) -> Task<Message> {
        let mut refs = Vec::new();

        match message {
            Message::AutoLoginResult(Some(login_info), _) | Message::LoginSuccess(login_info) => {
                refs.push(RemoteImage::global(
                    ImageKind::UserAvatar,
                    login_info.user_id,
                    &login_info.avatar_url,
                ));
                if let Some(icon_url) = login_info.vip.badge_url() {
                    refs.push(RemoteImage::global(
                        ImageKind::VipBadge,
                        crate::image::vip_badge_key(
                            login_info.user_id,
                            login_info.vip.tier(),
                            icon_url,
                        ),
                        icon_url,
                    ));
                }
            }
            Message::UserPlaylistsLoaded(playlists)
                if matches!(self.ui.current_route, Route::Discover(_) | Route::Radio) =>
            {
                refs.extend(remote_playlist_covers(playlists));
            }
            Message::RecommendedPlaylistsLoaded(generation, playlists)
                if *generation == self.ui.discover.load_generation
                    && matches!(self.ui.current_route, Route::Discover(_) | Route::Radio) =>
            {
                refs.extend(remote_playlist_covers(playlists));
            }
            Message::DailyRecommendPreviewLoaded(generation, Some(track))
                if *generation == self.ui.discover.load_generation
                    && matches!(self.ui.current_route, Route::Discover(_) | Route::Radio) =>
            {
                refs.extend(remote_track_covers(std::slice::from_ref(track)));
            }
            Message::PersonalFmPreviewLoaded(generation, tracks)
                if *generation == self.ui.discover.load_generation
                    && matches!(self.ui.current_route, Route::Discover(_) | Route::Radio) =>
            {
                if let Some(track) = tracks.first() {
                    refs.extend(remote_track_covers(std::slice::from_ref(track)));
                }
            }
            Message::HotPlaylistsLoaded(generation, playlists)
                if *generation == self.ui.discover.load_generation
                    && matches!(self.ui.current_route, Route::Discover(_) | Route::Radio) =>
            {
                refs.extend(remote_playlist_covers(playlists));
            }
            Message::OfficialPlaylistsLoaded(generation, playlists)
                if *generation == self.ui.discover.load_generation
                    && matches!(self.ui.current_route, Route::Discover(_) | Route::Radio) =>
            {
                refs.extend(remote_playlist_covers(playlists));
            }
            Message::PrivateRadarLoaded(generation, Some(playlist))
                if *generation == self.ui.discover.load_generation
                    && matches!(self.ui.current_route, Route::Discover(_) | Route::Radio) =>
            {
                refs.extend(remote_playlist_covers(std::slice::from_ref(playlist)));
            }
            Message::AddNcmPlaylist(songs, _) | Message::AddNcmPlaylistWithSource(songs, _, _) => {
                refs.extend(remote_track_covers(songs));
            }
            Message::PlayNcmSong(song) => {
                refs.push(RemoteImage::global(
                    ImageKind::SongCover,
                    song.id,
                    song.cover_url(),
                ));
            }
            Message::SearchResultsLoaded(payload)
                if self.ui.search.keyword == payload.context.keyword
                    && self.ui.search.active_tab == payload.context.tab
                    && self.ui.search.current_page == payload.context.page =>
            {
                match payload.context.tab {
                    crate::app::state::SearchTab::Songs => {
                        // Song results are rendered by a virtual list; their
                        // covers are requested from the visible range callback.
                    }
                    crate::app::state::SearchTab::Albums => {
                        refs.extend(remote_album_covers(&payload.albums));
                    }
                    crate::app::state::SearchTab::Artists => {
                        refs.extend(remote_artist_covers(&payload.artists));
                    }
                    crate::app::state::SearchTab::Playlists => {
                        refs.extend(remote_playlist_covers(&payload.playlists));
                    }
                    crate::app::state::SearchTab::Videos => {
                        refs.extend(remote_video_covers(&payload.videos));
                    }
                    crate::app::state::SearchTab::Radios => {
                        refs.extend(remote_radio_covers(&payload.radios));
                    }
                }
            }
            Message::NcmPlaylistDetailLoaded(generation, detail)
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && matches!(self.ui.current_route, Route::NcmPlaylist(id) if id == detail.id) =>
            {
                refs.push(RemoteImage::new(
                    ImageKind::PlaylistCover,
                    detail.id,
                    &detail.cover_url,
                ));
                if detail.creator.id != 0 {
                    refs.push(RemoteImage::new(
                        ImageKind::UserAvatar,
                        detail.creator.id,
                        &detail.creator.avatar_url,
                    ));
                }
            }
            Message::NcmPlaylistCacheLoaded(generation, playlist_id, Some(detail))
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && matches!(self.ui.current_route, Route::NcmPlaylist(id) if id == *playlist_id) =>
            {
                refs.push(RemoteImage::new(
                    ImageKind::PlaylistCover,
                    detail.id,
                    &detail.cover_url,
                ));
                if detail.creator.id != 0 {
                    refs.push(RemoteImage::new(
                        ImageKind::UserAvatar,
                        detail.creator.id,
                        &detail.creator.avatar_url,
                    ));
                }
            }
            Message::NcmPlaylistPreviewLoaded(generation, detail, _)
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && matches!(self.ui.current_route, Route::NcmPlaylist(id) if id == detail.id) =>
            {
                refs.push(RemoteImage::new(
                    ImageKind::PlaylistCover,
                    detail.id,
                    &detail.cover_url,
                ));
                if detail.creator.id != 0 {
                    refs.push(RemoteImage::new(
                        ImageKind::UserAvatar,
                        detail.creator.id,
                        &detail.creator.avatar_url,
                    ));
                }
            }
            Message::AlbumDetailLoaded(detail) if matches!(self.ui.current_route, Route::Album(id) if id == detail.id) =>
            {
                refs.push(RemoteImage::new(
                    ImageKind::AlbumCover,
                    detail.id,
                    &detail.image_url,
                ));
                if let Some(artist) = detail.primary_artist() {
                    refs.push(RemoteImage::new(
                        ImageKind::ArtistCover,
                        artist.id,
                        &artist.image_url,
                    ));
                }
            }
            Message::ArtistDetailLoaded(detail) if matches!(self.ui.current_route, Route::Artist(id) if id == detail.id) =>
            {
                refs.push(RemoteImage::new(
                    ImageKind::ArtistCover,
                    detail.id,
                    &detail.image_url,
                ));
            }
            Message::ArtistAlbumsLoaded(page_id, albums)
                if artist_route_matches_page_id(&self.ui.current_route, *page_id) =>
            {
                refs.extend(remote_album_covers(albums));
            }
            Message::UserPageDetailLoaded(page_id, detail)
                if user_route_matches_page_id(&self.ui.current_route, *page_id) =>
            {
                refs.push(RemoteImage::new(
                    ImageKind::UserAvatar,
                    detail.user_id,
                    &detail.avatar_url,
                ));
                if detail.artist_id != 0 {
                    refs.push(RemoteImage::new(
                        ImageKind::ArtistCover,
                        detail.artist_id,
                        &detail.background_url,
                    ));
                }
            }
            Message::UserPagePlaylistsLoaded(page_id, playlists)
                if user_route_matches_page_id(&self.ui.current_route, *page_id) =>
            {
                refs.extend(remote_playlist_covers(playlists));
            }
            Message::DownloadBatchEnqueue(items) => {
                refs.extend(items.iter().filter_map(|(_, ncm_id, _, metadata)| {
                    if let Some(crate::metadata::CoverSource::Url(url)) = &metadata.cover {
                        Some(RemoteImage::global(ImageKind::SongCover, *ncm_id, url))
                    } else {
                        None
                    }
                }));
            }
            Message::DownloadUrlResolved(_, ncm_id, _, metadata) => {
                if let Some(crate::metadata::CoverSource::Url(url)) = &metadata.cover {
                    refs.push(RemoteImage::global(ImageKind::SongCover, *ncm_id, url));
                }
            }
            Message::ImageViewportChanged(generation, images)
                if *generation == self.ui.image_state.generation =>
            {
                refs.extend(
                    images
                        .iter()
                        .map(|(kind, id, url)| RemoteImage::viewport(*kind, *id, url)),
                );
            }
            _ => {}
        }

        let mut tasks = refs
            .into_iter()
            .map(|image| {
                self.enqueue_image_download_scoped(image.kind, image.id, &image.url, image.scope)
            })
            .collect::<Vec<_>>();
        tasks.push(self.pump_image_downloads());

        Task::batch(tasks)
    }

    fn collect_current_song_image_task(&mut self) -> Task<Message> {
        let Some(song) = self.playback.current_song.clone() else {
            return Task::none();
        };
        let Some((kind, id)) = crate::image::song_cover_key_for_source(song.id, &song.file_path)
        else {
            return Task::none();
        };

        if let Some(local_path) = self.resolved_song_cover_local_path(&song) {
            if self.ui.image_state.get(kind, id).is_none() {
                self.store_image_path(kind, id, local_path.clone());
            }

            if !cover_path_matches(song.cover_path.as_deref(), &local_path) {
                return self.after_image_ready(kind, id, &local_path);
            }

            return Task::none();
        }

        if let Some(path_or_url) = song.cover_path.as_deref()
            && crate::image::is_remote_url(path_or_url)
        {
            let enqueue_task = self.enqueue_image_download_scoped(
                kind,
                id,
                path_or_url,
                ImageRequestScope::Global,
            );
            return Task::batch([enqueue_task, self.pump_image_downloads()]);
        }

        self.register_cached_image(kind, id)
            .unwrap_or_else(Task::none)
    }

    /// Re-register discover covers after a route transition cancelled page-scoped work.
    pub(super) fn collect_discover_image_tasks(&mut self) -> Task<Message> {
        let mut refs =
            remote_playlist_covers(&self.ui.discover.recommended_playlists).collect::<Vec<_>>();
        refs.extend(remote_playlist_covers(&self.ui.discover.hot_playlists));
        refs.extend(remote_playlist_covers(&self.ui.discover.official_playlists));
        if let Some(playlist) = &self.ui.discover.private_radar {
            refs.extend(remote_playlist_covers(std::slice::from_ref(playlist)));
        }
        if let Some(track) = &self.ui.discover.daily_recommend_preview {
            refs.extend(remote_track_covers(std::slice::from_ref(track)));
        }
        if let Some(track) = &self.ui.discover.personal_fm_preview {
            refs.extend(remote_track_covers(std::slice::from_ref(track)));
        }

        let mut tasks = refs
            .into_iter()
            .map(|image| {
                self.enqueue_image_download_scoped(image.kind, image.id, &image.url, image.scope)
            })
            .collect::<Vec<_>>();
        tasks.push(self.pump_image_downloads());
        Task::batch(tasks)
    }

    /// Return the already available local cover file for a song, using the
    /// unified image state first and the on-disk image cache second.
    pub(super) fn cached_song_cover_local_path(
        &self,
        song: &crate::database::DbSong,
    ) -> Option<std::path::PathBuf> {
        let (kind, id) = crate::image::song_cover_key_for_source(song.id, &song.file_path)?;

        if let Some((path, _, _)) = self.ui.image_state.image_data(kind, id)
            && path.exists()
        {
            return Some(path.clone());
        }

        crate::image::resolve_cached(kind, id)
    }

    /// Resolve the local cover file for a song without starting network work.
    pub(super) fn resolved_song_cover_local_path(
        &self,
        song: &crate::database::DbSong,
    ) -> Option<std::path::PathBuf> {
        if let Some(path) = song.cover_path.as_deref()
            && crate::image::is_valid_local_path(path)
        {
            return Some(std::path::PathBuf::from(path));
        }

        self.cached_song_cover_local_path(song)
    }

    // ── Internal ──

    /// If the given `(kind, id)` is already in memory or on disk, populate
    /// `ImageState` immediately. Otherwise queue it for the bounded downloader.
    fn enqueue_image_download_scoped(
        &mut self,
        kind: ImageKind,
        id: u64,
        url: &str,
        scope: ImageRequestScope,
    ) -> Task<Message> {
        if url.is_empty() {
            return Task::none();
        }
        if self.ui.image_state.get(kind, id).is_some() {
            // The image may have been loaded while the user was on another
            // page (for example, from a playlist grid). Keep the current
            // detail page in sync as well; otherwise its palette remains the
            // default even though the cover is already available.
            if let Some(path) = self
                .ui
                .image_state
                .image_data(kind, id)
                .map(|(path, _, _)| path.clone())
            {
                self.sync_loaded_image_to_current_page(kind, id, &path);
            }
            return Task::none();
        }
        if let Some(task) = self.register_cached_image(kind, id) {
            return task;
        }
        if self.core.ncm_client.is_none() {
            return Task::none();
        }

        self.ui
            .image_state
            .enqueue_with_scope(kind, id, url.to_string(), scope);
        Task::none()
    }

    pub(super) fn pump_image_downloads(&mut self) -> Task<Message> {
        let Some(client) = self.core.ncm_client.as_ref().cloned() else {
            return Task::none();
        };

        let mut tasks = Vec::new();

        while self.ui.image_state.inflight.len() < MAX_IMAGE_DOWNLOADS {
            let Some(request) = self.ui.image_state.pop_pending() else {
                break;
            };

            if self.ui.image_state.get(request.kind, request.id).is_some()
                || self.ui.image_state.is_inflight(request.kind, request.id)
            {
                continue;
            }

            if let Some(path) = crate::image::resolve_cached(request.kind, request.id) {
                self.store_image_handle(request.kind, request.id, path.clone());
                tasks.push(self.after_image_ready(request.kind, request.id, &path));
                continue;
            }

            let (task, handle) = start_image_download(client.clone(), request.clone());
            self.ui.image_state.mark_inflight(
                request.kind,
                request.id,
                request.generation,
                request.scope,
                handle,
            );
            tasks.push(task);
        }

        Task::batch(tasks)
    }

    fn register_cached_image(&mut self, kind: ImageKind, id: u64) -> Option<Task<Message>> {
        if self.ui.image_state.get(kind, id).is_some() {
            return None;
        }
        let path = crate::image::resolve_cached(kind, id)?;
        self.store_image_handle(kind, id, path.clone());
        Some(self.after_image_ready(kind, id, &path))
    }

    fn store_image_handle(&mut self, kind: ImageKind, id: u64, path: std::path::PathBuf) {
        self.ui.image_state.insert_path(kind, id, path.clone());
        self.sync_loaded_image_to_current_page(kind, id, &path);
    }

    fn after_image_ready(
        &mut self,
        kind: ImageKind,
        id: u64,
        path: &std::path::Path,
    ) -> Task<Message> {
        if !matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover) {
            return Task::none();
        }
        let key = (kind, id);

        let path_string = path.to_string_lossy().to_string();

        if let Some(current) = &mut self.playback.current_song
            && crate::image::song_cover_key_for_source(current.id, &current.file_path) == Some(key)
        {
            current.cover_path = Some(path_string.clone());
        }

        let mut matched_song = None;
        if let Some(idx) = self.playback.current_index
            && let Some(queue_song) = self.playback.queue.get_mut(idx)
            && crate::image::song_cover_key_for_source(queue_song.id, &queue_song.file_path)
                == Some(key)
        {
            queue_song.cover_path = Some(path_string.clone());
            matched_song = Some(queue_song.clone());

            if let Some(db) = &self.core.db {
                let db = db.clone();
                match kind {
                    ImageKind::SongCover => {
                        let song_clone = queue_song.clone();
                        tokio::spawn(async move {
                            if let Err(e) = db.upsert_ncm_song(&song_clone).await {
                                tracing::warn!("Failed to update cover path in database: {}", e);
                            }
                        });
                    }
                    ImageKind::LocalSongCover => {
                        let song_id = queue_song.id;
                        let cover_path = path_string.clone();
                        tokio::spawn(async move {
                            if let Err(e) = db.update_song_cover(song_id, &cover_path).await {
                                tracing::warn!(
                                    "Failed to update local cover path in database: {}",
                                    e
                                );
                            }
                        });
                    }
                    _ => {}
                }
            }
        }

        if let Some(song) = self.playback.current_song.clone()
            && crate::image::song_cover_key_for_source(song.id, &song.file_path) == Some(key)
        {
            return self.update_lyrics_background(&song);
        }

        if let Some(song) = matched_song
            && self.ui.lyrics.is_open
            && self
                .playback
                .preload_coordinator
                .window()
                .contains_song(song.id)
        {
            return self
                .prepare_lyrics_background_for_cover_path(song.id, std::path::PathBuf::from(path));
        }

        Task::none()
    }

    pub(crate) fn store_image_path(&mut self, kind: ImageKind, id: u64, path: std::path::PathBuf) {
        if !path.exists() {
            return;
        }
        self.store_image_handle(kind, id, path);
    }

    pub(crate) fn store_image_paths<I>(&mut self, images: I)
    where
        I: IntoIterator<Item = (ImageKind, u64, std::path::PathBuf)>,
    {
        for (kind, id, path) in images {
            self.store_image_path(kind, id, path);
        }
    }

    pub(crate) fn store_db_song_cover_paths(&mut self, songs: &[crate::database::DbSong]) {
        let images = songs.iter().filter_map(|song| {
            let path = song.cover_path.as_deref()?;
            if !crate::image::is_valid_local_path(path) {
                return None;
            }
            let (kind, id) = crate::image::song_cover_key_for_source(song.id, &song.file_path)?;
            Some((kind, id, std::path::PathBuf::from(path)))
        });
        self.store_image_paths(images);
    }

    pub(crate) fn store_local_playlist_cover_paths(
        &mut self,
        playlists: &[crate::database::DbPlaylist],
    ) {
        let images = playlists.iter().filter_map(|playlist| {
            let id = u64::try_from(playlist.id).ok()?;
            let path = playlist.cover_path.as_deref()?;
            if !crate::image::is_valid_local_path(path) {
                return None;
            }
            Some((
                ImageKind::LocalPlaylistCover,
                id,
                std::path::PathBuf::from(path),
            ))
        });
        self.store_image_paths(images);
    }

    fn sync_loaded_image_to_current_page(
        &mut self,
        kind: ImageKind,
        id: u64,
        path: &std::path::Path,
    ) {
        let Some(page) = self.ui.playlist_page.current.as_mut() else {
            return;
        };

        let path_string = path.to_string_lossy().to_string();
        match kind {
            ImageKind::PlaylistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Playlist
                    && page.id == ncm_playlist_page_id(id) =>
            {
                set_detail_cover(page, path, path_string);
            }
            ImageKind::LocalPlaylistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Playlist
                    && page.id == id as i64 =>
            {
                set_detail_cover(page, path, path_string);
            }
            ImageKind::AlbumCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Album
                    && page.id == album_page_id(id) =>
            {
                set_detail_cover(page, path, path_string);
            }
            ImageKind::ArtistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Artist
                    && page.id == artist_page_id(id) =>
            {
                set_detail_cover(page, path, path_string);
            }
            ImageKind::ArtistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::User
                    && page.owner_artist_id == Some(id) =>
            {
                set_detail_cover(page, path, path_string);
            }
            ImageKind::ArtistCover if page.owner_artist_id == Some(id) => {
                page.owner_avatar_path = Some(path_string);
            }
            ImageKind::UserAvatar if page.creator_id == id => {
                if page.kind == crate::ui::pages::playlist::DetailPageKind::User
                    && page.cover_path.is_none()
                {
                    set_detail_cover(page, path, path_string);
                } else {
                    page.owner_avatar_path = Some(path_string);
                }
            }
            _ => {}
        }
    }
}

fn start_image_download(
    client: crate::api::NcmClient,
    request: ImageRequest,
) -> (Task<Message>, iced::task::Handle) {
    let ImageRequest {
        kind,
        id,
        url,
        generation,
        scope,
    } = request;
    let resize = match kind {
        // Membership artwork is already a compact horizontal badge. Asking
        // the CDN for a square derivative changes its composition and makes
        // the visible mark look much smaller inside a contain-fit widget.
        ImageKind::VipBadge => None,
        ImageKind::SongCover | ImageKind::LocalSongCover | ImageKind::UserAvatar => {
            Some((200, 200))
        }
        _ => Some((300, 300)),
    };

    Task::perform(
        async move {
            let base_path = kind.cache_dir().join(format!("{}.jpg", kind.file_stem(id)));
            crate::utils::download_img(&client, &url, base_path, resize)
                .await
                .map(|path| ImageResult { kind, id, path })
        },
        move |result| match result {
            Some(r) => Message::ImageDownloadReady(generation, scope, r.kind, r.id, r.path),
            None => Message::ImageDownloadFailed(generation, scope, kind, id),
        },
    )
    .abortable()
}

struct RemoteImage {
    kind: ImageKind,
    id: u64,
    url: String,
    scope: ImageRequestScope,
}

impl RemoteImage {
    fn new(kind: ImageKind, id: u64, url: &str) -> Self {
        Self {
            kind,
            id,
            url: url.to_string(),
            scope: ImageRequestScope::Page,
        }
    }

    fn global(kind: ImageKind, id: u64, url: &str) -> Self {
        Self {
            kind,
            id,
            url: url.to_string(),
            scope: ImageRequestScope::Global,
        }
    }

    fn viewport(kind: ImageKind, id: u64, url: &str) -> Self {
        Self {
            kind,
            id,
            url: url.to_string(),
            scope: ImageRequestScope::Viewport,
        }
    }
}

fn remote_track_covers(tracks: &[crate::api::Track]) -> impl Iterator<Item = RemoteImage> + '_ {
    tracks
        .iter()
        .map(|track| RemoteImage::new(ImageKind::SongCover, track.id, track.cover_url()))
}

fn remote_playlist_covers(
    playlists: &[crate::api::PlaylistSummary],
) -> impl Iterator<Item = RemoteImage> + '_ {
    playlists
        .iter()
        .map(|item| RemoteImage::new(ImageKind::PlaylistCover, item.id, &item.cover_url))
}

fn remote_album_covers(
    albums: &[crate::api::AlbumSummary],
) -> impl Iterator<Item = RemoteImage> + '_ {
    albums
        .iter()
        .map(|item| RemoteImage::new(ImageKind::AlbumCover, item.id, &item.image_url))
}

fn remote_artist_covers(
    artists: &[crate::api::ArtistSummary],
) -> impl Iterator<Item = RemoteImage> + '_ {
    artists
        .iter()
        .map(|item| RemoteImage::new(ImageKind::ArtistCover, item.id, &item.image_url))
}

fn remote_video_covers(
    videos: &[crate::api::VideoSummary],
) -> impl Iterator<Item = RemoteImage> + '_ {
    videos
        .iter()
        .map(|item| RemoteImage::new(ImageKind::VideoCover, item.id, &item.cover_url))
}

fn remote_radio_covers(
    radios: &[crate::api::RadioSummary],
) -> impl Iterator<Item = RemoteImage> + '_ {
    radios
        .iter()
        .map(|item| RemoteImage::new(ImageKind::RadioCover, item.id, &item.cover_url))
}

fn cover_path_matches(path_or_url: Option<&str>, local_path: &std::path::Path) -> bool {
    path_or_url.is_some_and(|path| {
        crate::image::is_valid_local_path(path) && std::path::Path::new(path) == local_path
    })
}

fn ncm_playlist_page_id(id: u64) -> i64 {
    -(id as i64)
}

fn artist_page_id(id: u64) -> i64 {
    i64::MIN + id as i64
}

fn artist_route_matches_page_id(route: &Route, page_id: i64) -> bool {
    matches!(route, Route::Artist(id) if artist_page_id(*id) == page_id)
}

fn album_page_id(id: u64) -> i64 {
    (i64::MIN / 4) + id as i64
}

fn user_page_id(id: u64) -> i64 {
    (i64::MIN / 2) + id as i64
}

fn user_route_matches_page_id(route: &Route, page_id: i64) -> bool {
    matches!(route, Route::User(id) if user_page_id(*id) == page_id)
}

fn set_detail_cover(
    page: &mut crate::ui::pages::PlaylistView,
    path: &std::path::Path,
    path_string: String,
) {
    let palette = crate::utils::ColorPalette::from_image_path(path);
    page.cover_path = Some(path_string);
    page.palette = palette;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_route_matches_encoded_page_id() {
        let user_id = 123_456_789;
        let page_id = user_page_id(user_id);

        assert!(page_id < 0);
        assert!(user_route_matches_page_id(&Route::User(user_id), page_id));
        assert!(!user_route_matches_page_id(
            &Route::User(user_id + 1),
            page_id
        ));
        assert!(!user_route_matches_page_id(
            &Route::Artist(user_id),
            page_id
        ));
    }

    #[test]
    fn artist_route_matches_encoded_page_id() {
        let artist_id = 123_456_789;
        let page_id = artist_page_id(artist_id);

        assert!(page_id < 0);
        assert!(artist_route_matches_page_id(
            &Route::Artist(artist_id),
            page_id
        ));
        assert!(!artist_route_matches_page_id(
            &Route::Artist(artist_id + 1),
            page_id
        ));
        assert!(!artist_route_matches_page_id(
            &Route::User(artist_id),
            page_id
        ));
    }
}
