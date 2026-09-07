//! Foobar2000 ESLyric format parser
//!
//! ESLrc is a word-level lyrics format used by the ESLyric plugin for Foobar2000.
//! It uses standard LRC timestamps but interleaves them with words.
//! Format: [mm:ss.xx]word[mm:ss.xx]word[mm:ss.xx]...
//! Each timestamp marks the END time of the preceding word.

#[cfg(test)]
use super::lrc;
use crate::domain::lyrics::{LyricLineOwned, LyricWordOwned, process_lyrics};

/// Parse a single ESLrc line
fn parse_line(src: &str) -> Option<LyricLineOwned> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }

    let mut result = LyricLineOwned::default();
    let mut pos = 0;
    let mut current_start_time: Option<u64> = None;

    while pos < src.len() {
        // Try to parse a timestamp
        if src[pos..].starts_with('[')
            && let Some(bracket_end) = src[pos..].find(']')
        {
            let time_str = &src[pos..pos + bracket_end + 1];

            // Try to parse as LRC timestamp
            if let Some((_, time)) = parse_lrc_time(time_str) {
                if current_start_time.is_some() {
                    // This timestamp is the end time of the previous word
                    if let Some(last_word) = result.words.last_mut() {
                        last_word.end_time = time;
                    }
                }
                current_start_time = Some(time);
                pos += bracket_end + 1;
                continue;
            }
        }

        // Find the next timestamp or end of string
        let word_end = src[pos..].find('[').map(|i| pos + i).unwrap_or(src.len());
        let word_text = &src[pos..word_end];

        if !word_text.is_empty()
            && let Some(start) = current_start_time
        {
            result.words.push(LyricWordOwned {
                start_time: start,
                end_time: 0, // Will be set by next timestamp
                word: word_text.to_string(),
                roman_word: String::new(),
            });
        }

        pos = word_end;
    }

    if result.words.is_empty() {
        return None;
    }

    Some(result)
}

/// Parse LRC timestamp and return (consumed_bytes, time_ms)
fn parse_lrc_time(src: &str) -> Option<(usize, u64)> {
    if !src.starts_with('[') {
        return None;
    }

    let end_bracket = src.find(']')?;
    let time_str = &src[1..end_bracket];

    // Skip metadata tags
    if time_str.contains(':')
        && let Some(first_char) = time_str.chars().next()
        && first_char.is_alphabetic()
    {
        return None;
    }

    let parts: Vec<&str> = time_str.split([':', '.']).collect();

    let time_ms = match parts.len() {
        2 => {
            let min: u64 = parts[0].parse().ok()?;
            let sec: u64 = parts[1].parse().ok()?;
            min * 60 * 1000 + sec * 1000
        }
        3 => {
            let min: u64 = parts[0].parse().ok()?;
            let sec: u64 = parts[1].parse().ok()?;
            let ms_str = parts[2];
            let mut ms: u64 = ms_str.parse().ok()?;

            match ms_str.len() {
                1 => ms *= 100,
                2 => ms *= 10,
                3 => {}
                _ => return None,
            }

            min * 60 * 1000 + sec * 1000 + ms
        }
        _ => return None,
    };

    Some((end_bracket + 1, time_ms))
}

/// Parse ESLrc content into lyric lines
pub fn parse_eslrc(src: &str) -> Vec<LyricLineOwned> {
    let lines = src.lines();
    let mut result = Vec::with_capacity(lines.size_hint().1.unwrap_or(128).min(1024));

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(parsed) = parse_line(trimmed) {
            result.push(parsed);
        }
    }

    process_lyrics(&mut result);

    result
}

/// Convert lyrics to ESLrc format string
#[cfg(test)]
pub fn stringify_eslrc(lines: &[LyricLineOwned]) -> String {
    let capacity: usize = lines
        .iter()
        .map(|x| x.words.iter().map(|y| y.word.len()).sum::<usize>() + 13 * x.words.len())
        .sum();
    let mut result = String::with_capacity(capacity);

    for line in lines {
        if !line.words.is_empty() {
            lrc::write_timestamp(&mut result, line.words[0].start_time);
            for word in line.words.iter() {
                result.push_str(&word.word);
                lrc::write_timestamp(&mut result, word.end_time);
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
    fn test_parse_eslrc() {
        let content = "[00:10.82]Test[00:10.97] Word[00:12.62]";
        let lines = parse_eslrc(content);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[0].word, "Test");
        assert_eq!(lines[0].words[0].start_time, 10820);
        assert_eq!(lines[0].words[0].end_time, 10970);
        assert_eq!(lines[0].words[1].word, " Word");
    }

    #[test]
    fn test_stringify_eslrc() {
        let content = "[00:10.82]Test[00:10.97] Word[00:12.62]";
        let lines = parse_eslrc(content);
        let output = stringify_eslrc(&lines);
        assert!(output.contains("[00:10.820]Test[00:10.970]"));
    }
}
