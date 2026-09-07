//! Playback policy values shared by application and adapters.

use serde::{Deserialize, Serialize};

/// Play mode for playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlayMode {
    /// Play in order, stop at end.
    #[default]
    Sequential,
    /// Play in order, loop back to start.
    LoopAll,
    /// Repeat current song.
    LoopOne,
    /// Random order.
    Shuffle,
}

impl std::fmt::Display for PlayMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

impl PlayMode {
    /// Get the next play mode in cycle order.
    pub fn next(self) -> Self {
        match self {
            Self::Sequential => Self::LoopAll,
            Self::LoopAll => Self::LoopOne,
            Self::LoopOne => Self::Shuffle,
            Self::Shuffle => Self::Sequential,
        }
    }

    /// Get the legacy display name for the mode.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Sequential => "顺序播放",
            Self::LoopAll => "列表循环",
            Self::LoopOne => "单曲循环",
            Self::Shuffle => "随机播放",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PlayMode;

    #[test]
    fn serialized_values_remain_compatible() {
        assert_eq!(
            serde_json::to_string(&PlayMode::LoopOne).unwrap(),
            "\"loop_one\""
        );
        assert_eq!(
            serde_json::from_str::<PlayMode>("\"shuffle\"").unwrap(),
            PlayMode::Shuffle
        );
    }
}
