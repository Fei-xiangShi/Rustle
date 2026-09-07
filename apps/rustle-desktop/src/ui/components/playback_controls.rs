//! Application-specific playback action mapping.
//!
//! Generic control widgets live under `ui::widgets` and only receive visual
//! data plus caller-provided messages. This module owns Rustle's `PlayMode` and
//! `Message` mapping at the application boundary.

use iced::Element;

use crate::app::Message;
use crate::features::PlayMode;
use crate::ui::icons;
use crate::ui::responsive::UiTokens;
use crate::ui::widgets::{self, PlayModeButtonSize};

/// Build Rustle's play-mode control by translating domain state into the
/// generic widget's icon, label, and action inputs.
pub fn play_mode_button(
    play_mode: PlayMode,
    size: PlayModeButtonSize,
    tokens: UiTokens,
    is_fm_mode: bool,
) -> Element<'static, Message> {
    let (icon, label, action) = if is_fm_mode {
        (
            icons::RADIO,
            "私人FM",
            Message::ShowWarningToast("私人FM模式下无法更改播放模式".to_string()),
        )
    } else {
        let (icon, label) = match play_mode {
            PlayMode::Sequential => (icons::PLAY_SEQUENTIAL, "顺序播放"),
            PlayMode::LoopAll => (icons::LOOP_ALL, "列表循环"),
            PlayMode::LoopOne => (icons::LOOP_ONE, "单曲循环"),
            PlayMode::Shuffle => (icons::SHUFFLE, "随机播放"),
        };
        (icon, label, Message::CyclePlayMode)
    };

    widgets::play_mode_button::view(icon, label, size, tokens, action)
}
