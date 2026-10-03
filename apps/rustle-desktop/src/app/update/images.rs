//! Unified image pipeline handler.
//!
//! Queues remote image requests, runs a bounded number of downloads, then stores
//! the resulting handle in `ImageState` on `ImageDownloadReady`.

use iced::Task;

use crate::app::state::{ImageRequest, ImageRequestScope};
use crate::app::{App, Message, Route};
use crate::image::{ImageKind, ImageVariant};

const MAX_IMAGE_DOWNLOADS: usize = 6;

impl App {
    // ── Main handler ──

    /// Handle unified image-pipeline messages.
    pub fn handle_image(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::ImageDownloadReady(generation, scope, kind, id, variant, entry) => {
                let path = &entry.path;
                if !self.ui.image_state.is_current_variant_inflight(
                    *kind,
                    *id,
                    *variant,
                    *generation,
                    *scope,
                ) || self
                    .ui
                    .image_state
                    .inflight
                    .get(&(*kind, *id, *variant))
                    .is_none_or(|request| {
                        entry.source_url.as_deref() != Some(request.source_url.as_str())
                    })
                {
                    return Some(Task::none());
                }
                self.ui
                    .image_state
                    .clear_variant_inflight(*kind, *id, *variant);
                self.ui
                    .image_state
                    .insert_prepared(*kind, *id, *variant, entry.as_ref().clone());
                self.sync_preferred_image_to_current_page(*kind, *id);
                Some(Task::batch([
                    self.after_image_ready(*kind, *id, *variant, path),
                    self.pump_image_downloads(),
                ]))
            }

            Message::ImageDownloadFailed(generation, scope, kind, id, variant) => {
                if !self.ui.image_state.is_current_variant_inflight(
                    *kind,
                    *id,
                    *variant,
                    *generation,
                    *scope,
                ) {
                    return Some(Task::none());
                }
                let replacement_queued = self
                    .ui
                    .image_state
                    .clear_variant_inflight(*kind, *id, *variant);
                if !replacement_queued {
                    self.ui
                        .image_state
                        .failures
                        .insert((*kind, *id, *variant), std::time::Instant::now());
                }
                Some(self.pump_image_downloads())
            }

            Message::CurrentSongImageSourceResolved(song_id, ncm_id, url) => {
                self.ui
                    .image_state
                    .finish_source_resolution(ImageKind::SongCover, *ncm_id);
                let is_current = self.playback.current_song.as_ref().is_some_and(|song| {
                    song.id == *song_id
                        && crate::image::song_cover_key_for_source(song.id, &song.file_path)
                            == Some((ImageKind::SongCover, *ncm_id))
                });
                if !is_current {
                    return Some(Task::none());
                }
                let Some(url) = url.as_deref().filter(|url| !url.is_empty()) else {
                    return Some(Task::none());
                };
                let mut tasks =
                    current_song_remote_variants(ImageKind::SongCover, self.ui.lyrics.is_open)
                        .into_iter()
                        .map(|variant| {
                            self.enqueue_image_variant_download_scoped(
                                ImageKind::SongCover,
                                *ncm_id,
                                variant,
                                url,
                                ImageRequestScope::Global,
                            )
                        })
                        .collect::<Vec<_>>();
                tasks.push(self.pump_image_downloads());
                Some(Task::batch(tasks))
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
            Message::ImageDownloadReady(..)
                | Message::ImageDownloadFailed(..)
                | Message::PlaybackTick
                | Message::AnimationTick(_)
                | Message::MouseMoved(_)
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
            Message::PlaylistViewLoaded(_) | Message::RecentlyPlayedLoaded(_) => {
                if let Some(page) = self
                    .ui
                    .playlist_page
                    .current
                    .as_ref()
                    .filter(|page| page.is_local)
                {
                    if let (Ok(id), Some(source)) =
                        (u64::try_from(page.id), page.cover_path.as_deref())
                    {
                        refs.push(RemoteImage::detail(
                            ImageKind::LocalPlaylistCover,
                            id,
                            source,
                        ));
                    } else if let Some((kind, id, source)) = page.songs.iter().find_map(|song| {
                        let (kind, id) = song.cover_key?;
                        Some((kind, id, song.cover_url.as_deref()?))
                    }) {
                        refs.push(RemoteImage::detail(kind, id, source));
                    }
                }
                self.sync_local_header_cover();
            }
            Message::AutoLoginResult(Ok(login_info), _) | Message::LoginSuccess(login_info) => {
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
            // Enqueuing a playlist must not decode every song cover: visible
            // rows and the current song independently request what they need.
            Message::PlayNcmSong(song) => {
                refs.push(RemoteImage::global(
                    ImageKind::SongCover,
                    song.id,
                    song.cover_url(),
                ));
            }
            Message::SearchResultsLoaded(payload)
                if self.search_request_is_current(&payload.context) =>
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
            Message::RadioDetailLoaded(generation, id, Ok(detail))
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && self.ui.current_route == Route::Podcast(*id)
                    && detail.radio.id == *id =>
            {
                refs.push(RemoteImage::detail(
                    ImageKind::RadioCover,
                    *id,
                    &detail.radio.cover_url,
                ));
                if detail.radio.creator.id != 0 {
                    refs.push(RemoteImage::new(
                        ImageKind::UserAvatar,
                        detail.radio.creator.id,
                        &detail.radio.creator.avatar_url,
                    ));
                }
            }
            Message::NcmPlaylistDetailLoaded(generation, detail)
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && matches!(self.ui.current_route, Route::NcmPlaylist(id) if id == detail.id) =>
            {
                refs.push(RemoteImage::detail(
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
                refs.push(RemoteImage::detail(
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
                refs.push(RemoteImage::detail(
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
            Message::AlbumDetailLoaded(generation, detail)
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && matches!(self.ui.current_route, Route::Album(id) if id == detail.id) =>
            {
                refs.push(RemoteImage::detail(
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
            Message::ArtistDetailLoaded(generation, detail)
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && matches!(self.ui.current_route, Route::Artist(id) if id == detail.id) =>
            {
                refs.push(RemoteImage::detail(
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
            Message::UserPageDetailLoaded(generation, page_id, detail)
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && user_route_matches_page_id(&self.ui.current_route, *page_id) =>
            {
                refs.push(RemoteImage::detail(
                    ImageKind::UserAvatar,
                    detail.user_id,
                    &detail.avatar_url,
                ));
                if detail.artist_id != 0 {
                    refs.push(RemoteImage::detail(
                        ImageKind::ArtistCover,
                        detail.artist_id,
                        &detail.background_url,
                    ));
                }
            }
            Message::UserArtistDetailLoaded(generation, page_id, detail)
                if *generation == self.ui.playlist_page.ncm_load_generation
                    && user_route_matches_page_id(&self.ui.current_route, *page_id) =>
            {
                refs.push(RemoteImage::detail(
                    ImageKind::ArtistCover,
                    detail.id,
                    &detail.image_url,
                ));
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
                self.enqueue_image_variant_download_scoped(
                    image.kind,
                    image.id,
                    image.variant,
                    &image.url,
                    image.scope,
                )
            })
            .collect::<Vec<_>>();
        tasks.push(self.pump_image_downloads());

        Task::batch(tasks)
    }

    fn collect_current_song_image_task(&mut self) -> Task<Message> {
        if self.library.audio_index.running
            && crate::utils::audio_index::snapshot()
                .cache_dir
                .as_os_str()
                .is_empty()
        {
            return Task::none();
        }
        let Some(song) = self.playback.current_song.clone() else {
            return Task::none();
        };
        let Some((kind, id)) = crate::image::song_cover_key_for_source(song.id, &song.file_path)
        else {
            return Task::none();
        };

        let source = if kind == ImageKind::LocalSongCover {
            Some(song.file_path.clone())
        } else {
            song.cover_path
                .as_deref()
                .filter(|p| crate::image::is_remote_url(p))
                .map(str::to_owned)
                .or_else(|| self.ui.image_state.known_sources.get(&(kind, id)).cloned())
                .or_else(|| {
                    self.current_ncm_track(id)
                        .map(|track| track.cover_url().to_owned())
                })
                .or_else(|| {
                    crate::utils::audio_index::snapshot()
                        .locate(
                            &song.file_path,
                            song.id,
                            Some(&song.artist),
                            Some(&song.title),
                        )
                        .map(|_| format!("ncm://{id}"))
                })
        };
        if let Some(source) = source {
            let mut tasks = Vec::new();
            for variant in
                current_song_remote_variants(ImageKind::SongCover, self.ui.lyrics.is_open)
            {
                tasks.push(self.enqueue_image_variant_download_scoped(
                    kind,
                    id,
                    variant,
                    &source,
                    ImageRequestScope::Global,
                ));
            }
            tasks.push(self.pump_image_downloads());
            return Task::batch(tasks);
        }
        if kind == ImageKind::SongCover
            && let Some(client) = self.core.ncm_client.as_ref().cloned()
            && self.ui.image_state.begin_source_resolution(kind, id)
        {
            let song_id = song.id;
            return Task::perform(
                async move {
                    client.track_detail(&[id]).await.ok().and_then(|tracks| {
                        tracks
                            .into_iter()
                            .next()
                            .map(|track| track.cover_url().to_owned())
                    })
                },
                move |url| Message::CurrentSongImageSourceResolved(song_id, id, url),
            );
        }
        Task::none()
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
                self.enqueue_image_variant_download_scoped(
                    image.kind,
                    image.id,
                    image.variant,
                    &image.url,
                    image.scope,
                )
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

        self.ui
            .image_state
            .image_data_variant(kind, id, ImageVariant::Hero)
            .or_else(|| self.ui.image_state.image_data(kind, id))
            .map(|(path, _, _)| path.clone())
    }

    pub(super) fn resolved_song_cover_local_path(
        &self,
        song: &crate::database::DbSong,
    ) -> Option<std::path::PathBuf> {
        self.cached_song_cover_local_path(song)
    }

    // ── Internal ──

    /// Populate an exact role/source derivative from memory or disk, retain a
    /// legacy Thumbnail as fallback, and otherwise queue bounded network work.
    fn enqueue_image_variant_download_scoped(
        &mut self,
        kind: ImageKind,
        id: u64,
        variant: ImageVariant,
        url: &str,
        scope: ImageRequestScope,
    ) -> Task<Message> {
        let fallback;
        let url = if url.is_empty() && kind == ImageKind::SongCover {
            fallback = format!("ncm://{id}");
            fallback.as_str()
        } else {
            url
        };
        if url.is_empty() {
            return Task::none();
        }

        if self
            .ui
            .image_state
            .has_current_remote_variant(kind, id, variant, url)
        {
            let key = (kind, id, variant);
            if self.ui.image_state.song_lru.contains(&key) {
                self.ui
                    .image_state
                    .song_lru
                    .retain(|candidate| *candidate != key);
                self.ui.image_state.song_lru.push_back(key);
            }
            self.sync_preferred_image_to_current_page(kind, id);
            return Task::none();
        }

        self.ui
            .image_state
            .enqueue_variant_with_scope(kind, id, variant, url.to_string(), scope);
        Task::none()
    }

    pub(super) fn pump_image_downloads(&mut self) -> Task<Message> {
        let client = self.core.ncm_client.clone();
        let mut tasks = Vec::new();
        while self.ui.image_state.inflight.len() < MAX_IMAGE_DOWNLOADS {
            let Some(request) = self.ui.image_state.pop_pending() else {
                break;
            };
            if matches!(
                request.kind,
                ImageKind::SongCover | ImageKind::LocalSongCover
            ) && self.library.audio_index.running
                && crate::utils::audio_index::snapshot()
                    .cache_dir
                    .as_os_str()
                    .is_empty()
            {
                self.ui
                    .image_state
                    .queued
                    .insert((request.kind, request.id, request.variant));
                self.ui.image_state.pending.push_front(request);
                break;
            }
            if self.ui.image_state.has_current_remote_variant(
                request.kind,
                request.id,
                request.variant,
                &request.url,
            ) || self.ui.image_state.is_variant_inflight(
                request.kind,
                request.id,
                request.variant,
            ) {
                continue;
            }
            let local_audio = if request.kind == ImageKind::SongCover {
                let index = crate::utils::audio_index::snapshot();
                let track = self.current_ncm_track(request.id);
                let current = self.playback.current_song.as_ref().filter(|song| {
                    crate::image::ncm_song_id(song.id, &song.file_path) == Some(request.id)
                });
                let artist = track
                    .map(|track| track.artist_names())
                    .or_else(|| current.map(|song| song.artist.clone()));
                let title = track
                    .map(|track| track.title.as_str())
                    .or_else(|| current.map(|song| song.title.as_str()));
                index
                    .locate("", -(request.id as i64), artist.as_deref(), title)
                    .map(|file| file.path.clone())
            } else {
                None
            };
            let (task, handle) = start_image_download(client.clone(), request.clone(), local_audio);
            self.ui.image_state.mark_request_inflight(&request, handle);
            tasks.push(task);
        }
        Task::batch(tasks)
    }

    fn after_image_ready(
        &mut self,
        kind: ImageKind,
        id: u64,
        _variant: ImageVariant,
        path: &std::path::Path,
    ) -> Task<Message> {
        if !matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover) {
            return Task::none();
        }
        if let Some(song) = self.playback.current_song.as_ref()
            && crate::image::song_cover_key_for_source(song.id, &song.file_path) == Some((kind, id))
        {
            return self.prepare_lyrics_background_for_cover_path(song.id, path.to_owned());
        }
        Task::none()
    }

    pub(crate) fn store_image_path(&mut self, kind: ImageKind, id: u64, path: std::path::PathBuf) {
        let source = path.to_string_lossy().into_owned();
        self.ui
            .image_state
            .known_sources
            .insert((kind, id), source.clone());
        if !matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover) {
            self.ui.image_state.enqueue_variant_with_scope(
                kind,
                id,
                ImageVariant::Thumbnail,
                source,
                ImageRequestScope::Global,
            );
        }
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
        for song in songs {
            if let Some((kind, id)) =
                crate::image::song_cover_key_for_source(song.id, &song.file_path)
            {
                let source = if kind == ImageKind::LocalSongCover {
                    Some(song.file_path.clone())
                } else {
                    song.cover_path
                        .as_deref()
                        .filter(|p| crate::image::is_remote_url(p))
                        .map(str::to_owned)
                };
                if let Some(source) = source {
                    self.ui.image_state.known_sources.insert((kind, id), source);
                }
            }
        }
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

    fn sync_preferred_image_to_current_page(&mut self, kind: ImageKind, id: u64) {
        if self
            .ui
            .playlist_page
            .current
            .as_ref()
            .is_some_and(|page| page.is_local)
        {
            self.sync_local_header_cover();
            return;
        }
        let Some(path) = self
            .ui
            .image_state
            .image_data_variant(kind, id, ImageVariant::Detail)
            .or_else(|| self.ui.image_state.image_data(kind, id))
            .map(|(path, _, _)| path.clone())
        else {
            return;
        };
        let palette = self
            .ui
            .image_state
            .entries
            .get(&(kind, id, ImageVariant::Detail))
            .or_else(|| {
                self.ui
                    .image_state
                    .entries
                    .get(&(kind, id, ImageVariant::Thumbnail))
            })
            .and_then(|entry| entry.palette.clone());
        let Some(page) = self.ui.playlist_page.current.as_mut() else {
            return;
        };

        let path_string = path.to_string_lossy().to_string();
        match kind {
            ImageKind::RadioCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Podcast
                    && page.id == super::podcast::podcast_page_id(id) =>
            {
                set_detail_cover(page, palette, path_string);
            }
            ImageKind::PlaylistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Playlist
                    && page.id == ncm_playlist_page_id(id) =>
            {
                set_detail_cover(page, palette, path_string);
            }
            ImageKind::LocalPlaylistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Playlist
                    && page.id == id as i64 =>
            {
                set_detail_cover(page, palette, path_string);
            }
            ImageKind::AlbumCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Album
                    && page.id == album_page_id(id) =>
            {
                set_detail_cover(page, palette, path_string);
            }
            ImageKind::ArtistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::Artist
                    && page.id == artist_page_id(id) =>
            {
                set_detail_cover(page, palette, path_string);
            }
            ImageKind::ArtistCover
                if page.kind == crate::ui::pages::playlist::DetailPageKind::User
                    && page.owner_artist_id == Some(id) =>
            {
                set_detail_cover(page, palette, path_string);
            }
            ImageKind::ArtistCover if page.owner_artist_id == Some(id) => {
                page.owner_avatar_path = Some(path_string);
            }
            ImageKind::UserAvatar if page.creator_id == id => {
                if page.kind == crate::ui::pages::playlist::DetailPageKind::User
                    && page.cover_path.is_none()
                {
                    set_detail_cover(page, palette, path_string);
                } else {
                    page.owner_avatar_path = Some(path_string);
                }
            }
            _ => {}
        }
    }

    fn sync_local_header_cover(&mut self) {
        let Some(page) = self
            .ui
            .playlist_page
            .current
            .as_mut()
            .filter(|page| page.is_local)
        else {
            return;
        };
        if let Some(entry) =
            crate::ui::pages::playlist::local_header_cover_entry(page, &self.ui.image_state)
        {
            set_detail_cover(
                page,
                entry.palette.clone(),
                entry.path.to_string_lossy().into_owned(),
            );
        }
    }
}

fn start_image_download(
    client: Option<crate::api::NcmClient>,
    request: ImageRequest,
    local_audio: Option<std::path::PathBuf>,
) -> (Task<Message>, iced::task::Handle) {
    let ImageRequest {
        kind,
        id,
        variant,
        url,
        generation,
        scope,
    } = request;
    Task::perform(
        async move {
            let source = url.clone();
            let local = local_audio.or_else(|| {
                (!crate::image::is_remote_url(&url)).then(|| std::path::PathBuf::from(&url))
            });
            let decoded = if let Some(path) = local {
                tokio::task::spawn_blocking(move || {
                    crate::image::artwork::load(&path).map(|image| (path, image))
                })
                .await
                .ok()
                .flatten()
            } else {
                None
            };
            let url = if decoded.is_none() && url.starts_with("ncm://") {
                if let Some(client) = client.as_ref() {
                    client
                        .track_detail(&[id])
                        .await
                        .ok()
                        .and_then(|tracks| {
                            tracks
                                .into_iter()
                                .find(|track| track.id == id)
                                .map(|track| track.cover_url().to_owned())
                        })
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            } else {
                url
            };
            let decoded = if decoded.is_some() {
                decoded
            } else if crate::image::is_remote_url(&url) {
                if matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover) {
                    if let Some(client) = client.as_ref() {
                        match client
                            .image_bytes(&url, kind.requested_resize(variant))
                            .await
                        {
                            Ok(bytes) => tokio::task::spawn_blocking(move || {
                                image::load_from_memory(&bytes)
                                    .ok()
                                    .map(|image| (std::path::PathBuf::from(url), image))
                            })
                            .await
                            .ok()
                            .flatten(),
                            Err(error) => {
                                tracing::warn!(%error, "Song artwork request failed");
                                None
                            }
                        }
                    } else {
                        None
                    }
                } else {
                    load_collection_image(
                        client.as_ref(),
                        kind,
                        id,
                        variant,
                        &url,
                        kind.cache_dir(),
                    )
                    .await
                }
            } else {
                None
            };
            let (path, image) = decoded?;
            let backing_file = path.is_absolute().then(|| path.clone());
            let path = if matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover) {
                std::path::PathBuf::from(format!("artwork://{kind:?}/{id}/{variant:?}"))
            } else {
                path
            };
            tokio::task::spawn_blocking(move || {
                let mut entry = crate::image::artwork::prepare(kind, variant, path, source, image);
                entry.backing_file = backing_file;
                std::sync::Arc::new(entry)
            })
            .await
            .ok()
        },
        move |result| match result {
            Some(entry) => Message::ImageDownloadReady(generation, scope, kind, id, variant, entry),
            None => Message::ImageDownloadFailed(generation, scope, kind, id, variant),
        },
    )
    .abortable()
}

/// Collection caches remain available before login and while offline. Song
/// artwork never enters this disk-backed branch.
async fn load_collection_image(
    client: Option<&crate::api::NcmClient>,
    kind: ImageKind,
    id: u64,
    variant: ImageVariant,
    url: &str,
    cache_dir: std::path::PathBuf,
) -> Option<(std::path::PathBuf, image::DynamicImage)> {
    if matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover) {
        return None;
    }
    let lookup_dir = cache_dir.clone();
    let lookup_url = url.to_owned();
    let cached = tokio::task::spawn_blocking(move || {
        crate::image::resolve_remote_cached(&lookup_dir, kind, id, variant, &lookup_url)
    })
    .await
    .ok()
    .flatten();
    let path = if cached.is_some() {
        cached
    } else if let Some(client) = client {
        crate::utils::download_img(
            client,
            url,
            cache_dir.join(format!(
                "{}.jpg",
                crate::image::remote_file_stem(kind, id, variant, url)
            )),
            kind.requested_resize(variant),
        )
        .await
    } else {
        None
    };
    tokio::task::spawn_blocking(move || {
        let path =
            path.or_else(|| crate::utils::find_cached_image(&cache_dir, &kind.file_stem(id)))?;
        crate::image::artwork::load(&path).map(|image| (path, image))
    })
    .await
    .ok()
    .flatten()
}

struct RemoteImage {
    kind: ImageKind,
    id: u64,
    variant: ImageVariant,
    url: String,
    scope: ImageRequestScope,
}

impl RemoteImage {
    fn new(kind: ImageKind, id: u64, url: &str) -> Self {
        Self {
            kind,
            id,
            variant: ImageVariant::Thumbnail,
            url: url.to_string(),
            scope: ImageRequestScope::Page,
        }
    }

    fn detail(kind: ImageKind, id: u64, url: &str) -> Self {
        Self {
            kind,
            id,
            variant: ImageVariant::Detail,
            url: url.to_string(),
            scope: ImageRequestScope::Page,
        }
    }

    fn global(kind: ImageKind, id: u64, url: &str) -> Self {
        Self {
            kind,
            id,
            variant: ImageVariant::Thumbnail,
            url: url.to_string(),
            scope: ImageRequestScope::Global,
        }
    }

    fn viewport(kind: ImageKind, id: u64, url: &str) -> Self {
        Self {
            kind,
            id,
            variant: ImageVariant::Thumbnail,
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

fn current_song_remote_variants(
    kind: ImageKind,
    full_screen_artwork_open: bool,
) -> Vec<ImageVariant> {
    let mut variants = vec![ImageVariant::Thumbnail];
    if full_screen_artwork_open && kind == ImageKind::SongCover {
        variants.push(ImageVariant::Hero);
    }
    variants
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
    palette: Option<crate::utils::ColorPalette>,
    path_string: String,
) {
    page.cover_path = Some(path_string);
    // A higher resolution derivative can be unavailable or temporarily
    // undecodable. Keep the palette extracted from the already visible
    // thumbnail instead of replacing it with `None` and flashing defaults.
    if palette.is_some() {
        page.palette = palette;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn offline_collection_cache_survives_but_song_disk_cache_is_never_used() {
        let root =
            crate::cache::unique_temp_path(&std::env::temp_dir().join("rustle-offline-artwork"));
        std::fs::create_dir_all(&root).unwrap();
        let url = "https://example.invalid/artwork";
        let variant = ImageVariant::Thumbnail;
        for kind in [ImageKind::AlbumCover, ImageKind::SongCover] {
            let path = root.join(format!(
                "{}.png",
                crate::image::remote_file_stem(kind, 42, variant, url)
            ));
            image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
                .save(path)
                .unwrap();
        }
        assert!(
            load_collection_image(None, ImageKind::AlbumCover, 42, variant, url, root.clone())
                .await
                .is_some()
        );
        assert!(
            load_collection_image(None, ImageKind::SongCover, 42, variant, url, root.clone())
                .await
                .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn image_roles_map_to_research_dimensions() {
        let thumbnail = RemoteImage::new(
            ImageKind::PlaylistCover,
            1,
            "https://example.invalid/cover.jpg",
        );
        let detail = RemoteImage::detail(
            ImageKind::PlaylistCover,
            1,
            "https://example.invalid/cover.jpg",
        );

        assert_eq!(thumbnail.variant, ImageVariant::Thumbnail);
        assert_eq!(
            thumbnail.kind.requested_resize(thumbnail.variant),
            Some((300, 300))
        );
        assert_eq!(detail.variant, ImageVariant::Detail);
        assert_eq!(
            detail.kind.requested_resize(detail.variant),
            Some((600, 600))
        );
        assert_eq!(
            ImageKind::SongCover.requested_resize(ImageVariant::Hero),
            Some((1024, 1024))
        );
        assert_eq!(
            current_song_remote_variants(ImageKind::SongCover, false),
            vec![ImageVariant::Thumbnail]
        );
        assert_eq!(
            current_song_remote_variants(ImageKind::SongCover, true),
            vec![ImageVariant::Thumbnail, ImageVariant::Hero]
        );
        assert_eq!(
            current_song_remote_variants(ImageKind::LocalSongCover, true),
            vec![ImageVariant::Thumbnail]
        );
    }

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
