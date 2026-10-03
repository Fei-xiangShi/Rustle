// src/app/update/search.rs
//! Search message handlers

use iced::Task;

use crate::api::SearchType;
use crate::app::message::{
    Message, SearchErrorPayload, SearchRequestContext, SearchResultsPayload,
};
use crate::app::state::{App, Route, SearchTab};

/// Default number of results per page
use crate::app::state::SEARCH_PAGE_SIZE as PAGE_SIZE;

impl App {
    pub(super) fn clear_search_cover_cache(&mut self) {
        // Search result images are stored in the unified ImageState cache.
    }

    /// Handle search-related messages
    pub fn handle_search(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::SearchChanged(query) => {
                self.ui.search_query = query.clone();
                let state = &mut self.ui.search.suggestions;
                state.close();
                state.items.clear();
                if query.trim().is_empty() {
                    return Some(Task::none());
                }
                let generation = state.generation;
                let (task, handle) = Task::perform(
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        generation
                    },
                    Message::SearchSuggestDue,
                )
                .abortable();
                state.pending = Some(handle);
                Some(task)
            }
            Message::SearchSuggestDue(generation) => {
                if *generation != self.ui.search.suggestions.generation {
                    return Some(Task::none());
                }
                let Some(client) = self.core.ncm_client.clone() else {
                    return Some(Task::none());
                };
                let keyword = self.ui.search_query.trim().to_string();
                if keyword.is_empty() {
                    return Some(Task::none());
                }
                let generation = *generation;
                self.ui.search.suggestions.open = true;
                self.ui.search.suggestions.loading = true;
                let (task, handle) = Task::perform(
                    async move {
                        client
                            .search_suggestions(&keyword)
                            .await
                            .map_err(Into::into)
                    },
                    move |result| Message::SearchSuggestLoaded(generation, result),
                )
                .abortable();
                self.ui.search.suggestions.pending = Some(handle);
                Some(task)
            }
            Message::SearchSuggestLoaded(generation, result) => {
                let state = &mut self.ui.search.suggestions;
                if !state.accepts(*generation) {
                    return Some(Task::none());
                }
                state.loading = false;
                state.pending = None;
                match result {
                    Ok(items) => state.items = items.clone(),
                    Err(error) => {
                        tracing::debug!(%error, "Search suggestions unavailable");
                        state.items.clear();
                    }
                }
                Some(Task::none())
            }
            Message::SearchSuggestDismiss => {
                self.ui.search.suggestions.close();
                Some(Task::none())
            }
            Message::SearchSuggestMove(direction) => {
                let state = &mut self.ui.search.suggestions;
                if state.open && !state.items.is_empty() {
                    let len = state.items.len() as i32;
                    let next = state
                        .selected
                        .map(|index| index as i32 + direction)
                        .unwrap_or(if *direction > 0 { 0 } else { len - 1 });
                    let index = next.rem_euclid(len) as usize;
                    state.selected = Some(index);
                    return Some(crate::ui::components::search_bar::reveal_suggestion(index));
                }
                Some(Task::none())
            }
            Message::SearchSuggestPick(index) => Some(self.pick_search_suggestion(*index)),
            Message::SearchSuggestSongLoaded(generation, result) => {
                if *generation != self.ui.search.suggestions.generation {
                    return Some(Task::none());
                }
                match result {
                    Ok(tracks) if !tracks.is_empty() => {
                        Some(Task::done(Message::PlayNcmSong(tracks[0].clone())))
                    }
                    _ => Some(Self::toast_error("无法加载这首歌曲，请重试".to_string())),
                }
            }
            Message::SearchSubmit => {
                if self.ui.search.suggestions.open
                    && let Some(index) = self.ui.search.suggestions.selected
                {
                    return Some(self.pick_search_suggestion(index));
                }
                self.ui.search.suggestions.close();
                let Some(route) = self.route_for_message(message) else {
                    return Some(Task::none());
                };

                Some(self.navigate_to_route(route, true))
            }

            Message::SearchTabChanged(tab) => {
                if self.ui.search.active_tab == *tab {
                    return Some(Task::none());
                }

                let route = Route::Search {
                    keyword: self.ui.search.keyword.clone(),
                    tab: *tab,
                    page: 0,
                };
                Some(self.navigate_to_route(route, false))
            }

            Message::SearchResultsLoaded(payload) => {
                if !self.search_request_is_current(&payload.context) {
                    tracing::debug!(
                        "Ignoring stale search response: keyword={:?}, tab={:?}, page={}",
                        payload.context.keyword,
                        payload.context.tab,
                        payload.context.page
                    );
                    return Some(Task::none());
                }

                self.ui.search.loading = false;
                // Results can shrink between requests. Return to the last real
                // page instead of leaving an empty, unreachable page selected.
                let last_page = payload.total_count.div_ceil(PAGE_SIZE).saturating_sub(1);
                if payload.context.page > last_page {
                    self.ui.search.total_count = payload.total_count;
                    return Some(self.navigate_to_route(
                        Route::Search {
                            keyword: payload.context.keyword.clone(),
                            tab: payload.context.tab,
                            page: last_page,
                        },
                        false,
                    ));
                }
                self.clear_search_cover_cache();

                match payload.context.tab {
                    SearchTab::Songs => {
                        self.ui.search.song_views =
                            super::page_loader::convert_ncm_tracks_to_views_with_offset(
                                &payload.tracks,
                                payload.context.page as usize * PAGE_SIZE as usize,
                            );
                        self.ui.search.tracks = payload.tracks.clone();
                        self.ui.search.total_count = payload.total_count;
                    }
                    SearchTab::Artists => {
                        self.ui.search.artists = payload.artists.clone();
                        self.ui.search.total_count = payload.total_count;
                    }
                    SearchTab::Albums => {
                        self.ui.search.albums = payload.albums.clone();
                        self.ui.search.total_count = payload.total_count;
                    }
                    SearchTab::Playlists => {
                        self.ui.search.playlists = payload.playlists.clone();
                        self.ui.search.total_count = payload.total_count;
                    }
                    SearchTab::Videos => {
                        self.ui.search.videos = payload.videos.clone();
                        self.ui.search.total_count = payload.total_count;
                    }
                    SearchTab::Radios => {
                        self.ui.search.radios = payload.radios.clone();
                        self.ui.search.total_count = payload.total_count;
                    }
                };

                Some(Task::none())
            }

            Message::SearchFailed(error) => {
                if !self.search_request_is_current(&error.context) {
                    tracing::debug!(
                        "Ignoring stale search error: keyword={:?}, tab={:?}, page={}",
                        error.context.keyword,
                        error.context.tab,
                        error.context.page
                    );
                    return Some(Task::none());
                }

                self.ui.search.loading = false;
                self.ui.search.error = Some(error.error.clone());
                tracing::error!("Search failed: {}", error.error);
                Some(Self::toast_error(format!("搜索失败: {}", error.error)))
            }

            Message::SearchPageInputChanged(value) => {
                if value.len() <= 10 && value.bytes().all(|byte| byte.is_ascii_digit()) {
                    self.ui.search.page_input = value.clone();
                }
                Some(Task::none())
            }
            Message::SearchPageJump => {
                let total = self.ui.search.total_count.div_ceil(PAGE_SIZE);
                let Some(page) = parse_page_jump(&self.ui.search.page_input, total) else {
                    self.ui.search.page_input = (self.ui.search.current_page + 1).to_string();
                    return Some(Self::toast_warning(format!(
                        "请输入 1–{} 之间的页码",
                        total.max(1)
                    )));
                };
                self.ui.search.page_input = (page + 1).to_string();
                Some(
                    self.handle_search(&Message::SearchPageChanged(page))
                        .unwrap_or_else(Task::none),
                )
            }
            Message::SearchPageChanged(page) => {
                if self.ui.search.loading || *page >= self.ui.search.total_count.div_ceil(PAGE_SIZE)
                {
                    return Some(Task::none());
                }
                if self.ui.search.current_page == *page {
                    return Some(Task::none());
                }

                let route = Route::Search {
                    keyword: self.ui.search.keyword.clone(),
                    tab: self.ui.search.active_tab,
                    page: *page,
                };
                Some(self.navigate_to_route(route, false))
            }

            Message::HoverSearchSong(id) => {
                self.ui.search.song_animations.set_hovered_exclusive(*id);
                Some(Task::none())
            }

            Message::HoverSearchCard(id) => {
                self.ui.search.card_animations.set_hovered_exclusive(*id);
                Some(Task::none())
            }

            Message::PlaySearchSong(song_id) => {
                let Some(song_info) = self
                    .ui
                    .search
                    .tracks
                    .iter()
                    .find(|song| song.id == *song_id)
                    .cloned()
                else {
                    return Some(Task::none());
                };

                tracing::info!(
                    "Playing search result: {} - {}",
                    song_info.title,
                    song_info.artist_names()
                );
                Some(Task::done(Message::PlayNcmSong(song_info)))
            }

            Message::OpenSearchResult(id, tab) => {
                match tab {
                    SearchTab::Albums => {
                        tracing::info!("Open album: {}", id);
                        return Some(Task::done(Message::OpenAlbum(*id)));
                    }
                    SearchTab::Playlists => {
                        // Open NCM playlist
                        return Some(Task::done(Message::OpenNcmPlaylist(*id)));
                    }
                    SearchTab::Radios => {
                        return Some(self.navigate_to_route(Route::Podcast(*id), true));
                    }
                    SearchTab::Artists => {
                        tracing::info!("Open artist: {}", id);
                        return Some(Task::done(Message::OpenArtist(*id)));
                    }
                    _ => {}
                }
                Some(Task::none())
            }

            _ => None,
        }
    }

    /// Fetch search results from NCM API
    pub(super) fn fetch_results(
        &self,
        keyword: String,
        tab: SearchTab,
        page: u32,
    ) -> Task<Message> {
        let generation = self.ui.search.request_generation;
        let Some(client) = &self.core.ncm_client else {
            return Task::done(Message::SearchFailed(SearchErrorPayload {
                context: SearchRequestContext {
                    generation,
                    keyword,
                    tab,
                    page,
                },
                error: crate::error::AppError::new(
                    crate::error::ErrorCode::AuthenticationRequired,
                    "Please sign in to search",
                ),
            }));
        };

        let client = client.clone();
        let search_type = tab.to_search_type();
        let offset = page * PAGE_SIZE;

        Task::perform(
            async move {
                match client
                    .search(&keyword, search_type, PAGE_SIZE, offset)
                    .await
                {
                    Ok(response) => {
                        let (tracks, albums, artists, playlists, videos, radios, total_count) =
                            match search_type {
                                SearchType::Songs => (
                                    response.tracks,
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    response.track_count,
                                ),
                                SearchType::Albums => (
                                    vec![],
                                    response.albums,
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    response.album_count,
                                ),
                                SearchType::Artists => (
                                    vec![],
                                    vec![],
                                    response.artists,
                                    vec![],
                                    vec![],
                                    vec![],
                                    response.artist_count,
                                ),
                                SearchType::Playlists => (
                                    vec![],
                                    vec![],
                                    vec![],
                                    response.playlists,
                                    vec![],
                                    vec![],
                                    response.playlist_count,
                                ),
                                SearchType::Videos => (
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    response.videos,
                                    vec![],
                                    response.video_count,
                                ),
                                SearchType::Radios => (
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    response.radios,
                                    response.radio_count,
                                ),
                            };

                        Message::SearchResultsLoaded(SearchResultsPayload {
                            context: SearchRequestContext {
                                generation,
                                keyword,
                                tab,
                                page,
                            },
                            tracks,
                            albums,
                            artists,
                            playlists,
                            videos,
                            radios,
                            total_count,
                        })
                    }
                    Err(e) => Message::SearchFailed(SearchErrorPayload {
                        context: SearchRequestContext {
                            generation,
                            keyword,
                            tab,
                            page,
                        },
                        error: e.into(),
                    }),
                }
            },
            |msg| msg,
        )
    }

    fn pick_search_suggestion(&mut self, index: usize) -> Task<Message> {
        if !self.ui.search.suggestions.open {
            return Task::none();
        }
        let Some(item) = self.ui.search.suggestions.items.get(index).cloned() else {
            return Task::none();
        };
        self.ui.search.suggestions.close();
        match item.kind {
            SearchType::Songs => {
                let Some(client) = self.core.ncm_client.clone() else {
                    return Task::none();
                };
                let generation = self.ui.search.suggestions.generation;
                Task::perform(
                    async move { client.track_detail(&[item.id]).await.map_err(Into::into) },
                    move |result| Message::SearchSuggestSongLoaded(generation, result),
                )
            }
            SearchType::Artists => self.navigate_to_route(Route::Artist(item.id), true),
            SearchType::Albums => self.navigate_to_route(Route::Album(item.id), true),
            SearchType::Playlists => self.navigate_to_route(Route::NcmPlaylist(item.id), true),
            _ => Task::none(),
        }
    }

    pub(super) fn search_request_is_current(&self, context: &SearchRequestContext) -> bool {
        matches!(self.ui.current_route, Route::Search { .. })
            && self.ui.search.request_generation == context.generation
            && self.ui.search.keyword == context.keyword
            && self.ui.search.active_tab == context.tab
            && self.ui.search.current_page == context.page
    }
}

fn parse_page_jump(input: &str, total: u32) -> Option<u32> {
    input
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|page| *page > 0 && *page <= total)
        .map(|page| page - 1)
}

#[cfg(test)]
mod tests {
    use super::parse_page_jump;
    #[test]
    fn page_jump_validates_one_based_input_without_overflow() {
        assert_eq!(parse_page_jump("1", 3), Some(0));
        assert_eq!(parse_page_jump("3", 3), Some(2));
        for invalid in ["", "0", "4", "-1", "1.5", "4294967296"] {
            assert_eq!(parse_page_jump(invalid, 3), None);
        }
        assert_eq!(parse_page_jump("1", 0), None);
    }
}
