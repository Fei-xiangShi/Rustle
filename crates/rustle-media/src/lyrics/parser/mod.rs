//! Lyrics parsing module
//!
//! Supports multiple lyrics formats:
//! - LRC: Standard line-level lyrics [mm:ss.xx]text
//! - LQE: Lyricify Quick Export container (LYS + LRC attributes)
//! - YRC: NetEase Cloud Music word-level lyrics
//! - QRC: QQ Music word-level lyrics
//! - KRC: Kugou raw and decoded LX word-level lyrics
//! - ESLrc: Foobar2000 ESLyric word-level format
//! - LYS: Lyricify Syllable format
//! - TTML: Apple Music lyrics format (XML)
//! - ASS/SSA: Subtitle text, karaoke and vocal/translation styles
//! - SRT: SubRip subtitle text and explicit cue intervals

mod ass;
mod eslrc;
mod krc;
mod lqe;
mod lrc;
mod lys;
mod qrc;
mod srt;
mod timing;
mod ttml;
mod xml;
mod yrc;

pub use rustle_domain::lyrics::*;

fn split_bracket_prefix(line: &str) -> Option<(&str, &str)> {
    if !line.starts_with('[') {
        return None;
    }

    let bracket_end = line.find(']')?;
    Some((&line[1..bracket_end], &line[bracket_end + 1..]))
}

fn is_word_timed_line_header(header: &str) -> bool {
    let (start, duration) = match header.split_once(',') {
        Some(parts) => parts,
        None => return false,
    };

    !start.is_empty()
        && !duration.is_empty()
        && start.chars().all(|ch| ch.is_ascii_digit())
        && duration.chars().all(|ch| ch.is_ascii_digit())
}

fn is_lrc_timestamp_header(header: &str) -> bool {
    let parts: Vec<&str> = header.split(':').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
                && part.chars().any(|ch| ch.is_ascii_digit())
        })
}

/// Detect lyrics format from content
pub fn detect_format(content: &str) -> LyricsFormat {
    let trimmed = content.trim_start_matches(['\u{feff}', '\0']).trim();

    if trimmed
        .lines()
        .any(|line| line.trim_start().starts_with("Dialogue:"))
    {
        return LyricsFormat::Ass;
    }
    if trimmed
        .lines()
        .any(|line| srt::parse_range(line.trim()).is_some())
    {
        return LyricsFormat::Srt;
    }
    if trimmed.lines().any(|line| krc::is_krc_line(line.trim())) {
        return LyricsFormat::Krc;
    }

    // QQ Music returns QRC inside an XML `LyricContent` attribute. Detect the
    // container before the generic XML/TTML checks so callers can feed the
    // decrypted response directly to the parser.
    if trimmed.starts_with('<')
        && (trimmed.contains("LyricContent")
            || trimmed.contains("<QrcInfos")
            || trimmed.contains("<Lyric_"))
    {
        return LyricsFormat::Qrc;
    }

    if trimmed.starts_with("[Lyricify Quick Export]")
        || trimmed.contains("[lyrics: format@Lyricify Syllable]")
    {
        return LyricsFormat::Lqe;
    }

    // TTML format: XML with <tt> root element
    if trimmed.starts_with('<')
        && xml::parse(trimmed.as_bytes()).is_ok_and(|root| root.name == "tt")
    {
        return LyricsFormat::Ttml;
    }

    // Scan for the first actual lyric line instead of assuming that the first
    // non-empty line is a timestamp. NCM YRC responses can contain JSON
    // metadata lines before the YRC payload; AMLL's parser skips those lines.
    for first_line in trimmed
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('['))
    {
        if let Some((header, after_bracket)) = split_bracket_prefix(first_line) {
            let after_bracket = after_bracket.trim_start();
            if is_word_timed_line_header(header) {
                // YRC uses a `[start,duration]` line header followed by word markers
                // in the form `(start,duration,0)text`.
                if let Some(marker) = after_bracket
                    .strip_prefix('(')
                    .and_then(|text| text.split_once(')').map(|(marker, _)| marker))
                {
                    let parts: Vec<_> = marker.split(',').collect();
                    if parts.len() == 3
                        && parts.iter().all(|part| {
                            !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit())
                        })
                    {
                        return LyricsFormat::Yrc;
                    }
                }

                // QRC also uses `[start,duration]`, but the word timing marker trails
                // the text as `word(start,duration)`.
                if after_bracket.match_indices('(').any(|(index, _)| {
                    let Some((marker, _)) = after_bracket[index + 1..].split_once(')') else {
                        return false;
                    };
                    let parts: Vec<_> = marker.split(',').collect();
                    parts.len() == 2
                        && parts.iter().all(|part| {
                            !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit())
                        })
                }) {
                    return LyricsFormat::Qrc;
                }
            }

            // LYS format: starts with [digit] property marker
            if first_line.len() >= 3
                && let Some(c) = first_line.chars().nth(1)
                && c.is_ascii_digit()
                && let Some(c2) = first_line.chars().nth(2)
                && c2 == ']'
            {
                return LyricsFormat::Lys;
            }
        }

        // ESLrc: [mm:ss.xx]text[mm:ss.xx]
        // Check if it's an ESLrc pattern (timestamps interleaved with text).
        let timestamp_count = first_line.matches('[').count();
        if timestamp_count >= 2 {
            let parts: Vec<&str> = first_line.split('[').collect();
            if parts.len() >= 3 {
                let mut is_eslrc = true;
                for part in parts.iter().skip(1) {
                    if let Some(bracket_pos) = part.find(']') {
                        let after = &part[bracket_pos + 1..];
                        if after.is_empty() && part != parts.last().unwrap() {
                            is_eslrc = false;
                            break;
                        }
                    }
                }
                if is_eslrc {
                    return LyricsFormat::EsLrc;
                }
            }
        }

        // A regular timestamped line is enough to classify the content as LRC.
        if let Some((header, _)) = split_bracket_prefix(first_line)
            && is_lrc_timestamp_header(header)
        {
            return LyricsFormat::Lrc;
        }
    }

    LyricsFormat::Unknown
}

/// Parse lyrics from string content
pub fn parse_lyrics(content: &str) -> Vec<LyricLineOwned> {
    let format = detect_format(content);
    parse_lyrics_with_format(content, format)
}

/// Parse lyrics with specified format
pub fn parse_lyrics_with_format(content: &str, format: LyricsFormat) -> Vec<LyricLineOwned> {
    let normalized = content
        .trim_matches(['\u{feff}', '\0'])
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let content = normalized.as_str();
    // Keep parser output faithful to the source. Display-time normalization
    // and timing optimization are applied to a clone in the application
    // projection, like AMLL's rawLines/processedLines pipeline.
    match format {
        LyricsFormat::Lrc => lrc::parse_lrc(content),
        LyricsFormat::Lqe => lqe::parse_lqe(content),
        LyricsFormat::Yrc => yrc::parse_yrc(content),
        LyricsFormat::Qrc => qrc::parse_qrc(content),
        LyricsFormat::Krc => krc::parse_krc(content),
        LyricsFormat::Srt => srt::parse_srt(content),
        LyricsFormat::Ass => ass::parse_ass(content),
        LyricsFormat::EsLrc => eslrc::parse_eslrc(content),
        LyricsFormat::Lys => lys::parse_lys(content),
        LyricsFormat::Ttml => match ttml::parse_ttml(content.as_bytes()) {
            Ok(ttml_lyric) => ttml_lyric.lines,
            Err(_) => Vec::new(),
        },
        LyricsFormat::Unknown => {
            // Try LRC as fallback
            lrc::parse_lrc(content)
        }
    }
}

/// Parse sidecar LRC attributes like `.tlrc` using raw line timestamps.
pub fn parse_lrc_sidecar(content: &str) -> Vec<LyricLineOwned> {
    lrc::parse_lrc(content)
}

/// Merge translation lyrics into main lyrics
pub fn merge_translation(main: &mut [LyricLineOwned], translation: &[LyricLineOwned]) {
    merge_lrc_attr(main, translation, LyricAttr::Translation);
}

/// Merge romanized lyrics into main lyrics.
pub fn merge_romanization(main: &mut [LyricLineOwned], romanization: &[LyricLineOwned]) {
    merge_lrc_attr(main, romanization, LyricAttr::Romanization);
}

#[derive(Clone, Copy)]
enum LyricAttr {
    Translation,
    Romanization,
}

fn line_anchor_time(line: &LyricLineOwned) -> u64 {
    line.start_time
}

/// Maximum start-time drift accepted when attaching line-level attributes.
///
/// SPlayer uses the same tolerance when aligning translations and romanization
/// with the main lyrics. A small tolerance is necessary because the sources
/// may round or quantize timestamps differently.
const LYRIC_ATTR_ALIGN_TOLERANCE_MS: u64 = 300;

fn merge_lrc_attr(main: &mut [LyricLineOwned], attr_lines: &[LyricLineOwned], attr: LyricAttr) {
    let mut attr_lines: Vec<_> = attr_lines.iter().collect();
    attr_lines.sort_by_key(|line| line.start_time);
    let mut main_index = 0usize;
    let mut attr_index = 0usize;

    // Walk both sorted streams in order. When a timestamp does not match,
    // advance the earlier stream so an unmatched line cannot block all later
    // translation/romanization lines.
    while main_index < main.len() && attr_index < attr_lines.len() {
        let main_time = line_anchor_time(&main[main_index]);
        let attr_time = line_anchor_time(attr_lines[attr_index]);

        if main_time.abs_diff(attr_time) <= LYRIC_ATTR_ALIGN_TOLERANCE_MS {
            let text = attr_lines[attr_index]
                .words
                .iter()
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>()
                .join("");

            let text = text.trim();
            if !text.is_empty() && text != "//" && !text.contains("作品的著作权") {
                let main_line = &mut main[main_index];
                match attr {
                    LyricAttr::Translation if main_line.translated_lyric.is_empty() => {
                        main_line.translated_lyric = text.to_owned()
                    }
                    LyricAttr::Romanization if main_line.roman_lyric.is_empty() => {
                        main_line.roman_lyric = text.to_owned()
                    }
                    _ => {}
                }
            }

            main_index += 1;
            attr_index += 1;
        } else if main_time < attr_time {
            main_index += 1;
        } else {
            attr_index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_format_reaches_the_application_projection() {
        let inputs = [
            (LyricsFormat::Lrc, "[00:01]Hello world"),
            (LyricsFormat::EsLrc, "[00:01]Hello[00:02] world[00:03]"),
            (
                LyricsFormat::Lqe,
                "[Lyricify Quick Export]\n[lyrics: format@Lyricify Syllable]\n[0]Hello(1000,500) world(1500,500)",
            ),
            (LyricsFormat::Lys, "[0]Hello(1000,500) world(1500,500)"),
            (
                LyricsFormat::Yrc,
                "[1000,2000](1000,500,0)Hello(1500,500,0) world",
            ),
            (
                LyricsFormat::Qrc,
                "[1000,2000]Hello(1000,500) world(1500,500)",
            ),
            (
                LyricsFormat::Krc,
                "[1000,2000]<0,500,0>Hello<500,500,0> world",
            ),
            (
                LyricsFormat::Ttml,
                "<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><div><p begin=\"1s\" end=\"3s\">Hello world</p></div></body></tt>",
            ),
            (
                LyricsFormat::Srt,
                "1\n00:00:01,000 --> 00:00:03,000\nHello world",
            ),
            (
                LyricsFormat::Ass,
                "[Events]\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Hello world",
            ),
        ];
        for (format, content) in inputs {
            let raw = parse_lyrics_with_format(content, format);
            assert!(!raw.is_empty(), "{format:?}");
            assert_eq!(parse_lyrics(content), raw, "auto detection: {format:?}");
            let projected = rustle_application::lyrics::project_lyrics(raw);
            assert_eq!(projected[0].text, "Hello world", "{format:?}");
            assert!(projected[0].end_ms > projected[0].start_ms, "{format:?}");
        }
    }

    #[test]
    fn translation_uses_source_line_start_not_first_sung_word() {
        let mut main = parse_lyrics("[1000,3000](1700,1000,0)Main");
        merge_translation(&mut main, &parse_lyrics("[00:01.00]翻译"));
        assert_eq!(main[0].translated_lyric, "翻译");
        assert_eq!(main[0].words[0].start_time, 1700);
    }

    #[test]
    fn detects_nonzero_yrc_marker_and_qrc_cdata() {
        assert_eq!(
            detect_format("[1000,2000](1000,2000,1)Hi"),
            LyricsFormat::Yrc
        );
        assert_eq!(
            detect_format("[1000,2000]Hi(1000,0)there(1000,2000)"),
            LyricsFormat::Qrc
        );
        let lines = parse_lyrics("<QrcInfos><![CDATA[[0,1000]Hi(0,1000)]]></QrcInfos>");
        assert_eq!(lines[0].words[0].word, "Hi");
        assert_eq!(
            detect_format("<t:tt xmlns:t=\"http://www.w3.org/ns/ttml\"><t:body/></t:tt>"),
            LyricsFormat::Ttml
        );
    }

    #[test]
    fn translation_alignment_sorts_sidecars_and_ignores_placeholders() {
        let mut main = vec![
            test_line(1000, "A"),
            test_line(2000, "B"),
            test_line(3000, "C"),
        ];
        let translation = vec![
            test_line(3000, "Third"),
            test_line(1000, " First "),
            test_line(2000, "//"),
        ];
        merge_translation(&mut main, &translation);
        assert_eq!(main[0].translated_lyric, "First");
        assert!(main[1].translated_lyric.is_empty());
        assert_eq!(main[2].translated_lyric, "Third");
    }

    #[test]
    fn test_detect_lrc() {
        let content = "[00:01.12]First line\n[00:05.00]Second line";
        assert_eq!(detect_format(content), LyricsFormat::Lrc);
    }

    #[test]
    fn test_detect_yrc() {
        let content = "[0,1000](0,500,0)Hello(500,500,0)World";
        assert_eq!(detect_format(content), LyricsFormat::Yrc);
    }

    #[test]
    fn test_detect_yrc_after_ncm_metadata_lines() {
        let content =
            "{\"t\":0,\"c\":[{\"tx\":\"作词: \"}]}\n[0,1000](0,500,0)Hello(500,500,0)World";
        assert_eq!(detect_format(content), LyricsFormat::Yrc);
        assert_eq!(parse_lyrics(content)[0].words[0].word, "Hello");
    }

    #[test]
    fn test_detect_yrc_after_bracket_metadata_lines() {
        let content = "[ti:Title]\n[ar:Artist]\n[0,1000](0,500,0)Hello(500,500,0)World";
        assert_eq!(detect_format(content), LyricsFormat::Yrc);
    }

    #[test]
    fn test_detect_qrc_xml_container() {
        let content = r#"<?xml version="1.0"?><QrcInfos><LyricInfo LyricContent="[0,1000]Hi(0,1000)"/></QrcInfos>"#;
        assert_eq!(detect_format(content), LyricsFormat::Qrc);
        assert_eq!(parse_lyrics(content)[0].words[0].word, "Hi");
    }

    #[test]
    fn test_detect_lqe() {
        let content = "[Lyricify Quick Export]\n[lyrics: format@Lyricify Syllable]\n[0]Hi(0,500)";
        assert_eq!(detect_format(content), LyricsFormat::Lqe);
    }

    #[test]
    fn test_detect_parenthesized_lrc_stays_lrc() {
        let content = "[00:01.000](Se-no! Ah Ah Ah Ah)\n[00:03.000]Next";
        assert_eq!(detect_format(content), LyricsFormat::Lrc);
    }

    #[test]
    fn test_detect_lrc_with_inline_parentheses_stays_lrc() {
        let content = "[00:01.000]Hello (world)\n[00:03.000]Next";
        assert_eq!(detect_format(content), LyricsFormat::Lrc);
    }

    #[test]
    fn test_parse_lrc() {
        let content = "[00:01.12]First line\n[00:05.00]Second line";
        let lines = parse_lyrics(content);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_time, 1120);
    }

    #[test]
    fn test_parse_parenthesized_lrc_through_auto_detect_marks_background() {
        let content = "[00:01.000](Hello, world!）";
        let lines = parse_lyrics(content);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].is_bg);
        assert_eq!(lines[0].words[0].word, "Hello, world!");
    }

    #[test]
    fn test_merge_translation_matches_duplicate_timestamps_sequentially() {
        let mut main = vec![
            LyricLineOwned {
                start_time: 1000,
                words: vec![LyricWordOwned {
                    start_time: 1000,
                    end_time: 2000,
                    word: "Main A".into(),
                    roman_word: String::new(),
                }],
                ..Default::default()
            },
            LyricLineOwned {
                start_time: 1000,
                words: vec![LyricWordOwned {
                    start_time: 1000,
                    end_time: 2000,
                    word: "Main B".into(),
                    roman_word: String::new(),
                }],
                ..Default::default()
            },
        ];
        let translation = vec![LyricLineOwned {
            start_time: 1000,
            words: vec![LyricWordOwned {
                start_time: 1000,
                end_time: 2000,
                word: "Trans".into(),
                roman_word: String::new(),
            }],
            ..Default::default()
        }];

        merge_translation(&mut main, &translation);

        assert_eq!(main[0].translated_lyric, "Trans");
        assert!(main[1].translated_lyric.is_empty());
    }

    #[test]
    fn test_merge_translation_preserves_existing_inline_text() {
        let mut main = vec![
            LyricLineOwned {
                start_time: 1000,
                translated_lyric: "Inline".into(),
                words: vec![LyricWordOwned {
                    start_time: 1000,
                    end_time: 2000,
                    word: "Main A".into(),
                    roman_word: String::new(),
                }],
                ..Default::default()
            },
            LyricLineOwned {
                start_time: 2000,
                words: vec![LyricWordOwned {
                    start_time: 2000,
                    end_time: 3000,
                    word: "Main B".into(),
                    roman_word: String::new(),
                }],
                ..Default::default()
            },
        ];
        let translation = vec![
            LyricLineOwned {
                start_time: 1000,
                words: vec![LyricWordOwned {
                    start_time: 1000,
                    end_time: 2000,
                    word: "Sidecar A".into(),
                    roman_word: String::new(),
                }],
                ..Default::default()
            },
            LyricLineOwned {
                start_time: 2000,
                words: vec![LyricWordOwned {
                    start_time: 2000,
                    end_time: 3000,
                    word: "Sidecar B".into(),
                    roman_word: String::new(),
                }],
                ..Default::default()
            },
        ];

        merge_translation(&mut main, &translation);

        assert_eq!(main[0].translated_lyric, "Inline");
        assert_eq!(main[1].translated_lyric, "Sidecar B");
    }

    fn test_line(start_time: u64, text: &str) -> LyricLineOwned {
        LyricLineOwned {
            start_time,
            words: vec![LyricWordOwned {
                start_time,
                end_time: start_time + 1000,
                word: text.into(),
                roman_word: String::new(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_merge_translation_allows_timestamp_drift_and_skips_extra_lines() {
        let mut main = vec![test_line(1000, "Main A"), test_line(2000, "Main B")];
        let translation = vec![
            test_line(1200, "Trans A"),
            test_line(1500, "Extra translation"),
            test_line(2200, "Trans B"),
        ];

        merge_translation(&mut main, &translation);

        assert_eq!(main[0].translated_lyric, "Trans A");
        assert_eq!(main[1].translated_lyric, "Trans B");
    }

    #[test]
    fn test_merge_translation_skips_main_lines_without_translation() {
        let mut main = vec![
            test_line(1000, "Main A"),
            test_line(1500, "Main without translation"),
            test_line(2000, "Main B"),
        ];
        let translation = vec![test_line(1000, "Trans A"), test_line(2000, "Trans B")];

        merge_translation(&mut main, &translation);

        assert_eq!(main[0].translated_lyric, "Trans A");
        assert!(main[1].translated_lyric.is_empty());
        assert_eq!(main[2].translated_lyric, "Trans B");
    }
}
