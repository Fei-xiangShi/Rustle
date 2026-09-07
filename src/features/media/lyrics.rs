//! Lyrics discovery for local audio files
//!
//! Finds lyrics from local files (LRC, TTML, etc.) or embedded metadata.
//! Uses the `features::lyrics` module for parsing all supported formats.

use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::fs;
use std::path::{Path, PathBuf};

use crate::features::lyrics::{self, LyricLineOwned};

/// Supported lyrics file extensions
const LYRICS_EXTENSIONS: &[&str] = &[
    "lrc",  // Standard LRC
    "lqe",  // Lyricify Quick Export
    "yrc",  // NetEase YRC
    "qrc",  // QQ Music QRC
    "lys",  // Lyricify Syllable
    "ttml", // Apple Music TTML
];

/// Find lyrics for an audio file
///
/// Priority:
/// 1. Same-name lyrics file (supports all formats: .lrc, .yrc, .qrc, .lys, .ttml)
/// 2. Embedded lyrics (USLT tag)
pub fn find_lyrics(audio_path: &Path) -> Option<Vec<LyricLineOwned>> {
    // Priority 1: Check for same-name lyrics file (any supported format)
    if let Some(lyrics_path) = find_lyrics_file(audio_path)
        && let Ok(content) = fs::read_to_string(&lyrics_path)
    {
        let mut lines = lyrics::parse_lyrics(&content);
        merge_local_translation_sidecar(&lyrics_path, &mut lines);
        if !lines.is_empty() {
            tracing::debug!("Loaded {} lyrics lines from {:?}", lines.len(), lyrics_path);
            return Some(lines);
        }
    }

    // Priority 2: Check embedded lyrics
    if let Some(embedded) = extract_embedded_lyrics(audio_path) {
        let lines = lyrics::parse_lyrics(&embedded);
        if !lines.is_empty() {
            tracing::debug!(
                "Loaded {} embedded lyrics lines from {:?}",
                lines.len(),
                audio_path
            );
            return Some(lines);
        }

        // Plain text lyrics without timestamps - create single line
        if !embedded.trim().is_empty() {
            return Some(vec![LyricLineOwned {
                words: vec![lyrics::LyricWordOwned {
                    start_time: 0,
                    end_time: u64::MAX,
                    word: embedded,
                    roman_word: String::new(),
                }],
                start_time: 0,
                end_time: u64::MAX,
                ..Default::default()
            }]);
        }
    }

    None
}

/// Find lyrics file with same name as audio file
/// Searches for all supported extensions
fn find_lyrics_file(audio_path: &Path) -> Option<PathBuf> {
    let parent = audio_path.parent()?;
    let stem = audio_path.file_stem()?.to_str()?;

    for ext in LYRICS_EXTENSIONS {
        // Try lowercase extension
        let path = parent.join(format!("{}.{}", stem, ext));
        if path.exists() {
            return Some(path);
        }

        // Try uppercase extension
        let path = parent.join(format!("{}.{}", stem, ext.to_uppercase()));
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn find_sidecar_file(base_path: &Path, extension: &str) -> Option<PathBuf> {
    let parent = base_path.parent()?;
    let stem = base_path.file_stem()?.to_str()?;

    for ext in [extension.to_string(), extension.to_uppercase()] {
        let path = parent.join(format!("{}.{}", stem, ext));
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn merge_local_translation_sidecar(lyrics_path: &Path, lines: &mut [LyricLineOwned]) {
    if lines.is_empty() {
        return;
    }

    let Some(tlrc_path) = find_sidecar_file(lyrics_path, "tlrc") else {
        return;
    };

    let Ok(trans_content) = fs::read_to_string(&tlrc_path) else {
        return;
    };

    let trans_lines = lyrics::parse_lrc_sidecar(&trans_content);
    if trans_lines.is_empty() {
        return;
    }

    lyrics::merge_translation(lines, &trans_lines);
    tracing::debug!(
        "Merged local translation sidecar {:?} into {:?}",
        tlrc_path,
        lyrics_path
    );
}

/// Extract embedded lyrics from audio file
fn extract_embedded_lyrics(audio_path: &Path) -> Option<String> {
    let tagged_file = Probe::open(audio_path).ok()?.read().ok()?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())?;

    // Try USLT (Unsynchronized Lyrics) first
    if let Some(lyrics) = tag.get_string(ItemKey::Lyrics)
        && !lyrics.is_empty()
    {
        return Some(lyrics.to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_lyrics_extensions() {
        assert!(LYRICS_EXTENSIONS.contains(&"lrc"));
        assert!(LYRICS_EXTENSIONS.contains(&"lqe"));
        assert!(LYRICS_EXTENSIONS.contains(&"yrc"));
        assert!(LYRICS_EXTENSIONS.contains(&"ttml"));
    }

    #[test]
    fn test_find_lyrics_merges_local_tlrc_sidecar() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::temp_dir().join(format!(
            "rustle_local_lyrics_test_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&test_dir).unwrap();

        let audio_path = test_dir.join("song.mp3");
        let lrc_path = test_dir.join("song.lrc");
        let tlrc_path = test_dir.join("song.tlrc");

        std::fs::write(&audio_path, []).unwrap();
        std::fs::write(&lrc_path, "[00:01.000]Hello\n[00:03.000]World\n").unwrap();
        std::fs::write(&tlrc_path, "[00:01.000]你好\n[00:03.000]世界\n").unwrap();

        let lines = find_lyrics(&audio_path).unwrap();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].translated_lyric, "你好");
        assert_eq!(lines[1].translated_lyric, "世界");

        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn test_find_lyrics_merges_tlrc_even_when_some_inline_translations_exist() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::temp_dir().join(format!(
            "rustle_local_lyrics_mixed_test_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&test_dir).unwrap();

        let audio_path = test_dir.join("song.mp3");
        let lrc_path = test_dir.join("song.lrc");
        let tlrc_path = test_dir.join("song.tlrc");

        std::fs::write(&audio_path, []).unwrap();
        std::fs::write(
            &lrc_path,
            "[00:01.000]Hello\n[00:01.000]你好\n[00:03.000]World\n",
        )
        .unwrap();
        std::fs::write(&tlrc_path, "[00:03.000]世界\n").unwrap();

        let lines = find_lyrics(&audio_path).unwrap();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].translated_lyric, "你好");
        assert_eq!(lines[1].translated_lyric, "世界");

        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn test_find_lyrics_preserves_inline_translation_when_tlrc_also_exists() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::temp_dir().join(format!(
            "rustle_local_lyrics_preserve_inline_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&test_dir).unwrap();

        let audio_path = test_dir.join("song.mp3");
        let lrc_path = test_dir.join("song.lrc");
        let tlrc_path = test_dir.join("song.tlrc");

        std::fs::write(&audio_path, []).unwrap();
        std::fs::write(
            &lrc_path,
            "[00:01.000]Hello\n[00:01.000]你好\n[00:03.000]World\n",
        )
        .unwrap();
        std::fs::write(&tlrc_path, "[00:01.000]覆盖我\n[00:03.000]世界\n").unwrap();

        let lines = find_lyrics(&audio_path).unwrap();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].translated_lyric, "你好");
        assert_eq!(lines[1].translated_lyric, "世界");

        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn test_find_lyrics_merges_user_sample_tlrc_with_metadata_and_blank_lines() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::temp_dir().join(format!(
            "rustle_local_lyrics_user_sample_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&test_dir).unwrap();

        let audio_path = test_dir.join("song.mp3");
        let lrc_path = test_dir.join("song.lrc");
        let tlrc_path = test_dir.join("song.tlrc");

        std::fs::write(&audio_path, []).unwrap();
        std::fs::write(
            &lrc_path,
            "[00:00.00] 作词 : Dept/Kelsey Kuan/Sonny Zero/clam\n\
[00:00.16] 作曲 : Dept/Griffy/Kelsey Kuan/clam\n\
[00:00.33]\n\
[00:13.26]Autumn wind feels colder now\n\
[00:15.93]Ever since you're not around\n\
[00:18.70]I'm watching leaves fall on the ground\n\
[00:21.37]And rot away alone\n",
        )
        .unwrap();
        std::fs::write(
            &tlrc_path,
            "[by:七月葡萄酸]\n\
[00:00.33]\n\
[00:13.26]秋天的风好像更冷了些\n\
[00:15.93]尤其当你离开之后\n\
[00:18.70]我静静地看树叶落下\n\
[00:21.37]孤单地腐烂\n\
[00:24.02]当风起时\n\
[00:25.75]落叶成堆\n",
        )
        .unwrap();

        let lines = find_lyrics(&audio_path).unwrap();

        assert_eq!(lines.len(), 6);
        assert_eq!(lines[2].translated_lyric, "秋天的风好像更冷了些");
        assert_eq!(lines[3].translated_lyric, "尤其当你离开之后");
        assert_eq!(lines[4].translated_lyric, "我静静地看树叶落下");
        assert_eq!(lines[5].translated_lyric, "孤单地腐烂");

        std::fs::remove_dir_all(&test_dir).unwrap();
    }
}
