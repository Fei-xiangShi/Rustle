//! Audio quality values shared by playback, storage, and service adapters.

use serde::{Deserialize, Serialize};

/// Product-level audio quality taxonomy.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, PartialOrd, Ord,
)]
#[serde(rename_all = "lowercase")]
pub enum QualityLevel {
    #[default]
    Standard,
    Higher,
    ExHigh,
    Lossless,
    HiRes,
    #[serde(rename = "jyeffect")]
    JvEffect,
    Sky,
    Dolby,
    JyMaster,
}

impl QualityLevel {
    pub fn from_api_rate(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Standard,
            1 => Self::Higher,
            2 => Self::ExHigh,
            3 => Self::Lossless,
            4 => Self::HiRes,
            5 => Self::JvEffect,
            6 => Self::Sky,
            7 => Self::Dolby,
            8 => Self::JyMaster,
            _ => return None,
        })
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Self::Standard => "128K",
            Self::Higher => "192K",
            Self::ExHigh => "320K",
            Self::Lossless => "SQ",
            Self::HiRes => "Hi-Res",
            Self::JvEffect => "臻音",
            Self::Sky => "环绕声",
            Self::Dolby => "Dolby",
            Self::JyMaster => "母带",
        }
    }

    /// Higher values represent a more premium server quality tier.
    pub fn priority(self) -> u8 {
        match self {
            Self::Standard => 0,
            Self::Higher => 1,
            Self::ExHigh => 2,
            Self::Lossless => 3,
            Self::HiRes => 4,
            Self::JvEffect => 5,
            Self::Sky => 6,
            Self::Dolby => 7,
            Self::JyMaster => 8,
        }
    }
}

/// Detect a supported audio container/codec from its leading bytes.
///
/// The returned value is the stable cache-file extension used by download and
/// streaming adapters. Detection is content-based and independent of filenames.
pub fn detect_audio_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes.starts_with(b"fLaC") {
        return Some("flac");
    }
    if bytes.starts_with(&[0xFF, 0xFB])
        || bytes.starts_with(&[0xFF, 0xFA])
        || bytes.starts_with(&[0xFF, 0xF3])
        || bytes.starts_with(&[0xFF, 0xF2])
        || bytes.starts_with(b"ID3")
    {
        return Some("mp3");
    }
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return Some("m4a");
    }
    if bytes.starts_with(b"OggS") {
        return Some(if bytes.windows(8).any(|window| window == b"OpusHead") {
            "opus"
        } else {
            "ogg"
        });
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        return Some("wav");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{QualityLevel, detect_audio_format};

    #[test]
    fn serialized_values_preserve_the_existing_manifest_contract() {
        let cases = [
            (QualityLevel::Standard, "\"standard\""),
            (QualityLevel::HiRes, "\"hires\""),
            (QualityLevel::JvEffect, "\"jyeffect\""),
            (QualityLevel::JyMaster, "\"jymaster\""),
        ];

        for (quality, expected) in cases {
            assert_eq!(serde_json::to_string(&quality).unwrap(), expected);
            assert_eq!(
                serde_json::from_str::<QualityLevel>(expected).unwrap(),
                quality
            );
        }
    }

    #[test]
    fn format_detection_uses_content_signatures() {
        assert_eq!(detect_audio_format(b"fLaCfixture"), Some("flac"));
        assert_eq!(detect_audio_format(b"OggSxxxxOpusHead"), Some("opus"));
        assert_eq!(detect_audio_format(b"xxxxftypfixture"), Some("m4a"));
        assert_eq!(detect_audio_format(b"not audio"), None);
    }
}
