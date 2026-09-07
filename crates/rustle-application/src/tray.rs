//! Framework-neutral system-tray presentation and command contracts.

use rustle_domain::playback::PlayMode;

/// Runtime availability of the native tray recovery surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAvailability {
    Starting,
    Available,
    Unavailable(String),
}

impl TrayAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Commands emitted by native tray adapters.
#[derive(Debug, Clone)]
pub enum TrayCommand {
    Window(TrayWindowCommand),
    PlayPause,
    NextTrack,
    PrevTrack,
    SetPlayMode(PlayMode),
    ToggleFavorite,
    Quit,
    AvailabilityChanged(TrayAvailability),
}

#[derive(Debug, Clone, Copy)]
pub enum TrayWindowCommand {
    PrimaryActivation,
    Toggle,
}

/// Dynamic application state needed by every native tray implementation.
#[derive(Debug, Clone, Default)]
pub struct TrayState {
    pub is_playing: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub play_mode: PlayMode,
    pub ncm_song_id: Option<u64>,
    pub is_favorited: bool,
}

/// Already-localized labels supplied by the desktop/application boundary.
#[derive(Debug, Clone, Copy)]
pub struct TrayLabels {
    pub play: &'static str,
    pub pause: &'static str,
    pub previous: &'static str,
    pub next: &'static str,
    pub favorite: &'static str,
    pub unfavorite: &'static str,
    pub play_mode: &'static str,
    pub sequential: &'static str,
    pub loop_all: &'static str,
    pub loop_one: &'static str,
    pub shuffle: &'static str,
    pub toggle_window: &'static str,
    pub quit: &'static str,
    pub not_playing: &'static str,
}

/// Complete native-tray input. Platform code performs no localization lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayPresentation {
    pub is_playing: bool,
    pub now_playing: String,
    pub tooltip: String,
    pub play_pause: &'static str,
    pub previous: &'static str,
    pub next: &'static str,
    pub favorite: &'static str,
    pub favorite_enabled: bool,
    pub is_favorited: bool,
    pub play_mode_label: &'static str,
    pub sequential: &'static str,
    pub loop_all: &'static str,
    pub loop_one: &'static str,
    pub shuffle: &'static str,
    pub toggle_window: &'static str,
    pub quit: &'static str,
    pub play_mode: PlayMode,
}

impl TrayPresentation {
    pub fn new(state: &TrayState, labels: TrayLabels) -> Self {
        let now_playing = match (&state.title, &state.artist) {
            (Some(title), Some(artist)) if !artist.is_empty() => {
                format!("♪ {title} — {artist}")
            }
            (Some(title), _) => format!("♪ {title}"),
            _ => labels.not_playing.to_string(),
        };

        Self {
            is_playing: state.is_playing,
            tooltip: format!("Rustle — {now_playing}"),
            now_playing,
            play_pause: if state.is_playing {
                labels.pause
            } else {
                labels.play
            },
            previous: labels.previous,
            next: labels.next,
            favorite: if state.is_favorited && state.ncm_song_id.is_some() {
                labels.unfavorite
            } else {
                labels.favorite
            },
            favorite_enabled: state.ncm_song_id.is_some(),
            is_favorited: state.is_favorited && state.ncm_song_id.is_some(),
            play_mode_label: labels.play_mode,
            sequential: labels.sequential,
            loop_all: labels.loop_all,
            loop_one: labels.loop_one,
            shuffle: labels.shuffle,
            toggle_window: labels.toggle_window,
            quit: labels.quit,
            play_mode: state.play_mode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: TrayLabels = TrayLabels {
        play: "Play",
        pause: "Pause",
        previous: "Previous",
        next: "Next",
        favorite: "Favorite",
        unfavorite: "Unfavorite",
        play_mode: "Mode",
        sequential: "Sequential",
        loop_all: "Loop all",
        loop_one: "Loop one",
        shuffle: "Shuffle",
        toggle_window: "Window",
        quit: "Quit",
        not_playing: "Not playing",
    };

    #[test]
    fn projection_contains_all_dynamic_and_localized_state() {
        let presentation = TrayPresentation::new(
            &TrayState {
                is_playing: true,
                title: Some("Song".to_string()),
                artist: Some("Artist".to_string()),
                play_mode: PlayMode::LoopOne,
                ncm_song_id: Some(42),
                is_favorited: true,
            },
            LABELS,
        );

        assert_eq!(presentation.now_playing, "♪ Song — Artist");
        assert_eq!(presentation.play_pause, "Pause");
        assert_eq!(presentation.favorite, "Unfavorite");
        assert!(presentation.favorite_enabled);
        assert_eq!(presentation.play_mode, PlayMode::LoopOne);
    }
}
