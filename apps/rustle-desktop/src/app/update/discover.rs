//! Discover page message handlers

use iced::Task;
use tracing::{debug, error};

use crate::api::{PRIVATE_RADAR_PLAYLIST_ID, PlaylistSummary};
use crate::app::message::Message;
use crate::app::state::{App, Route};
use crate::i18n::Key;

fn visible_recommended_playlists(playlists: &[PlaylistSummary]) -> Vec<PlaylistSummary> {
    playlists
        .iter()
        .filter(|playlist| playlist.id != PRIVATE_RADAR_PLAYLIST_ID)
        .cloned()
        .collect()
}

pub(super) fn is_login_only_feature_playlist(playlist_id: u64) -> bool {
    playlist_id == 0 || playlist_id == PRIVATE_RADAR_PLAYLIST_ID
}

impl App {
    pub(super) fn clear_discover_cover_cache(&mut self) {
        // Discover images are stored in the unified ImageState cache.
    }

    /// Handle discover page related messages
    pub fn handle_discover(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::RecommendedPlaylistsLoaded(generation, playlists) => {
                if *generation != self.ui.discover.load_generation {
                    return Some(Task::none());
                }
                debug!("Loaded {} recommended playlists", playlists.len());
                self.ui.discover.recommended_playlists = visible_recommended_playlists(playlists);
                self.ui.discover.recommended_loading = false;

                Some(Task::none())
            }

            Message::DailyRecommendPreviewLoaded(generation, track) => {
                if *generation != self.ui.discover.load_generation {
                    return Some(Task::none());
                }
                self.ui.discover.daily_recommend_preview = track.clone();
                Some(Task::none())
            }

            Message::PersonalFmPreviewLoaded(generation, tracks) => {
                if *generation != self.ui.discover.load_generation {
                    return Some(Task::none());
                }
                self.ui.discover.personal_fm_preview = tracks.first().cloned();
                self.ui.discover.personal_fm_prefetched_tracks = tracks.clone();
                Some(Task::none())
            }

            Message::PrivateRadarLoaded(generation, playlist) => {
                if *generation != self.ui.discover.load_generation {
                    return Some(Task::none());
                }
                self.ui.discover.private_radar = playlist.clone();
                self.ui.discover.private_radar_loading = false;
                Some(Task::none())
            }

            Message::HotPlaylistsLoaded(generation, playlists) => {
                if *generation != self.ui.discover.load_generation {
                    return Some(Task::none());
                }
                debug!("Loaded {} hot playlists", playlists.len());
                self.ui.discover.hot_playlists = playlists.clone();
                self.ui.discover.hot_loading = false;
                Some(Task::none())
            }

            Message::OfficialPlaylistsLoaded(generation, playlists) => {
                if *generation != self.ui.discover.load_generation {
                    return Some(Task::none());
                }
                debug!("Loaded {} official playlists", playlists.len());
                self.ui.discover.official_playlists = playlists.clone();
                self.ui.discover.official_loading = false;

                Some(Task::none())
            }

            Message::HoverDiscoverPlaylist(playlist_id) => {
                if let Some(id) = playlist_id {
                    self.ui
                        .discover
                        .card_animations
                        .set_hovered_exclusive(Some(*id));
                } else {
                    self.ui.discover.card_animations.set_hovered_exclusive(None);
                }
                Some(Task::none())
            }

            Message::PlayDiscoverPlaylist(playlist_id) => {
                debug!("Playing discover playlist: {}", playlist_id);
                let playlist_id = *playlist_id;

                if !self.core.is_logged_in && is_login_only_feature_playlist(playlist_id) {
                    return Some(Self::toast_warning(
                        self.core.locale.get(Key::NotLoggedIn).to_string(),
                    ));
                }

                // Load and play the playlist
                if let Some(client) = &self.core.ncm_client {
                    let client = client.clone();
                    let error_msg = self
                        .core
                        .locale
                        .get(Key::DiscoverPlaylistLoadFailed)
                        .to_string();

                    if playlist_id == 0 {
                        return Some(Task::perform(
                            async move {
                                match client.recommend_tracks().await {
                                    Ok(tracks) if !tracks.is_empty() => Some(tracks),
                                    Ok(_) => None,
                                    Err(e) => {
                                        error!("Failed to get daily recommend: {}", e);
                                        None
                                    }
                                }
                            },
                            move |tracks_opt| {
                                if let Some(tracks) = tracks_opt {
                                    Message::AddNcmPlaylist(tracks, true)
                                } else {
                                    Message::ShowErrorToast(error_msg)
                                }
                            },
                        ));
                    }

                    return Some(Task::perform(
                        async move {
                            match client.playlist_detail(playlist_id).await {
                                Ok(detail) => {
                                    // Tracks are already included in the detail
                                    if detail.tracks.is_empty() {
                                        return None;
                                    }
                                    Some((detail.id, detail.tracks))
                                }
                                Err(e) => {
                                    error!("Failed to get playlist detail: {}", e);
                                    None
                                }
                            }
                        },
                        move |detail_opt| {
                            if let Some((detail_id, tracks)) = detail_opt {
                                Message::AddNcmPlaylistWithSource(tracks, true, Some(detail_id))
                            } else {
                                Message::ShowErrorToast(error_msg)
                            }
                        },
                    ));
                }
                Some(Task::none())
            }

            Message::SeeAllRecommended => {
                let route = Route::Discover(crate::app::state::DiscoverViewMode::AllRecommended);
                Some(self.navigate_to_route(route, true))
            }

            Message::SeeAllHot => {
                let route = Route::Discover(crate::app::state::DiscoverViewMode::AllHot);
                Some(self.navigate_to_route(route, true))
            }

            Message::SeeAllOfficial => {
                let route = Route::Discover(crate::app::state::DiscoverViewMode::AllOfficial);
                Some(self.navigate_to_route(route, true))
            }

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_login_only_feature_playlist, visible_recommended_playlists};
    use crate::api::{PRIVATE_RADAR_PLAYLIST_ID, PlaylistSummary, UserSummary};

    fn playlist(id: u64, name: &str) -> PlaylistSummary {
        PlaylistSummary {
            id,
            name: name.to_string(),
            cover_url: String::new(),
            creator: UserSummary::default(),
            subscribed: false,
        }
    }

    #[test]
    fn recommendation_row_excludes_private_radar_by_id_only() {
        let playlists = vec![
            playlist(PRIVATE_RADAR_PLAYLIST_ID, "私人雷达"),
            playlist(2, "我的私人雷达歌单"),
            playlist(3, "晚间推荐"),
        ];

        let visible = visible_recommended_playlists(&playlists);

        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].id, 2);
        assert_eq!(visible[1].id, 3);
    }

    #[test]
    fn daily_and_private_radar_require_login() {
        assert!(is_login_only_feature_playlist(0));
        assert!(is_login_only_feature_playlist(PRIVATE_RADAR_PLAYLIST_ID));
        assert!(!is_login_only_feature_playlist(42));
    }
}
