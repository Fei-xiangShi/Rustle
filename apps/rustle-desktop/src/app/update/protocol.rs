//! Protocol URL handler — dispatches parsed rustle:// URIs to existing app actions

use iced::Task;

use crate::app::message::Message;
use crate::app::state::{App, Route, SearchTab};
use crate::protocol::uri::{self, PlaybackCmd, PlaylistTarget, ProtocolAction};

impl App {
    /// Handle protocol URI messages from the update dispatcher
    pub(super) fn handle_protocol(&mut self, message: &Message) -> Option<Task<Message>> {
        if let Message::UriReceived(uri) = message {
            let uri = uri.clone();
            tracing::info!("Processing URI: {}", uri);
            return Some(self.dispatch_uri(&uri));
        }
        None
    }

    fn dispatch_uri(&mut self, uri: &str) -> Task<Message> {
        match uri::parse_rustle_uri(uri) {
            Ok(action) => self.dispatch_protocol_action(action),
            Err(err) => {
                tracing::warn!("Invalid URI '{}': {}", uri, err);
                Task::done(Message::ShowWarningToast(format!("无效链接: {}", err)))
            }
        }
    }

    fn dispatch_protocol_action(&mut self, action: ProtocolAction) -> Task<Message> {
        match action {
            ProtocolAction::NavigateToSong(id) => {
                // TODO: navigate to song detail page (NcmPlaylist shows song context)
                self.navigate_to_route(Route::NcmPlaylist(id), true)
            }
            ProtocolAction::NavigateToPlaylist(target) => match target {
                PlaylistTarget::Local(id) => self.navigate_to_route(Route::Playlist(id), true),
                PlaylistTarget::Ncm(id) => self.navigate_to_route(Route::NcmPlaylist(id), true),
            },
            ProtocolAction::NavigateToArtist(id) => self.navigate_to_route(Route::Artist(id), true),
            ProtocolAction::NavigateToAlbum(id) => self.navigate_to_route(Route::Album(id), true),
            ProtocolAction::Search(query) => self.navigate_to_route(
                Route::Search {
                    keyword: query,
                    tab: SearchTab::Songs,
                    page: 0,
                },
                true,
            ),
            ProtocolAction::PlaySong(id) => {
                self.invalidate_open_files_autoplay();
                let Some(client) = self.core.ncm_client.clone() else {
                    return Task::done(Message::ShowWarningToast("未登录".to_string()));
                };
                let Some(selection) = self.begin_playback_selection(false) else {
                    return Task::none();
                };
                Task::perform(
                    async move { client.track_detail(&[id]).await },
                    move |result| match result {
                        Ok(mut tracks) => {
                            tracks.truncate(1);
                            Message::PlaybackSelectionLoaded(selection, tracks)
                        }
                        Err(error) => {
                            Message::PlaybackSelectionFailed(selection, error.to_string())
                        }
                    },
                )
            }

            ProtocolAction::PlaybackControl(cmd) => {
                let msg = match cmd {
                    PlaybackCmd::Play | PlaybackCmd::Pause | PlaybackCmd::Toggle => {
                        Message::TogglePlayback
                    }
                    PlaybackCmd::Next => Message::NextSong,
                    PlaybackCmd::Previous => Message::PrevSong,
                };
                Task::done(msg)
            }
        }
    }
}
