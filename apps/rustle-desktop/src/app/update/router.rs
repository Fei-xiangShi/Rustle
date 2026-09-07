// src/app/update/router.rs
//! Centralized route transitions and route-driven side effects

use iced::Task;

use crate::app::message::Message;
use crate::app::state::{App, DiscoverViewMode, NavigationEntry, Route, SearchTab};
use crate::ui::components::{LibraryItem, NavItem};

impl App {
    pub(super) fn sync_audio_analysis_state(&self) {
        let enabled = matches!(self.ui.current_route, Route::AudioEngine)
            && !self.core.is_window_hidden()
            && !self.core.settings.display.power_saving_mode;
        self.set_audio_analysis_enabled(enabled);
    }

    fn close_route_overlays(&mut self) {
        if self.ui.lyrics.is_open {
            self.ui.lyrics.is_open = false;
            self.ui.lyrics.animation.stop();
            self.ui.lyrics.pending_viewport_size = None;
            self.ui.lyrics.last_update = None;
        }
    }

    fn reset_route_transient_state(&mut self) {
        self.ui.playlist_page.search_expanded = false;
        self.ui.playlist_page.search_query.clear();
        self.ui.playlist_page.song_animations.cleanup_completed();
        self.ui.playlist_page.description_expanded = false;
    }

    fn clear_playlist_route_markers(&mut self) {
        self.ui.playlist_page.current = None;
        self.ui.playlist_page.viewing_recently_played = false;
    }

    fn sync_route_state(&mut self, route: &Route) -> bool {
        let previous_route = self.ui.current_route.clone();
        self.close_route_overlays();
        self.reset_route_transient_state();

        if previous_route != *route {
            self.ui.image_state.cancel_pending_and_inflight();
            self.ui.smooth_scroll.cancel_all();

            if route.has_gradient_background() {
                self.ui.playlist_page.reset_gradient_animation();
            } else {
                self.ui.playlist_page.reset_gradient_to_background();
            }
        }

        if matches!(previous_route, Route::Search { .. }) && !matches!(route, Route::Search { .. })
        {
            self.clear_search_cover_cache();
        }

        if matches!(previous_route, Route::Discover(_)) && !matches!(route, Route::Discover(_)) {
            self.clear_discover_cover_cache();
        }

        let should_reload_search = self.should_reload_search(route);
        self.ui.current_route = route.clone();

        match route {
            Route::Discover(mode) => {
                self.ui.search.keyword.clear();
                self.clear_playlist_route_markers();
                self.ui.discover.view_mode = *mode;
            }
            Route::Radio => {
                self.ui.search.keyword.clear();
                self.clear_playlist_route_markers();
            }
            Route::Downloads => {
                self.ui.search.keyword.clear();
                self.clear_playlist_route_markers();
            }
            Route::Settings(section) => {
                self.ui.search.keyword.clear();
                self.clear_playlist_route_markers();
                self.ui.active_settings_section = *section;
            }
            Route::AudioEngine => {
                self.ui.search.keyword.clear();
                self.clear_playlist_route_markers();
            }
            Route::Playlist(_)
            | Route::NcmPlaylist(_)
            | Route::User(_)
            | Route::Artist(_)
            | Route::Album(_) => {
                self.ui.search.keyword.clear();
                self.ui.playlist_page.viewing_recently_played = false;
            }
            Route::RecentlyPlayed => {
                self.ui.search.keyword.clear();
                self.ui.playlist_page.current = None;
                self.ui.playlist_page.viewing_recently_played = true;
                self.ui.playlist_page.scroll_state.borrow_mut().jump_to(0.0);
            }
            Route::Search { keyword, tab, page } => {
                self.clear_playlist_route_markers();
                self.ui.search.keyword = keyword.clone();
                self.ui.search.active_tab = *tab;
                self.ui.search.current_page = *page;
                self.ui.search.loading = should_reload_search;
                self.ui.search_query = keyword.clone();
                if should_reload_search {
                    self.ui.search.tracks.clear();
                    self.ui.search.artists.clear();
                    self.ui.search.albums.clear();
                    self.ui.search.playlists.clear();
                    self.ui.search.videos.clear();
                    self.ui.search.radios.clear();
                    self.clear_search_cover_cache();
                }
            }
        }

        self.sync_audio_analysis_state();
        should_reload_search
    }

    fn route_effects(&mut self, route: &Route, should_reload_search: bool) -> Task<Message> {
        match route {
            Route::Discover(_) => {
                let load_task = if !self.ui.discover.data_loaded {
                    self.load_discover_data()
                } else {
                    Task::none()
                };
                let image_task = self.collect_discover_image_tasks();
                Task::batch([
                    iced::widget::operation::snap_to(
                        iced::widget::Id::new("discover_scroll"),
                        iced::widget::scrollable::RelativeOffset { x: 0.0, y: 0.0 },
                    ),
                    load_task,
                    image_task,
                ])
            }
            Route::Radio => Task::batch([
                iced::widget::operation::snap_to(
                    iced::widget::Id::new("discover_scroll"),
                    iced::widget::scrollable::RelativeOffset { x: 0.0, y: 0.0 },
                ),
                self.collect_discover_image_tasks(),
                self.start_personal_fm_route(),
            ]),
            Route::Downloads => iced::widget::operation::snap_to(
                iced::widget::Id::new("downloads_scroll"),
                iced::widget::scrollable::RelativeOffset { x: 0.0, y: 0.0 },
            ),
            Route::Settings(section) => {
                self.refresh_cache_stats();
                let target_y = self.section_scroll_position(*section);
                let scroll = iced::widget::operation::scroll_to(
                    iced::widget::Id::new("settings_scroll"),
                    iced::widget::scrollable::AbsoluteOffset {
                        x: Some(0.0),
                        y: Some(target_y),
                    },
                );
                // Always re-measure on entry in case content changed (login state, etc.)
                scroll
                    .chain(self.settings_scroll_offset_task())
                    .chain(Task::done(Message::MeasureSectionPositions))
            }
            Route::AudioEngine => iced::widget::operation::snap_to(
                iced::widget::Id::new("audio_engine_scroll"),
                iced::widget::scrollable::RelativeOffset { x: 0.0, y: 0.0 },
            ),
            Route::Playlist(id) => self.open_local_playlist_route(*id),
            Route::NcmPlaylist(id) => self.open_ncm_playlist_route(*id),
            Route::User(id) => Task::batch([
                iced::widget::operation::snap_to(
                    iced::widget::Id::new("playlist_scroll"),
                    iced::widget::scrollable::RelativeOffset { x: 0.0, y: 0.0 },
                ),
                self.open_user_route(*id),
            ]),
            Route::Artist(id) => self.open_artist_route(*id),
            Route::Album(id) => self.open_album_route(*id),
            Route::RecentlyPlayed => {
                if let Some(db) = &self.core.db {
                    let db = db.clone();
                    Task::perform(
                        async move {
                            match db.get_recently_played(200).await {
                                Ok(songs) => Message::RecentlyPlayedLoaded(songs),
                                Err(e) => {
                                    tracing::error!("Failed to load recently played: {}", e);
                                    Message::Noop
                                }
                            }
                        },
                        |msg| msg,
                    )
                } else {
                    Task::none()
                }
            }
            Route::Search { keyword, tab, page } => {
                self.ui.search.scroll_state.borrow_mut().jump_to(0.0);
                let fetch_task = if should_reload_search {
                    self.fetch_results(keyword.clone(), *tab, *page)
                } else {
                    Task::none()
                };
                Task::batch([
                    iced::widget::operation::snap_to(
                        iced::widget::Id::new("search_scroll"),
                        iced::widget::scrollable::RelativeOffset { x: 0.0, y: 0.0 },
                    ),
                    fetch_task,
                ])
            }
        }
    }
    pub(super) fn route_for_message(&self, message: &Message) -> Option<Route> {
        match message {
            Message::Navigate(nav) => Some(match nav {
                NavItem::Home => Route::Discover(DiscoverViewMode::Overview),
                NavItem::Radio => Route::Radio,
                NavItem::Downloads => Route::Downloads,
                NavItem::Settings => Route::Settings(self.ui.active_settings_section),
                NavItem::AudioEngine => Route::AudioEngine,
            }),
            Message::LibrarySelect(LibraryItem::RecentlyPlayed) => Some(Route::RecentlyPlayed),
            Message::OpenSettings | Message::OpenSettingsWithCloseLyrics => {
                Some(Route::Settings(self.ui.active_settings_section))
            }
            Message::OpenAudioEngine => Some(Route::AudioEngine),
            Message::OpenPlaylist(id) => Some(Route::Playlist(*id)),
            Message::OpenNcmPlaylist(id) => Some(Route::NcmPlaylist(*id)),
            Message::OpenUser(id) => Some(Route::User(*id)),
            Message::OpenArtist(id) => Some(Route::Artist(*id)),
            Message::OpenAlbum(id) => Some(Route::Album(*id)),
            Message::ScrollToSection(section) => Some(Route::Settings(*section)),
            Message::SearchSubmit => {
                let keyword = self.ui.search_query.trim().to_string();
                if keyword.is_empty() {
                    None
                } else {
                    Some(Route::Search {
                        keyword,
                        tab: SearchTab::Songs,
                        page: 0,
                    })
                }
            }
            Message::SeeAllRecommended => Some(Route::Discover(DiscoverViewMode::AllRecommended)),
            Message::SeeAllHot => Some(Route::Discover(DiscoverViewMode::AllHot)),
            Message::SeeAllOfficial => Some(Route::Discover(DiscoverViewMode::AllOfficial)),
            _ => None,
        }
    }

    fn should_reload_search(&self, route: &Route) -> bool {
        match route {
            Route::Search { keyword, tab, page } => {
                self.ui.search.keyword != *keyword
                    || self.ui.search.active_tab != *tab
                    || self.ui.search.current_page != *page
            }
            _ => false,
        }
    }

    pub(super) fn navigate_to_route(&mut self, route: Route, push_history: bool) -> Task<Message> {
        if push_history {
            self.ui
                .nav_history
                .push(NavigationEntry::Route(route.clone()));
        } else {
            self.ui
                .nav_history
                .replace_current(NavigationEntry::Route(route.clone()));
        }

        let should_reload_search = self.sync_route_state(&route);
        Task::batch([
            self.route_effects(&route, should_reload_search),
            self.pump_image_downloads(),
        ])
    }
}
