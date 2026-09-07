// src/app/update/playlist.rs
//! Playlist page and edit dialog message handlers

use iced::Task;

use crate::app::helpers::{load_playlist_view, load_watched_folders};
use crate::app::message::Message;
use crate::app::state::{App, Route};
use crate::ui::overlay::{ModalKind, OverlayKind};

impl App {
    pub(super) fn open_local_playlist_route(&mut self, playlist_id: i64) -> Task<Message> {
        if self.is_viewing_playlist(playlist_id) {
            tracing::debug!("Already viewing playlist {}, skipping load", playlist_id);
            return Task::none();
        }

        tracing::info!("Opening playlist: {}", playlist_id);
        self.reset_playlist_page_state();

        // Publish a lightweight local skeleton immediately. This keeps the
        // route from flashing to an empty surface while the database task
        // loads songs and computes the full view model.
        let preview_name = self
            .library
            .playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.name.clone())
            .unwrap_or_else(|| "加载中...".to_string());
        self.ui.playlist_page.current = Some(crate::ui::pages::PlaylistView {
            kind: crate::ui::pages::playlist::DetailPageKind::Playlist,
            id: playlist_id,
            name: preview_name,
            description: None,
            profile_stats: None,
            artist_tab: crate::ui::pages::playlist::ArtistPageTab::TopSongs,
            artist_albums: Vec::new(),
            user_playlists: Vec::new(),
            cover_path: None,
            owner: "本地".to_string(),
            owner_artist_id: None,
            owner_avatar_path: None,
            creator_id: 0,
            song_count: 0,
            total_duration: String::new(),
            like_count: String::new(),
            songs: Vec::new(),
            palette: None,
            is_local: true,
            is_subscribed: false,
            watched_folder_path: None,
            watch_enabled: false,
        });
        self.ui.playlist_page.load_state =
            crate::app::update::page_loader::PlaylistLoadState::Loading;

        if let Some(db) = &self.core.db {
            let db = db.clone();
            Task::perform(load_playlist_view(db, playlist_id), |result| match result {
                Some(payload) => Message::PlaylistViewLoaded(payload),
                None => Message::DatabaseError(crate::error::AppError::new(
                    crate::error::ErrorCode::StorageNotFound,
                    "The playlist was not found",
                )),
            })
        } else {
            Task::none()
        }
    }

    pub(super) fn reset_playlist_page_state(&mut self) {
        self.ui.playlist_page.search_expanded = false;
        self.ui.playlist_page.search_query.clear();
        self.ui.playlist_page.viewing_recently_played = false;
        self.ui.playlist_page.ncm_cache_baseline = None;
        self.ui.playlist_page.ncm_replace_songs_on_chunk = false;
        self.ui.playlist_page.scroll_state.borrow_mut().jump_to(0.0);
        self.ui.clear_playlist_animations();
        self.ui.playlist_page.reset_gradient_animation();

        if self.ui.lyrics.is_open {
            self.ui.lyrics.is_open = false;
            self.ui.lyrics.animation.stop();
        }
    }

    /// Handle playlist-related messages
    pub fn handle_playlist(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::OpenPlaylist(id) => {
                let route = Route::Playlist(*id);
                if self.ui.current_route != route {
                    return Some(self.navigate_to_route(route, true));
                }

                Some(self.open_local_playlist_route(*id))
            }

            Message::RequestDeletePlaylist(id) => {
                tracing::info!("Requesting delete for playlist: {}", id);
                let name = self
                    .library
                    .playlists
                    .iter()
                    .find(|p| p.id == *id)
                    .map(|p| p.name.as_str())
                    .unwrap_or("Unknown");
                use crate::ui::overlay::{ModalConfig, ModalKind, OverlayEntry, OverlayKind};
                self.ui
                    .overlay_stack
                    .push(OverlayEntry::new(OverlayKind::Modal(
                        ModalKind::DeleteConfirm {
                            playlist_id: *id,
                            playlist_name: name.to_string(),
                        },
                        ModalConfig::default().width(380.0),
                    )));
                Some(Task::none())
            }

            Message::ConfirmDeletePlaylist => {
                let playlist_id = self.ui.overlay_stack.last().and_then(|e| match &e.kind {
                    OverlayKind::Modal(ModalKind::DeleteConfirm { playlist_id, .. }, _) => {
                        Some(*playlist_id)
                    }
                    _ => None,
                });
                if let Some(playlist_id) = playlist_id {
                    tracing::info!("Confirming delete for playlist: {}", playlist_id);
                    self.ui.overlay_stack.clear();
                    if let Some(db) = &self.core.db {
                        let db = db.clone();
                        return Some(Task::perform(
                            async move {
                                let _ = db.delete_watched_folder_by_playlist(playlist_id).await;
                                db.delete_playlist(playlist_id).await.ok();
                                playlist_id
                            },
                            Message::PlaylistDeleted,
                        ));
                    }
                }
                Some(Task::none())
            }

            Message::RequestDownloadPlaylist(playlist_id, name, count) => {
                tracing::info!(
                    "Requesting download for playlist: {} ({} songs)",
                    playlist_id,
                    count
                );
                use crate::ui::overlay::{ModalConfig, ModalKind, OverlayEntry, OverlayKind};
                self.ui
                    .overlay_stack
                    .push(OverlayEntry::new(OverlayKind::Modal(
                        ModalKind::DownloadConfirm {
                            playlist_id: *playlist_id,
                            playlist_name: name.clone(),
                            song_count: *count,
                        },
                        ModalConfig::default().width(380.0),
                    )));
                Some(Task::none())
            }

            Message::ConfirmDownloadPlaylist => {
                let playlist_id = self.ui.overlay_stack.last().and_then(|e| match &e.kind {
                    OverlayKind::Modal(ModalKind::DownloadConfirm { playlist_id, .. }, _) => {
                        Some(*playlist_id)
                    }
                    _ => None,
                });
                if let Some(playlist_id) = playlist_id {
                    tracing::info!("Confirming download for playlist: {}", playlist_id);
                    self.ui.overlay_stack.clear();
                    return Some(Task::done(Message::DownloadPlaylist(playlist_id)));
                }
                Some(Task::none())
            }

            Message::PlaylistDeleted(id) => {
                tracing::info!("Playlist {} deleted", id);
                // Remove from sidebar list
                self.library.playlists.retain(|p| p.id != *id);
                // Clear current playlist if it was the deleted one
                if self.ui.playlist_page.current.as_ref().map(|p| p.id) == Some(*id) {
                    self.ui.playlist_page.current = None;
                }
                if let Some(db) = &self.core.db {
                    let db = db.clone();
                    return Some(Task::batch([
                        Self::toast_success("歌单已删除".to_string()),
                        Task::perform(load_watched_folders(db), Message::WatchedFoldersLoaded),
                    ]));
                }
                Some(Self::toast_success("歌单已删除".to_string()))
            }

            Message::PlaylistViewLoaded(payload) => {
                let view = &payload.view;
                tracing::info!("Playlist view loaded: {}", view.name);
                self.store_image_paths(payload.images.iter().cloned());
                self.ui.playlist_page.current = Some(view.clone());
                self.ui.playlist_page.load_state =
                    crate::app::update::page_loader::PlaylistLoadState::Ready;
                // Reset scroll position for playlist page
                self.ui.playlist_page.scroll_state.borrow_mut().jump_to(0.0);
                Some(Task::none())
            }

            Message::HoverSong(id) => {
                self.ui
                    .playlist_page
                    .song_animations
                    .set_hovered_exclusive(*id);
                Some(Task::none())
            }

            Message::HoverIcon(id) => {
                self.ui
                    .playlist_page
                    .icon_animations
                    .set_hovered_exclusive(*id);
                Some(Task::none())
            }

            Message::HoverSidebar(id) => {
                self.ui.sidebar_animations.set_hovered_exclusive(*id);
                Some(Task::none())
            }

            Message::ToggleMyPlaylistsSection => {
                self.ui.my_playlists_expanded = !self.ui.my_playlists_expanded;
                Some(Task::none())
            }

            Message::ToggleCollectedPlaylistsSection => {
                self.ui.collected_playlists_expanded = !self.ui.collected_playlists_expanded;
                Some(Task::none())
            }

            Message::AnimationTick(now) => {
                // Update audio state
                self.update_audio_tick();

                // Check if lyrics page close animation is complete
                self.check_lyrics_page_close();

                // Update lyrics animations if lyrics page is open
                if self.ui.lyrics.is_open {
                    let _ = self.update_lyrics_animations();
                }

                // 清理已完成的淡出动画
                self.ui.cleanup_animations(*now);

                let lyrics_viewport_task = self.flush_pending_lyrics_viewport_after_animation();
                let smooth_scroll_task = self.advance_smooth_scroll(*now);

                Some(Task::batch([lyrics_viewport_task, smooth_scroll_task]))
            }

            Message::EditPlaylist(id) => {
                tracing::info!("Edit playlist: {}", id);
                if let Some(playlist) = &self.ui.playlist_page.current {
                    use crate::ui::overlay::{ModalConfig, ModalKind, OverlayEntry, OverlayKind};
                    self.ui
                        .overlay_stack
                        .push(OverlayEntry::new(OverlayKind::Modal(
                            ModalKind::PlaylistEdit {
                                playlist_id: *id,
                                name: playlist.name.clone(),
                                description: playlist.description.clone().unwrap_or_default(),
                                cover_path: playlist.cover_path.clone(),
                                watch_enabled: playlist.watch_enabled,
                                watch_available: playlist.watched_folder_path.is_some(),
                                watch_path: playlist.watched_folder_path.clone(),
                            },
                            ModalConfig::default().width(480.0),
                        )));
                }
                Some(Task::none())
            }

            Message::EditPlaylistNameChanged(name) => {
                if let Some(entry) = self.ui.overlay_stack.last_mut()
                    && let OverlayKind::Modal(ModalKind::PlaylistEdit { name: n, .. }, _) =
                        &mut entry.kind
                {
                    *n = name.clone();
                }
                Some(Task::none())
            }

            Message::EditPlaylistDescriptionChanged(desc) => {
                if let Some(entry) = self.ui.overlay_stack.last_mut()
                    && let OverlayKind::Modal(ModalKind::PlaylistEdit { description: d, .. }, _) =
                        &mut entry.kind
                {
                    *d = desc.clone();
                }
                Some(Task::none())
            }

            Message::EditPlaylistWatchEnabledChanged(enabled) => {
                if let Some(entry) = self.ui.overlay_stack.last_mut()
                    && let OverlayKind::Modal(
                        ModalKind::PlaylistEdit {
                            watch_enabled: w, ..
                        },
                        _,
                    ) = &mut entry.kind
                {
                    *w = *enabled;
                }
                Some(Task::none())
            }

            Message::PickCoverImage => Some(Task::perform(
                async {
                    let result = rfd::AsyncFileDialog::new()
                        .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                        .pick_file()
                        .await;
                    result.map(|f| f.path().to_string_lossy().to_string())
                },
                Message::CoverImagePicked,
            )),

            Message::CoverImagePicked(path) => {
                if let Some(p) = path
                    && let Some(entry) = self.ui.overlay_stack.last_mut()
                    && let OverlayKind::Modal(ModalKind::PlaylistEdit { cover_path: c, .. }, _) =
                        &mut entry.kind
                {
                    *c = Some(p.clone());
                }
                Some(Task::none())
            }

            Message::SavePlaylistEdits => {
                let edit_data = self
                    .ui
                    .overlay_stack
                    .last()
                    .and_then(|entry| match &entry.kind {
                        OverlayKind::Modal(
                            ModalKind::PlaylistEdit {
                                playlist_id,
                                name,
                                description,
                                cover_path,
                                watch_available,
                                watch_enabled,
                                watch_path: _,
                            },
                            _,
                        ) => Some((
                            *playlist_id,
                            name.clone(),
                            description.clone(),
                            cover_path.clone(),
                            *watch_available,
                            *watch_enabled,
                        )),
                        _ => None,
                    });

                if let (
                    Some(db),
                    Some((playlist_id, name, description, cover, watch_available, watch_enabled)),
                ) = (&self.core.db, edit_data)
                {
                    let db = db.clone();
                    self.ui.overlay_stack.clear();

                    return Some(Task::perform(
                        async move {
                            db.update_playlist_full(
                                playlist_id,
                                &name,
                                if description.is_empty() {
                                    None
                                } else {
                                    Some(&description)
                                },
                                cover.as_deref(),
                            )
                            .await
                            .ok();
                            if watch_available {
                                let _ = db
                                    .set_watched_folder_enabled(playlist_id, watch_enabled)
                                    .await;
                            }
                            playlist_id
                        },
                        Message::PlaylistUpdated,
                    ));
                }
                Some(Task::none())
            }

            Message::PlaylistUpdated(playlist_id) => {
                if let Some(db) = &self.core.db {
                    let db1 = db.clone();
                    let db2 = db.clone();
                    let db3 = db.clone();
                    let id = *playlist_id;
                    return Some(Task::batch([
                        Task::perform(load_playlist_view(db1, id), |result| match result {
                            Some(payload) => Message::PlaylistViewLoaded(payload),
                            None => Message::DatabaseError(crate::error::AppError::new(
                                crate::error::ErrorCode::StorageNotFound,
                                "The playlist was not found",
                            )),
                        }),
                        Task::perform(
                            async move { db2.get_all_playlists().await.unwrap_or_default() },
                            Message::PlaylistsLoaded,
                        ),
                        Task::perform(load_watched_folders(db3), Message::WatchedFoldersLoaded),
                    ]));
                }
                Some(Task::none())
            }

            Message::TogglePlaylistSearch => {
                self.ui.playlist_page.search_expanded = !self.ui.playlist_page.search_expanded;
                if self.ui.playlist_page.search_expanded {
                    self.ui.playlist_page.search_animation.start();
                    // Focus the search input
                    Some(iced::widget::operation::focus(iced::widget::Id::new(
                        "playlist_search_input",
                    )))
                } else {
                    self.ui.playlist_page.search_animation.stop();
                    self.ui.playlist_page.search_query.clear();
                    Some(Task::none())
                }
            }

            Message::PlaylistSearchChanged(query) => {
                self.ui.playlist_page.search_query = query.clone();
                Some(Task::none())
            }

            Message::PlaylistSearchSubmit => {
                // Search is already applied via filtering in view
                // This just handles the Enter key press
                Some(Task::none())
            }

            Message::PlaylistSearchBlur => {
                // If search query is empty and input loses focus, collapse the search
                if self.ui.playlist_page.search_query.is_empty()
                    && self.ui.playlist_page.search_expanded
                {
                    self.ui.playlist_page.search_expanded = false;
                    self.ui.playlist_page.search_animation.stop();
                }
                Some(Task::none())
            }

            Message::ToggleDescriptionExpand => {
                self.ui.playlist_page.description_expanded =
                    !self.ui.playlist_page.description_expanded;
                Some(Task::none())
            }

            Message::PlaylistPickerConfirm(song_id, playlist_id) => {
                let sid = *song_id;
                let pid = *playlist_id;
                self.ui.overlay_stack.pop();
                if let Some(ref db) = self.core.db {
                    let db = std::sync::Arc::clone(db);
                    let locale = self.core.locale;
                    return Some(Task::perform(
                        async move {
                            let _ = db.add_song_to_playlist(pid, sid).await;
                        },
                        move |_| {
                            Message::ShowSuccessToast(
                                locale
                                    .get(crate::i18n::Key::SongAddedToPlaylist)
                                    .to_string(),
                            )
                        },
                    ));
                }
                Some(Task::none())
            }

            _ => None,
        }
    }
}
