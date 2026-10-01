//! QQ Music QRC format parser
//!
//! QRC 逐字歌词格式 (QQ音乐)
//! 格式: [start_time,duration]word(word_start,word_duration)word(word_start,word_duration)...
//! 与 YRC 不同，QRC 的单词在时间戳之前

use rustle_domain::lyrics::{LyricLineOwned, LyricWordOwned};

use super::timing::finish_timed_lines;

fn decode_xml_entities(value: &str) -> String {
    quick_xml::escape::unescape(value)
        .unwrap_or(std::borrow::Cow::Borrowed(value))
        .into_owned()
}

/// Extract the actual QRC payload from QQ Music's XML wrapper. Some responses
/// contain unescaped quotes inside the attribute, so prefer the last quote
/// before the closing tag and fall back to a regular quoted attribute.
fn lyric_content(src: &str) -> String {
    if !src.trim_start().starts_with('<') {
        return src.to_owned();
    }
    if let Ok(root) = super::xml::parse(src.as_bytes()) {
        let mut content = None;
        root.visit(&mut |node| {
            if content.is_none() {
                content = node.attr("LyricContent").map(str::to_owned);
            }
        });
        // CDATA and ordinary XML text have already been decoded correctly.
        return content.unwrap_or_else(|| root.text());
    }
    // Some QQ responses have unescaped quotes in the attribute. Retain the
    // established tolerant extraction only when structured XML parsing fails.
    let Some(marker) = src.find("LyricContent") else {
        return src.to_owned();
    };
    let after_marker = &src[marker + "LyricContent".len()..];
    let Some(open_quote) = after_marker.find('"') else {
        return src.to_owned();
    };
    let content = &after_marker[open_quote + 1..];
    let closing = content
        .rfind("\"/>")
        .or_else(|| content.rfind("\">"))
        .or_else(|| content.find('"'));
    decode_xml_entities(closing.map_or(src, |end| &content[..end]))
}

/// Parse line timestamp: [start_time,duration]
fn parse_line_time(src: &str) -> Option<(usize, u64, u64)> {
    if !src.starts_with('[') {
        return None;
    }

    let end_bracket = src.find(']')?;
    let time_str = &src[1..end_bracket];
    let parts: Vec<&str> = time_str.split(',').collect();

    if parts.len() != 2 {
        return None;
    }

    let start_time: u64 = parts[0].parse().ok()?;
    let duration: u64 = parts[1].parse().ok()?;

    Some((end_bracket + 1, start_time, duration))
}

/// Parse word timestamp: (start_time,duration)
fn parse_word_time(src: &str) -> Option<(usize, u64, u64)> {
    if !src.starts_with('(') {
        return None;
    }

    let end_paren = src.find(')')?;
    let time_str = &src[1..end_paren];
    let parts: Vec<&str> = time_str.split(',').collect();

    if parts.len() != 2 {
        return None;
    }

    let start_time: u64 = parts[0].parse().ok()?;
    let duration: u64 = parts[1].parse().ok()?;

    Some((end_paren + 1, start_time, duration))
}

/// Parse a single word with its following timestamp
fn parse_word(src: &str) -> Option<(usize, LyricWordOwned)> {
    // An ordinary parenthesis is lyric text, not a broken timestamp.
    let (paren_pos, (time_consumed, start_time, duration)) = src
        .match_indices('(')
        .find_map(|(index, _)| parse_word_time(&src[index..]).map(|time| (index, time)))?;

    // Word text is before the timestamp
    let word_text = &src[..paren_pos];

    Some((
        paren_pos + time_consumed,
        LyricWordOwned {
            start_time,
            end_time: start_time.saturating_add(duration),
            word: word_text.to_string(),
            roman_word: String::new(),
        },
    ))
}

/// Parse words from QRC line content
fn parse_words(src: &str) -> Vec<LyricWordOwned> {
    let mut words = Vec::new();
    let mut pos = 0;

    while pos < src.len() {
        if let Some((consumed, word)) = parse_word(&src[pos..]) {
            words.push(word);
            pos += consumed;
        } else {
            // Preserve trailing punctuation after the last timed word.
            if let Some(last) = words.last_mut() {
                last.word.push_str(&src[pos..]);
            }
            break;
        }
    }

    words
}

/// Parse a single QRC line
fn parse_line(line: &str) -> Option<LyricLineOwned> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Parse line timestamp
    let (consumed, start_time, duration) = parse_line_time(line)?;

    // Parse words
    let words = parse_words(&line[consumed..]);

    if words.is_empty() {
        return None;
    }

    Some(LyricLineOwned {
        words,
        start_time,
        end_time: start_time.saturating_add(duration),
        ..Default::default()
    })
}

/// Parse QRC content into lyric lines
pub fn parse_qrc(src: &str) -> Vec<LyricLineOwned> {
    let decoded = lyric_content(src);
    let lines = decoded.lines();
    let mut result = Vec::with_capacity(lines.size_hint().1.unwrap_or(128).min(1024));

    for line in lines {
        if let Some(parsed) = parse_line(line) {
            result.push(parsed);
        }
    }

    finish_timed_lines(&mut result);

    result
}

/// Convert lyrics to QRC format string
#[cfg(test)]
pub fn stringify_qrc(lines: &[LyricLineOwned]) -> String {
    use std::fmt::Write;

    let capacity: usize = lines
        .iter()
        .map(|x| x.words.iter().map(|y| y.word.len()).sum::<usize>() + 32)
        .sum();
    let mut result = String::with_capacity(capacity);

    for line in lines {
        if !line.words.is_empty() {
            let start_time = line.words[0].start_time;
            let duration: u64 = line.words.iter().map(|x| x.end_time - x.start_time).sum();
            write!(result, "[{start_time},{duration}]").unwrap();

            for word in line.words.iter() {
                let word_start = word.start_time;
                let word_duration = word.end_time - word.start_time;
                result.push_str(&word.word);
                write!(result, "({word_start},{word_duration})").unwrap();
            }
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_parentheses_and_trailing_punctuation_are_preserved() {
        let lines = parse_qrc("[1000,4000]Hello(1500,1000)(世界)(3000,1000)！");
        assert_eq!(lines[0].words[1].word, "(世界)！");
        assert_eq!((lines[0].start_time, lines[0].end_time), (1000, 5000));
        assert_eq!(lines[0].words[1].start_time, 3000);
    }

    #[test]
    fn cdata_single_quotes_and_numeric_xml_references() {
        let cdata = parse_qrc("<QrcInfos><![CDATA[[0,1000]&amp;(0,1000)]]></QrcInfos>");
        assert_eq!(cdata[0].words[0].word, "&amp;");
        let attr = parse_qrc(
            "<QrcInfos><LyricInfo LyricContent='[0,1000]&#20320;(0,1000)\n[1000,1000]&amp;amp;(1000,1000)' /></QrcInfos>",
        );
        assert_eq!(attr.len(), 2);
        assert_eq!(attr[0].words[0].word, "你");
        assert_eq!(attr[1].words[0].word, "&amp;");
    }

    #[test]
    fn invalid_and_overflowing_timestamps_never_panic() {
        assert!(parse_qrc("[0,1000]😀(不合法)").is_empty());
        let lines = parse_qrc("[18446744073709551615,1000]词(18446744073709551615,1000)");
        assert_eq!(lines[0].end_time, rustle_domain::lyrics::MAX_LRC_TIMESTAMP);
    }

    #[test]
    fn test_parse_word() {
        let (consumed, word) = parse_word("Hello(0,500)").unwrap();
        assert_eq!(consumed, 12);
        assert_eq!(word.word, "Hello");
        assert_eq!(word.start_time, 0);
        assert_eq!(word.end_time, 500);
    }

    #[test]
    fn test_parse_qrc() {
        let content = "[0,2000]Hello(0,500) (500,100)World(600,500)!(1100,400)";
        let lines = parse_qrc(content);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 4);
        assert_eq!(lines[0].words[0].word, "Hello");
        assert_eq!(lines[0].words[1].word, " ");
    }

    #[test]
    fn test_parse_qrc_xml_container_and_entities() {
        let content = r#"<?xml version="1.0"?><QrcInfos><LyricInfo LyricContent="[0,1000]Rock &amp; Roll(0,1000)"/></QrcInfos>"#;
        let lines = parse_qrc(content);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words[0].word, "Rock & Roll");
    }

    #[test]
    fn test_stringify_qrc() {
        let content = "[0,1000]Hello(0,500)World(500,500)";
        let lines = parse_qrc(content);
        let output = stringify_qrc(&lines);
        assert!(output.contains("[0,1000]"));
        assert!(output.contains("Hello(0,500)"));
    }
}
