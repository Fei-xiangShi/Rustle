//! Lyrics discovery for local audio files
//!
//! Finds lyrics from local files (LRC, TTML, etc.) or embedded metadata.
//! Uses the `features::lyrics` module for parsing all supported formats.

use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, TagType};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::parser::{self as lyrics, LyricLineOwned};

/// Supported lyrics file extensions
const LYRICS_EXTENSIONS: &[&str] = &[
    "ttml", "lqe", "lys", "qrc", "krc", "yrc", "eslrc", "lrc", "ass", "ssa", "srt",
];
const MAX_LYRICS_BYTES: u64 = 4 * 1024 * 1024;

/// Find lyrics for an audio file
///
/// Priority:
/// 1. Same-name lyrics file (supports all formats: .lrc, .yrc, .qrc, .lys, .ttml)
/// 2. Embedded lyrics (USLT tag)
pub fn find_lyrics(audio_path: &Path) -> Option<Vec<LyricLineOwned>> {
    let mut best = None;
    let mut best_score = None;
    let mut plain = None;
    // Parse every candidate: a corrupt preferred file must not hide another
    // usable sidecar, and a line-only file must not hide word synchronization.
    for lyrics_path in find_lyrics_files(audio_path) {
        let Some(content) = read_lyrics_text(&lyrics_path) else {
            continue;
        };
        let extension = lyrics_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        let mut lines = lyrics::parse_lyrics(&content);
        if lines.is_empty() {
            if plain.is_none() && extension.eq_ignore_ascii_case("lrc") {
                plain = plain_lyrics(&content);
            }
            continue;
        }
        merge_local_translation_sidecar(&lyrics_path, &mut lines);
        let word_timed = has_word_timing(&lines);
        let priority = LYRICS_EXTENSIONS
            .iter()
            .position(|ext| ext.eq_ignore_ascii_case(extension))
            .unwrap_or(LYRICS_EXTENSIONS.len());
        let score = (word_timed, LYRICS_EXTENSIONS.len() - priority);
        if best_score.is_none_or(|previous| score > previous) {
            best_score = Some(score);
            best = Some(lines);
        }
    }
    if best.is_some() {
        return best;
    }

    for embedded in extract_embedded_lyrics(audio_path) {
        let lines = lyrics::parse_lyrics(&embedded);
        if !lines.is_empty() {
            let score = has_word_timing(&lines);
            if best
                .as_ref()
                .is_none_or(|previous| score && !has_word_timing(previous))
            {
                best = Some(lines);
            }
        } else if plain.is_none() {
            plain = plain_lyrics(&embedded);
        }
    }
    let mut lines = best.or(plain)?;
    merge_local_translation_sidecar(audio_path, &mut lines);
    Some(lines)
}

fn has_word_timing(lines: &[LyricLineOwned]) -> bool {
    lines.iter().any(|line| {
        line.words
            .windows(2)
            .any(|pair| pair[0].start_time != pair[1].start_time)
    })
}

fn plain_lyrics(content: &str) -> Option<Vec<LyricLineOwned>> {
    let content = content.trim_matches(['\u{feff}', '\0']).trim();
    if !content.is_empty() && !content.starts_with(['[', '<']) && !content.starts_with("Dialogue:")
    {
        return Some(vec![LyricLineOwned {
            words: vec![lyrics::LyricWordOwned {
                start_time: 0,
                end_time: lyrics::MAX_LRC_TIMESTAMP,
                word: content.to_owned(),
                roman_word: String::new(),
            }],
            start_time: 0,
            end_time: lyrics::MAX_LRC_TIMESTAMP,
            ..Default::default()
        }]);
    }
    None
}

/// Find lyrics file with same name as audio file
/// Searches for all supported extensions
fn find_lyrics_files(audio_path: &Path) -> Vec<PathBuf> {
    let Some(parent) = audio_path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_stem() == audio_path.file_stem()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        LYRICS_EXTENSIONS
                            .iter()
                            .any(|supported| ext.eq_ignore_ascii_case(supported))
                    })
        })
        .collect();
    paths.sort();
    paths
}

fn read_lyrics_text(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_LYRICS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_LYRICS_BYTES {
        return None;
    }
    if bytes.starts_with(b"krc1") {
        const KEY: &[u8; 16] = b"@Gaw^2tGQ61-\xce\xd2ni";
        let compressed: Vec<_> = bytes[4..]
            .iter()
            .enumerate()
            .map(|(i, byte)| byte ^ KEY[i % KEY.len()])
            .collect();
        bytes.clear();
        flate2::read::ZlibDecoder::new(compressed.as_slice())
            .take(MAX_LYRICS_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_LYRICS_BYTES {
            return None;
        }
    }
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        encoding_rs::UTF_16LE.decode(&bytes[2..]).0.into_owned()
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        encoding_rs::UTF_16BE.decode(&bytes[2..]).0.into_owned()
    } else {
        crate::encoding::decode_string(&bytes)
    };
    Some(
        text.trim_matches(['\u{feff}', '\0'])
            .replace("\r\n", "\n")
            .replace('\r', "\n"),
    )
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
    fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_stem() == base_path.file_stem()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
        })
}

fn merge_local_translation_sidecar(lyrics_path: &Path, lines: &mut [LyricLineOwned]) {
    if lines.is_empty() {
        return;
    }

    for extension in ["tlrc", "rlrc"] {
        let Some(path) = find_sidecar_file(lyrics_path, extension) else {
            continue;
        };
        let Some(content) = read_lyrics_text(&path) else {
            continue;
        };
        let attributes = lyrics::parse_lrc_sidecar(&content);
        if extension == "tlrc" {
            lyrics::merge_translation(lines, &attributes);
        } else {
            lyrics::merge_romanization(lines, &attributes);
        }
    }
}

/// Extract embedded lyrics from audio file
fn extract_embedded_lyrics(audio_path: &Path) -> Vec<String> {
    let tagged_file = Probe::open(audio_path).and_then(|probe| {
        probe
            .options(
                ParseOptions::new()
                    .read_properties(false)
                    .read_cover_art(false),
            )
            .read()
    });
    let Ok(tagged_file) = tagged_file else {
        return Vec::new();
    };
    let tags =
        tagged_file
            .primary_tag()
            .into_iter()
            .chain(tagged_file.tags().iter().filter(|tag| {
                Some(tag.tag_type()) != tagged_file.primary_tag().map(|tag| tag.tag_type())
            }));
    let mut candidates = Vec::new();
    for tag in tags {
        candidates.extend(tag.items().filter_map(|item| {
            let is_lyrics = matches!(item.key(), ItemKey::Lyrics | ItemKey::UnsyncLyrics);
            if !is_lyrics {
                return None;
            }
            match item.value() {
                ItemValue::Text(text) if !text.trim().is_empty() => Some(text.clone()),
                _ => None,
            }
        }));
        // Lofty 0.25 keeps nonstandard fields and binary SYLT in native companion
        // tags. Reading only generic ItemKeys silently discards these candidates.
        match tag.tag_type() {
            TagType::VorbisComments => {
                let native = lofty::ogg::tag::VorbisComments::from(tag.clone());
                candidates.extend(
                    native
                        .items()
                        .filter(|(key, _)| is_lyric_key(key))
                        .map(|(_, value)| value.to_owned()),
                );
            }
            TagType::Id3v2 => {
                use lofty::id3::v2::{
                    Frame, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame, TimestampFormat,
                };
                let native = Id3v2Tag::from(tag.clone());
                for frame in &native {
                    match frame {
                        Frame::UserText(text) if is_lyric_key(&text.description) => {
                            candidates.push(text.content.to_string())
                        }
                        Frame::Binary(binary) if frame.id_str() == "SYLT" => {
                            if let Ok(sync) =
                                SynchronizedTextFrame::parse(&binary.data, binary.flags())
                                && sync.timestamp_format == TimestampFormat::MS
                                && sync.content_type == SyncTextContentType::Lyrics
                            {
                                let mut text = String::new();
                                use std::fmt::Write;
                                for (time, word) in sync.content {
                                    if word.starts_with(['\n', '\r']) && !text.is_empty() {
                                        text.push('\n');
                                    }
                                    let _ = write!(
                                        text,
                                        "[{:02}:{:02}.{:03}]{}",
                                        time / 60_000,
                                        time / 1000 % 60,
                                        time % 1000,
                                        word.trim_start_matches(['\n', '\r'])
                                    );
                                }
                                candidates.push(text);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    candidates
}

fn is_lyric_key(key: &str) -> bool {
    let key = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    ["lyrics", "syncedlyrics", "unsyncedlyrics", "uslt"]
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestDir, write_mp3};
    use lofty::tag::{Tag, TagType};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mp3_uslt_is_read_and_timed_even_with_a_bom_and_cr_lines() {
        let dir = TestDir::new("uslt");
        let path = dir.0.join("song.mp3");
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(
            ItemKey::UnsyncLyrics,
            "\u{feff}[00:01]你好\r[00:03]世界\0".to_owned(),
        );
        write_mp3(&path, &tag);
        let lines = find_lyrics(&path).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words[0].word, "你好");
        assert_eq!(lines[1].start_time, 3000);
    }

    #[test]
    fn id3_custom_lyrics_and_binary_sylt_are_not_lost_in_generic_tags() {
        use lofty::TextEncoding;
        use lofty::{
            config::WriteOptions,
            id3::v2::{
                BinaryFrame, Frame, FrameId, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame,
                TimestampFormat,
            },
        };
        let dir = TestDir::new("native-lyrics");
        let path = dir.0.join("song.mp3");
        let mut native = Id3v2Tag::default();
        native.insert_user_text("SYNCED-LYRICS-ENG".into(), "[00:01]Hello".into());
        write_mp3(&path, &native);
        assert_eq!(find_lyrics(&path).unwrap()[0].words[0].word, "Hello");
        let sync = SynchronizedTextFrame::new(
            TextEncoding::UTF16,
            *b"eng",
            TimestampFormat::MS,
            SyncTextContentType::Lyrics,
            None,
            vec![
                (1000, "你".into()),
                (1500, "好".into()),
                (3000, "\n世界".into()),
            ],
        );
        let mut native = Id3v2Tag::default();
        native.insert(Frame::Binary(BinaryFrame::new(
            FrameId::new("SYLT").unwrap(),
            sync.as_bytes(WriteOptions::default()).unwrap(),
        )));
        write_mp3(&path, &native);
        let lines = find_lyrics(&path).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[1].start_time, 1500);
        assert_eq!(lines[1].words[0].word, "世界");
    }

    #[test]
    fn broken_preferred_file_falls_back_and_word_timing_beats_plain_lrc() {
        let dir = TestDir::new("lyric-candidates");
        let audio = dir.0.join("song.mp3");
        fs::write(audio.with_extension("ttml"), "<tt><broken").unwrap();
        fs::write(audio.with_extension("LrC"), "[00:01]line only").unwrap();
        fs::write(
            audio.with_extension("yrc"),
            "[1000,2000](1000,500,0)Hello(1500,1000,0) world",
        )
        .unwrap();
        let lines = find_lyrics(&audio).unwrap();
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[1].start_time, 1500);
        fs::remove_file(audio.with_extension("yrc")).unwrap();
        assert_eq!(find_lyrics(&audio).unwrap()[0].words[0].word, "line only");
    }

    #[test]
    fn utf16_gbk_and_encrypted_krc_files_are_decoded_before_parsing() {
        use std::io::Write;
        let dir = TestDir::new("lyric-encoding");
        let audio = dir.0.join("song.mp3");
        let text = "[00:01]你好";
        let bytes: Vec<_> = [0xff, 0xfe]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        fs::write(audio.with_extension("lrc"), bytes).unwrap();
        assert_eq!(find_lyrics(&audio).unwrap()[0].words[0].word, "你好");
        fs::write(audio.with_extension("lrc"), encoding_rs::GBK.encode(text).0).unwrap();
        assert_eq!(find_lyrics(&audio).unwrap()[0].words[0].word, "你好");

        let raw = "[1000,2000]<0,500,0>你<500,500,0>好";
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(raw.as_bytes()).unwrap();
        let key = b"@Gaw^2tGQ61-\xce\xd2ni";
        let bytes: Vec<_> = b"krc1"
            .iter()
            .copied()
            .chain(
                encoder
                    .finish()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(i, b)| b ^ key[i % 16]),
            )
            .collect();
        fs::write(audio.with_extension("krc"), bytes).unwrap();
        let lines = find_lyrics(&audio).unwrap();
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].start_time, 1000);
        assert_eq!(lines[0].words[0].word, "你");
    }

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
