//! DJ radio details share playlist presentation while retaining radio identity.
use iced::Task;

use super::page_loader::{PlaylistLoadState, convert_ncm_tracks_to_views_with_offset};
use crate::app::{App, Message, Route};
use crate::ui::pages::playlist::{ArtistPageTab, DetailPageKind, PlaylistView};

const PROGRAM_BATCH_SIZE: u32 = 500;

pub(super) fn podcast_page_id(id: u64) -> i64 {
    i64::MIN / 2 + id as i64
}

impl App {
    pub(super) fn open_podcast_route(&mut self, id: u64) -> Task<Message> {
        let preview = self
            .ui
            .search
            .radios
            .iter()
            .find(|radio| radio.id == id)
            .cloned();
        self.reset_playlist_page_state();
        let generation = self.ui.playlist_page.ncm_load_generation;
        let page_id = podcast_page_id(id);
        self.ui.playlist_page.current = Some(PlaylistView {
            kind: DetailPageKind::Podcast,
            id: page_id,
            name: preview
                .as_ref()
                .map(|radio| radio.name.clone())
                .unwrap_or_else(|| "加载播客…".into()),
            description: None,
            profile_stats: None,
            artist_tab: ArtistPageTab::TopSongs,
            artist_albums: Vec::new(),
            user_playlists: Vec::new(),
            cover_path: None,
            owner: preview
                .as_ref()
                .map(|radio| radio.creator.nickname.clone())
                .unwrap_or_default(),
            owner_artist_id: None,
            owner_avatar_path: None,
            creator_id: preview.as_ref().map_or(0, |radio| radio.creator.id),
            song_count: preview.as_ref().map_or(0, |radio| radio.program_count),
            total_duration: "正在加载节目…".into(),
            like_count: String::new(),
            songs: Vec::new(),
            palette: None,
            is_local: false,
            is_subscribed: false,
            watched_folder_path: None,
            watch_enabled: false,
        });
        self.ui
            .playlist_page
            .begin_online_tracks(page_id, generation);
        self.ui.playlist_page.load_state = PlaylistLoadState::Loading;
        let Some(client) = self.core.ncm_client.clone() else {
            return self.radio_load_failed("请先登录再查看播客");
        };
        let (task, handle) = Task::perform(
            async move { client.radio_detail(id).await.map_err(Into::into) },
            move |result| Message::RadioDetailLoaded(generation, id, result),
        )
        .abortable();
        self.ui.playlist_page.podcast_request = Some(handle);
        task
    }

    fn fetch_radio_programs(&mut self, generation: u64, id: u64, offset: u32) -> Task<Message> {
        let Some(client) = self.core.ncm_client.clone() else {
            return Task::done(Message::RadioProgramsLoaded(
                generation,
                id,
                offset,
                Err(crate::error::AppError::new(
                    crate::error::ErrorCode::AuthenticationRequired,
                    "请先登录再查看播客",
                )),
            ));
        };
        let (task, handle) = Task::perform(
            async move {
                client
                    .radio_programs(id, offset, PROGRAM_BATCH_SIZE)
                    .await
                    .map_err(Into::into)
            },
            move |result| Message::RadioProgramsLoaded(generation, id, offset, result),
        )
        .abortable();
        self.ui.playlist_page.podcast_request = Some(handle);
        task
    }

    fn radio_request_is_current(&self, generation: u64, id: u64) -> bool {
        self.ui.current_route == Route::Podcast(id)
            && self.ui.playlist_page.ncm_load_generation == generation
    }

    pub(super) fn handle_podcast(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::RadioDetailLoaded(generation, id, result) => {
                if !self.radio_request_is_current(*generation, *id) {
                    return Some(Task::none());
                }
                self.ui.playlist_page.podcast_request = None;
                match result {
                    Ok(detail) if detail.radio.id == *id => {
                        if let Some(page) = &mut self.ui.playlist_page.current {
                            page.name = detail.radio.name.clone();
                            page.description = Some(detail.description.clone());
                            page.owner = detail.radio.creator.nickname.clone();
                            page.creator_id = detail.radio.creator.id;
                            page.song_count = detail.radio.program_count;
                        }
                        Some(self.fetch_radio_programs(*generation, *id, 0))
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Podcast detail failed");
                        Some(self.radio_load_failed("播客详情加载失败，请重新打开重试"))
                    }
                    Ok(_) => Some(self.radio_load_failed("播客详情不匹配，请重新打开重试")),
                }
            }
            Message::RadioProgramsLoaded(generation, id, offset, result) => {
                if !self.radio_request_is_current(*generation, *id) {
                    return Some(Task::none());
                }
                self.ui.playlist_page.podcast_request = None;
                match result {
                    Ok(batch) => {
                        let existing = self
                            .ui
                            .playlist_page
                            .current_online_tracks()
                            .unwrap_or_default();
                        let mut seen: std::collections::HashSet<_> =
                            existing.iter().map(|track| track.id).collect();
                        let tracks: Vec<_> = batch
                            .tracks
                            .iter()
                            .filter(|track| seen.insert(track.id))
                            .cloned()
                            .collect();
                        let views =
                            convert_ncm_tracks_to_views_with_offset(&tracks, existing.len());
                        self.ui.playlist_page.append_online_tracks(
                            podcast_page_id(*id),
                            *generation,
                            &tracks,
                        );
                        if let Some(page) = &mut self.ui.playlist_page.current {
                            page.songs.extend(views);
                            page.total_duration = if batch.more {
                                format!("已加载 {} 期，正在加载…", page.songs.len())
                            } else {
                                format!("{} 期节目", page.songs.len())
                            };
                        }
                        if batch.more
                            && *offset > 0
                            && !batch.tracks.is_empty()
                            && tracks.is_empty()
                        {
                            return Some(
                                self.radio_load_failed("接口重复返回已有节目，请重新打开重试"),
                            );
                        }
                        if batch.more && batch.received > 0 {
                            Some(self.fetch_radio_programs(
                                *generation,
                                *id,
                                offset.saturating_add(batch.received),
                            ))
                        } else if batch.more {
                            Some(self.radio_load_failed("节目列表未完整返回，请重新打开重试"))
                        } else {
                            self.ui.playlist_page.load_state = PlaylistLoadState::Ready;
                            Some(Task::none())
                        }
                    }
                    Err(error) => {
                        tracing::warn!(radio_id = id, offset, error_code = %error.code().as_str(), %error, "Podcast programs failed");
                        Some(self.radio_load_failed(&format!(
                            "节目加载失败：{}。已加载内容保留，请稍后重试",
                            error.user_summary()
                        )))
                    }
                }
            }
            _ => None,
        }
    }

    fn radio_load_failed(&mut self, message: &str) -> Task<Message> {
        self.ui.playlist_page.load_state = PlaylistLoadState::Ready;
        if let Some(page) = &mut self.ui.playlist_page.current {
            page.total_duration = message.into();
        }
        Self::toast_error(message.to_string())
    }
}
