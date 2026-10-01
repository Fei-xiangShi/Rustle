//! Context menu and song info/edit dialog handlers

use iced::Task;

use super::super::App;
use crate::app::state::SongEditDialogState;
use crate::app::{ContextMenuAction, Message};
use crate::database::DbSong;
use crate::i18n::Key;
use crate::ui::overlay::{ModalKind, OverlayKind};

impl App {
    pub fn handle_context_menu(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::RightClickSong(song_id) => {
                let pos = self.core.mouse_position;
                let song = self.find_song_anywhere(*song_id);
                let (source, is_liked) = song
                    .as_ref()
                    .map(|s| {
                        let src = crate::utils::compute_source(
                            &s.file_path,
                            s.id,
                            Some(&s.artist),
                            Some(&s.title),
                        );
                        let liked = if s.id < 0 {
                            self.core
                                .user_info
                                .as_ref()
                                .map(|u| u.like_songs.contains(&((-s.id) as u64)))
                                .unwrap_or(false)
                        } else {
                            false
                        };
                        (src, liked)
                    })
                    .unwrap_or((crate::utils::Source::Online, false));
                self.ui.context_menu = Some(crate::app::state::ContextMenuState {
                    song_id: *song_id,
                    x: pos.x,
                    y: pos.y,
                    source,
                    is_liked,
                });
                Some(Task::none())
            }
            Message::CloseContextMenu => {
                self.ui.context_menu = None;
                Some(Task::none())
            }
            Message::ContextMenuAction(action, song_id) => {
                self.ui.context_menu = None;
                self.dispatch_menu_action(*action, *song_id)
            }
            // ── Song Info / Edit ──
            Message::EditSongTags(song_id) => Some(self.open_song_edit_dialog(*song_id)),
            Message::OpenSongEditDialog(data) => {
                let (s, meta, cover) = data.as_ref();
                let edit_state = SongEditDialogState {
                    song_id: s.id,
                    title: meta.title.clone(),
                    artist: meta.artist.clone(),
                    album: meta.album.clone(),
                    track_number: meta.track_number,
                    year: meta.year,
                    genre: meta.genre.clone().unwrap_or_default(),
                    cover_path: None,
                    cover_handle: cover.clone(),
                };
                self.ui.song_edit_dialog = Some(edit_state.clone());
                use crate::ui::overlay::{ModalConfig, ModalKind, OverlayEntry, OverlayKind};
                self.ui
                    .overlay_stack
                    .push(OverlayEntry::new(OverlayKind::Modal(
                        ModalKind::SongEdit(edit_state),
                        ModalConfig::default().width(600.0),
                    )));
                Some(Task::none())
            }
            Message::SongEditFieldChanged {
                song_id,
                field,
                value,
            } => {
                if let Some(ref mut e) = self.ui.song_edit_dialog
                    && e.song_id == *song_id
                {
                    match field.as_str() {
                        "title" => e.title = value.clone(),
                        "artist" => e.artist = value.clone(),
                        "album" => e.album = value.clone(),
                        "track_number" => {
                            e.track_number = value.parse::<u32>().ok();
                        }
                        "year" => {
                            e.year = value.parse::<u32>().ok();
                        }
                        "genre" => e.genre = value.clone(),
                        _ => {}
                    }
                }
                // Also update overlay entry so the view reads current values
                if let Some(entry) = self.ui.overlay_stack.last_mut()
                    && let OverlayKind::Modal(ModalKind::SongEdit(state), _) = &mut entry.kind
                    && state.song_id == *song_id
                {
                    match field.as_str() {
                        "title" => state.title = value.clone(),
                        "artist" => state.artist = value.clone(),
                        "album" => state.album = value.clone(),
                        "track_number" => {
                            state.track_number = value.parse::<u32>().ok();
                        }
                        "year" => {
                            state.year = value.parse::<u32>().ok();
                        }
                        "genre" => state.genre = value.clone(),
                        _ => {}
                    }
                }
                Some(Task::none())
            }
            Message::SaveSongEdits(song_id) => self.save_song_edits(*song_id),
            Message::PickSongEditCover(song_id) => {
                let sid = *song_id;
                Some(Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    move |path| match path {
                        Some(p) => Message::SongEditCoverReplaced(sid, p),
                        None => Message::Noop,
                    },
                ))
            }
            Message::SongEditCoverReplaced(song_id, path) => {
                let p = path.clone();
                if let Some(ref mut e) = self.ui.song_edit_dialog
                    && e.song_id == *song_id
                {
                    e.cover_path = Some(p.clone());
                }
                // Also update overlay entry
                if let Some(entry) = self.ui.overlay_stack.last_mut()
                    && let OverlayKind::Modal(ModalKind::SongEdit(state), _) = &mut entry.kind
                    && state.song_id == *song_id
                {
                    state.cover_path = Some(p);
                }
                Some(Task::none())
            }
            Message::SongEditsSaved(song_id) => {
                let task = if let (Some(db), Some(song)) =
                    (self.core.db.clone(), self.find_song_anywhere(*song_id))
                {
                    Some(Task::perform(
                        async move {
                            let updated = tokio::task::spawn_blocking(move || {
                                let mut updated = song;
                                let meta = crate::metadata::SongMetadata::resolve(&updated);
                                meta.merge_into(&mut updated);
                                updated
                            })
                            .await;
                            if let Ok(updated) = updated
                                && let Err(error) = db.refresh_song_metadata(&updated).await
                            {
                                tracing::warn!(%error, "Cannot refresh edited song metadata");
                            }
                        },
                        |_| Message::Noop,
                    ))
                } else {
                    None
                };
                Some(Task::batch([
                    task.unwrap_or(Task::none()),
                    Self::toast_success(self.core.locale.get(Key::SongEditSaved).to_string()),
                ]))
            }
            Message::SongEditsFailed { error, .. } => Some(Self::toast_error(format!(
                "{}: {}",
                self.core.locale.get(Key::SongEditFailed),
                error
            ))),
            Message::NcmPlaylistRemoveResult(ncm_id, playlist_id, result) => match result {
                Ok(mutation) => {
                    if self.ui.current_route == crate::app::Route::NcmPlaylist(*playlist_id) {
                        if let Ok(song_id) = i64::try_from(*ncm_id) {
                            let song_id = -song_id;
                            if let Some(playlist) = self.ui.playlist_page.current.as_mut() {
                                playlist.songs.retain(|song| song.id != song_id);
                            }
                        }
                        self.ui
                            .home
                            .current_ncm_playlist_songs
                            .retain(|track| track.id != *ncm_id);
                        self.ui.playlist_page.remove_current_online_track(*ncm_id);
                    }
                    tracing::debug!(
                        ncm_song_id = ncm_id,
                        ncm_playlist_id = playlist_id,
                        changed_count = mutation.changed_count,
                        retried_after_code_512 = mutation.retried_after_code_512,
                        "ncm_playlist_track_removed"
                    );
                    Some(Self::toast_success("已从歌单中删除".to_string()))
                }
                Err(error) => Some(Self::toast_error(format!("删除失败: {error}"))),
            },
            _ => None,
        }
    }

    fn dispatch_menu_action(
        &mut self,
        action: ContextMenuAction,
        song_id: i64,
    ) -> Option<Task<Message>> {
        match action {
            ContextMenuAction::PlayNow => Some(Task::done(Message::PlaySong(song_id))),
            ContextMenuAction::PlayNext => {
                let preload = self.insert_next_in_queue(song_id);
                Some(Task::batch([
                    Self::toast_success("已添加到下一首".to_string()),
                    preload,
                ]))
            }
            ContextMenuAction::AddToFavorites => {
                // NCM songs use negative id -> positive ncm_id
                let ncm_id = if song_id < 0 { (-song_id) as u64 } else { 0 };
                if ncm_id > 0 {
                    Some(Task::done(Message::ToggleFavorite(ncm_id)))
                } else {
                    Some(Self::toast_info("仅支持收藏在线歌曲".to_string()))
                }
            }
            ContextMenuAction::AddToPlaylist => self.open_playlist_picker(song_id),
            ContextMenuAction::ViewArtist => self.navigate_to_artist(song_id),
            ContextMenuAction::ViewAlbum => self.navigate_to_album(song_id),
            ContextMenuAction::ShowInFolder => Some(self.show_in_folder(song_id)),
            ContextMenuAction::Download => {
                // Enqueue song for download
                Some(Task::done(Message::DownloadSong(song_id)))
            }
            ContextMenuAction::EditSongTags => Some(self.open_song_edit_dialog(song_id)),
            ContextMenuAction::RemoveFromList => self.remove_song_from_list(song_id),
        }
    }

    // ── Helpers ──────────────────────────────────────

    pub(super) fn current_ncm_track(&self, ncm_id: u64) -> Option<&crate::api::Track> {
        self.ui
            .playlist_page
            .current_online_track(ncm_id)
            .or_else(|| {
                self.ui
                    .home
                    .current_ncm_playlist_songs
                    .iter()
                    .find(|track| track.id == ncm_id)
            })
    }

    fn insert_next_in_queue(&mut self, song_id: i64) -> Task<Message> {
        if song_id < 0
            && let Some(track) = self.current_ncm_track((-song_id) as u64).cloned()
        {
            self.extend_queue_artist_metadata(std::slice::from_ref(&track));
        }

        let song = self.find_song_anywhere(song_id);
        if let Some(s) = song {
            let idx = self.playback.current_index.map(|i| i + 1).unwrap_or(0);
            if idx <= self.playback.queue.len() {
                self.playback.queue.insert(idx, s);
                self.persist_queue_snapshot();
                self.clear_shuffle_cache();
                self.cache_shuffle_indices();
                self.refresh_preload_window();
                return self.preload_adjacent_tracks_with_ncm();
            }
        }
        Task::none()
    }

    fn navigate_to_artist(&mut self, song_id: i64) -> Option<Task<Message>> {
        // Try NCM artist id first
        let ncm_id = if song_id < 0 {
            Some((-song_id) as u64)
        } else {
            None
        };
        // Search NCM playlist songs for artist info
        if let Some(id) = ncm_id {
            let artist_id = self
                .current_ncm_track(id)
                .and_then(|s| s.primary_artist().map(|artist| artist.id));
            if let Some(aid) = artist_id {
                return Some(Task::done(Message::OpenArtist(aid)));
            }
        }
        // Fallback: search by artist name
        let song = self.find_song_anywhere(song_id);
        if let Some(s) = song
            && !s.artist.is_empty()
        {
            return Some(Task::done(Message::OpenArtistByName(s.artist)));
        }
        Some(Self::toast_info("无法找到歌手信息".to_string()))
    }

    fn navigate_to_album(&mut self, song_id: i64) -> Option<Task<Message>> {
        let ncm_id = if song_id < 0 {
            Some((-song_id) as u64)
        } else {
            None
        };
        if let Some(id) = ncm_id {
            let album_id = self
                .current_ncm_track(id)
                .and_then(|s| (s.album.id != 0).then_some(s.album.id));
            if let Some(aid) = album_id {
                return Some(Task::done(Message::OpenAlbum(aid)));
            }
        }
        Some(Self::toast_info("无法找到专辑信息".to_string()))
    }

    fn open_playlist_picker(&mut self, song_id: i64) -> Option<Task<Message>> {
        use crate::ui::overlay::{ModalConfig, ModalKind, OverlayEntry, OverlayKind};
        let is_ncm = song_id < 0;
        let ncm_playlists = if is_ncm {
            let owned: Vec<_> = self
                .ui
                .home
                .user_playlists
                .iter()
                .filter(|pl| !pl.subscribed)
                .cloned()
                .collect();
            Some(owned)
        } else {
            None
        };
        self.ui
            .overlay_stack
            .push(OverlayEntry::new(OverlayKind::Modal(
                ModalKind::PlaylistPicker {
                    song_id,
                    ncm_playlists,
                },
                ModalConfig::default().width(360.0),
            )));
        Some(Task::none())
    }

    fn remove_song_from_list(&mut self, song_id: i64) -> Option<Task<Message>> {
        let mut removed = false;
        // 1. Remove from current playlist if viewing one
        match self.ui.current_route {
            crate::app::Route::Playlist(playlist_id) => {
                if let Some(p) = self.ui.playlist_page.current.as_mut() {
                    p.songs.retain(|s| s.id != song_id);
                }
                if let Some(ref db) = self.core.db {
                    let db = std::sync::Arc::clone(db);
                    let pid = playlist_id;
                    return Some(Task::perform(
                        async move {
                            let _ = db.remove_song_from_playlist(pid, song_id).await;
                        },
                        move |_| Message::ShowSuccessToast("已从歌单中删除".to_string()),
                    ));
                }
                removed = true;
            }
            crate::app::Route::NcmPlaylist(playlist_id) => {
                if let Some(ref client) = self.core.ncm_client {
                    let client = client.clone();
                    let ncm_id = (-song_id) as u64;
                    return Some(Task::perform(
                        async move {
                            client
                                .mutate_playlist_tracks(
                                    playlist_id,
                                    &[ncm_id],
                                    crate::api::PlaylistTrackOperation::Delete,
                                )
                                .await
                                .map_err(crate::error::AppError::from)
                        },
                        move |result| Message::NcmPlaylistRemoveResult(ncm_id, playlist_id, result),
                    ));
                }
                return Some(Self::toast_error("未登录网易云账号".to_string()));
            }
            _ => {}
        }
        // 2. Fallback: remove from playback queue
        if let Some(pos) = self.playback.queue.iter().position(|s| s.id == song_id) {
            self.playback.queue.remove(pos);
            self.prune_queue_artist_metadata();
            if let Some(ref mut idx) = self.playback.current_index {
                if pos < *idx {
                    *idx -= 1;
                } else if pos == *idx {
                    *idx = idx.saturating_sub(1);
                }
            }
            self.persist_queue_snapshot();
            self.clear_shuffle_cache();
            self.cache_shuffle_indices();
            self.refresh_preload_window();
            removed = true;
        }
        if removed {
            Some(Task::batch([
                Self::toast_success("已从列表移除".to_string()),
                self.preload_adjacent_tracks_with_ncm(),
            ]))
        } else {
            Some(Self::toast_info("歌曲不在播放列表中".to_string()))
        }
    }

    pub(super) fn find_song_anywhere(&self, song_id: i64) -> Option<DbSong> {
        if let Some(s) = self.library.db_songs.iter().find(|s| s.id == song_id) {
            return Some(s.clone());
        }
        if let Some(s) = self.playback.queue.iter().find(|s| s.id == song_id) {
            return Some(s.clone());
        }
        if self.playback.current_song.as_ref().map(|s| s.id) == Some(song_id) {
            return self.playback.current_song.clone();
        }
        if song_id < 0 {
            let ncm_id = (-song_id) as u64;
            if let Some(info) = self.current_ncm_track(ncm_id) {
                let mut song = Self::ncm_track_to_db_song(info);
                if let Some(file) = crate::utils::audio_index::snapshot().locate(
                    &song.file_path,
                    song.id,
                    Some(&song.artist),
                    Some(&song.title),
                ) {
                    song.file_path = file.path.to_string_lossy().into_owned();
                }
                return Some(song);
            }
        }
        None
    }

    fn show_in_folder(&mut self, song_id: i64) -> Task<Message> {
        let song = self.find_song_anywhere(song_id);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let Some(song) = song else {
                        return Message::ShowErrorToast("无法找到文件".into());
                    };
                    let index = crate::utils::audio_index::snapshot();
                    let path = index
                        .locate(
                            &song.file_path,
                            song.id,
                            Some(&song.artist),
                            Some(&song.title),
                        )
                        .map(|file| file.path.clone())
                        .unwrap_or_else(|| song.file_path.into());
                    if !path.is_absolute() || !path.is_file() {
                        return Message::ShowErrorToast("无法找到文件，可能已被移动或清理".into());
                    }
                    match crate::platform::open_in_file_manager(&path) {
                        Ok(()) => Message::Noop,
                        Err(error) => {
                            tracing::warn!(%error, "Cannot reveal audio file");
                            Message::ShowErrorToast("无法在文件管理器中定位文件".into())
                        }
                    }
                })
                .await
                .unwrap_or_else(|_| Message::ShowErrorToast("打开文件位置失败".into()))
            },
            |message| message,
        )
    }

    fn open_song_edit_dialog(&mut self, song_id: i64) -> Task<Message> {
        let song = self.find_song_anywhere(song_id);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let Some(mut s) = song else {
                        return Message::ShowErrorToast("歌曲未找到".into());
                    };
                    if let Some(file) = crate::utils::audio_index::snapshot().locate(
                        &s.file_path,
                        s.id,
                        Some(&s.artist),
                        Some(&s.title),
                    ) {
                        s.file_path = file.path.to_string_lossy().into_owned();
                    }
                    let meta = crate::metadata::SongMetadata::resolve(&s);
                    let cover = meta.cover_handle(&s.file_path);
                    Message::OpenSongEditDialog(Box::new((s, meta, cover)))
                })
                .await
                .unwrap_or_else(|_| Message::ShowErrorToast("读取歌曲信息失败".into()))
            },
            |msg| msg,
        )
    }

    fn save_song_edits(&mut self, song_id: i64) -> Option<Task<Message>> {
        let edit = self.ui.song_edit_dialog.take()?;
        let song = self.find_song_anywhere(song_id);
        let song = song?;
        let path = crate::utils::audio_index::snapshot()
            .locate(
                &song.file_path,
                song.id,
                Some(&song.artist),
                Some(&song.title),
            )
            .map(|file| file.path.clone())
            .unwrap_or_else(|| song.file_path.into());
        Some(Task::perform(
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    let cover_data = edit
                        .cover_path
                        .as_ref()
                        .map(std::fs::read)
                        .transpose()
                        .map_err(|error| {
                            rustle_media::error::MediaError::io("read replacement cover", error)
                        })?;
                    let cover_mime = cover_data.as_ref().map(|data| {
                        match crate::utils::detect_image_format(data) {
                            "png" => "image/png",
                            "webp" => "image/webp",
                            "gif" => "image/gif",
                            _ => "image/jpeg",
                        }
                        .to_owned()
                    });
                    crate::features::import::save_metadata(
                        &path,
                        &crate::features::import::MetadataEdits {
                            title: Some(edit.title),
                            artist: Some(edit.artist),
                            album: Some(edit.album),
                            track_number: edit.track_number,
                            year: edit.year,
                            genre: Some(edit.genre),
                            cover_data,
                            cover_mime,
                            lyrics: None,
                        },
                    )
                })
                .await;
                match result {
                    Ok(Ok(())) => Message::SongEditsSaved(song_id),
                    Ok(Err(error)) => Message::SongEditsFailed {
                        song_id,
                        error: error.into(),
                    },
                    Err(error) => {
                        tracing::warn!(%error, "Song edit worker failed");
                        Message::ShowErrorToast("保存歌曲信息失败".into())
                    }
                }
            },
            |message| message,
        ))
    }
}
