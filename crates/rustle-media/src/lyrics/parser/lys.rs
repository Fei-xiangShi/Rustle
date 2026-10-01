//! Lyricify Syllable (LYS) format parser
//!
//! LYS is a word-level lyrics format used by Lyricify.
//! Format: [property]word(start_time,duration)word(start_time,duration)...
//! Property values:
//! - 0, 1: Normal line
//! - 2, 5: Duet line
//! - 6, 7: Background line
//! - 8: Background + Duet line

use rustle_domain::lyrics::{LyricLineOwned, LyricWordOwned, process_lyrics};

/// Parse property marker: [digit]
fn parse_property(src: &str) -> Option<(usize, bool, bool)> {
    if !src.starts_with('[') {
        return None;
    }

    let end_bracket = src.find(']')?;
    if end_bracket != 2 {
        return None;
    }

    let prop_char = src.chars().nth(1)?;
    if !prop_char.is_ascii_digit() {
        return None;
    }

    let prop: u8 = prop_char.to_digit(10)? as u8;

    let (is_bg, is_duet) = match prop {
        0 | 1 => (false, false),
        2 | 5 => (false, true),
        3 | 4 => (false, false),
        6 | 7 => (true, false),
        8 => (true, true),
        _ => (false, false),
    };

    Some((3, is_bg, is_duet))
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

/// Parse a single word with its following timestamp (same as QRC)
fn parse_word(src: &str) -> Option<(usize, LyricWordOwned)> {
    let paren_pos = src
        .match_indices('(')
        .find_map(|(index, _)| parse_word_time(&src[index..]).map(|_| index))?;
    let word_text = &src[..paren_pos];
    let (time_consumed, start_time, duration) = parse_word_time(&src[paren_pos..])?;

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

/// Parse words from LYS line content
fn parse_words(src: &str) -> Vec<LyricWordOwned> {
    let mut words = Vec::new();
    let mut pos = 0;

    while pos < src.len() {
        if let Some((consumed, word)) = parse_word(&src[pos..]) {
            words.push(word);
            pos += consumed;
        } else {
            if let Some(last) = words.last_mut() {
                last.word.push_str(&src[pos..]);
            }
            break;
        }
    }

    words
}

/// Parse a single LYS line
fn parse_line(line: &str) -> Option<LyricLineOwned> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Parse property marker
    let (consumed, is_bg, is_duet) = parse_property(line)?;

    // Parse words
    let words = parse_words(&line[consumed..]);

    if words.is_empty() {
        return None;
    }

    Some(LyricLineOwned {
        words,
        is_bg,
        is_duet,
        ..Default::default()
    })
}

/// Parse LYS content into lyric lines
pub fn parse_lys(src: &str) -> Vec<LyricLineOwned> {
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

/// Convert lyrics to LYS format string
#[cfg(test)]
pub fn stringify_lys(lines: &[LyricLineOwned]) -> String {
    use std::fmt::Write;

    let capacity: usize = lines
        .iter()
        .map(|x| x.words.iter().map(|y| y.word.len()).sum::<usize>() + 32)
        .sum();
    let mut result = String::with_capacity(capacity);

    for line in lines {
        if !line.words.is_empty() {
            let prop = match (line.is_bg, line.is_duet) {
                (false, false) => "[0]",
                (false, true) => "[2]",
                (true, false) => "[6]",
                (true, true) => "[8]",
            };
            result.push_str(prop);

            for word in line.words.iter() {
                let start_time = word.start_time;
                let duration = word.end_time - word.start_time;
                result.push_str(&word.word);
                write!(result, "({start_time},{duration})").unwrap();
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
    fn test_parse_property() {
        assert_eq!(parse_property("[0]"), Some((3, false, false)));
        assert_eq!(parse_property("[2]"), Some((3, false, true)));
        assert_eq!(parse_property("[6]"), Some((3, true, false)));
        assert_eq!(parse_property("[8]"), Some((3, true, true)));
    }

    #[test]
    fn test_parse_lys() {
        let content = "[0]Test(1234,567)Word(1801,400)";
        let lines = parse_lys(content);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].is_bg);
        assert!(!lines[0].is_duet);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[0].word, "Test");
        assert_eq!(lines[0].words[0].start_time, 1234);
    }

    #[test]
    fn test_parse_lys_duet() {
        let content = "[2]Duet(0,500)Line(500,500)";
        let lines = parse_lys(content);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].is_bg);
        assert!(lines[0].is_duet);
    }

    #[test]
    fn test_parse_lys_background() {
        let content = "[8]Background(0,500)Duet(500,500)";
        let lines = parse_lys(content);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].is_bg);
        assert!(lines[0].is_duet);
    }

    #[test]
    fn test_stringify_lys() {
        let content = "[8]Test(1234,567)";
        let lines = parse_lys(content);
        let output = stringify_lys(&lines);
        assert_eq!(output, "[8]Test(1234,567)\n");
    }
}
