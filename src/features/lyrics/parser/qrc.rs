//! QQ Music QRC format parser
//!
//! QRC 逐字歌词格式 (QQ音乐)
//! 格式: [start_time,duration]word(word_start,word_duration)word(word_start,word_duration)...
//! 与 YRC 不同，QRC 的单词在时间戳之前

use crate::domain::lyrics::{LyricLineOwned, LyricWordOwned, process_lyrics};

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
    // Find the timestamp position
    let paren_pos = src.find('(')?;

    // Word text is before the timestamp
    let word_text = &src[..paren_pos];

    // Parse the timestamp
    let (time_consumed, start_time, duration) = parse_word_time(&src[paren_pos..])?;

    Some((
        paren_pos + time_consumed,
        LyricWordOwned {
            start_time,
            end_time: start_time + duration,
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
    let (consumed, _start_time, _duration) = parse_line_time(line)?;

    // Parse words
    let words = parse_words(&line[consumed..]);

    if words.is_empty() {
        return None;
    }

    Some(LyricLineOwned {
        words,
        ..Default::default()
    })
}

/// Parse QRC content into lyric lines
pub fn parse_qrc(src: &str) -> Vec<LyricLineOwned> {
    let lines = src.lines();
    let mut result = Vec::with_capacity(lines.size_hint().1.unwrap_or(128).min(1024));

    for line in lines {
        if let Some(parsed) = parse_line(line) {
            result.push(parsed);
        }
    }

    process_lyrics(&mut result);

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
    fn test_stringify_qrc() {
        let content = "[0,1000]Hello(0,500)World(500,500)";
        let lines = parse_qrc(content);
        let output = stringify_qrc(&lines);
        assert!(output.contains("[0,1000]"));
        assert!(output.contains("Hello(0,500)"));
    }
}
