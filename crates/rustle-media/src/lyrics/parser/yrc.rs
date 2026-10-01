//! NetEase Cloud Music YRC format parser
//!
//! YRC is a word-level lyrics format used by NetEase Cloud Music.
//! Format: [start_time,duration](word_start,word_duration,0)word(word_start,word_duration,0)word...

use rustle_domain::lyrics::{LyricLineOwned, LyricWordOwned};

use super::timing::finish_timed_lines;

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

/// Parse word timestamp: (start_time,duration,0)
fn parse_word_time(src: &str) -> Option<(usize, u64, u64)> {
    if !src.starts_with('(') {
        return None;
    }

    let end_paren = src.find(')')?;
    let time_str = &src[1..end_paren];
    let parts: Vec<&str> = time_str.split(',').collect();

    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()))
    {
        return None;
    }

    let start_time: u64 = parts[0].parse().ok()?;
    let duration: u64 = parts[1].parse().ok()?;
    // parts[2] is always 0 in YRC format

    Some((end_paren + 1, start_time, duration))
}

/// Parse words from YRC line content
fn parse_words(src: &str) -> Vec<LyricWordOwned> {
    let mut words = Vec::new();
    let mut pos = 0;

    while pos < src.len() {
        // Try to parse word timestamp
        if let Some((consumed, start_time, duration)) = parse_word_time(&src[pos..]) {
            pos += consumed;

            // Only a complete timing marker terminates text. Parentheses in
            // the lyric are ordinary text, including those containing Unicode.
            let word_end = src[pos..]
                .match_indices('(')
                .find(|(index, _)| parse_word_time(&src[pos + index..]).is_some())
                .map(|(index, _)| pos + index)
                .unwrap_or(src.len());
            let word_text = &src[pos..word_end];

            words.push(LyricWordOwned {
                start_time,
                end_time: start_time.saturating_add(duration),
                word: word_text.to_string(),
                roman_word: String::new(),
            });

            pos = word_end;
        } else {
            // Search on character boundaries even for malformed source text.
            pos += src[pos..].chars().next().map_or(1, char::len_utf8);
        }
    }

    words
}

/// Parse a single YRC line
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

/// Parse YRC content into lyric lines
pub fn parse_yrc(src: &str) -> Vec<LyricLineOwned> {
    let lines = src.lines();
    let mut result = Vec::with_capacity(lines.size_hint().1.unwrap_or(128).min(1024));

    for line in lines {
        if let Some(parsed) = parse_line(line) {
            result.push(parsed);
        }
    }

    finish_timed_lines(&mut result);

    result
}

/// Convert lyrics to YRC format string
#[cfg(test)]
pub fn stringify_yrc(lines: &[LyricLineOwned]) -> String {
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
                write!(result, "({word_start},{word_duration},0)").unwrap();

                // Replace parentheses with Chinese equivalents (YRC requirement)
                for c in word.word.chars() {
                    match c {
                        '(' => result.push('（'),
                        ')' => result.push('）'),
                        _ => result.push(c),
                    }
                }
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
    fn parentheses_and_unicode_do_not_truncate_or_panic() {
        let lines = parse_yrc("[1000,3000](1500,1000,0)你好(和声)(2500,1000,0)世界");
        assert_eq!(lines[0].words[0].word, "你好(和声)");
        assert_eq!(lines[0].words[1].word, "世界");
        assert_eq!((lines[0].start_time, lines[0].end_time), (1000, 4000));
        assert_eq!(lines[0].words[0].start_time, 1500);
        assert!(parse_yrc("[0,1000](坏时间)中文😀").is_empty());
    }

    #[test]
    fn overflowing_timestamps_never_panic() {
        let lines = parse_yrc("[18446744073709551615,1000](18446744073709551615,1000,0)词");
        assert_eq!(lines[0].end_time, rustle_domain::lyrics::MAX_LRC_TIMESTAMP);
        assert!(parse_yrc("[0,1000](999999999999999999999,1000,0)词").is_empty());
    }

    #[test]
    fn test_parse_line_time() {
        assert_eq!(parse_line_time("[0,1000]"), Some((8, 0, 1000)));
        assert_eq!(parse_line_time("[12345,6789]"), Some((12, 12345, 6789)));
    }

    #[test]
    fn test_parse_word_time() {
        assert_eq!(parse_word_time("(0,500,0)"), Some((9, 0, 500)));
        assert_eq!(parse_word_time("(1234,567,0)"), Some((12, 1234, 567)));
    }

    #[test]
    fn test_parse_yrc() {
        let content = "[0,2000](0,500,0)Hello(500,500,0) (1000,500,0)World(1500,500,0)!";
        let lines = parse_yrc(content);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 4);
        assert_eq!(lines[0].words[0].word, "Hello");
        assert_eq!(lines[0].words[0].start_time, 0);
        assert_eq!(lines[0].words[0].end_time, 500);
    }

    #[test]
    fn test_stringify_yrc() {
        let content = "[0,1000](0,500,0)Hello(500,500,0)World";
        let lines = parse_yrc(content);
        let output = stringify_yrc(&lines);
        assert!(output.contains("[0,1000]"));
        assert!(output.contains("(0,500,0)Hello"));
    }
}
