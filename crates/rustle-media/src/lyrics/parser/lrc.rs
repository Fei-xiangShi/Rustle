//! Standard LRC format parser
//!
//! Supports the common [mm:ss.xx]text format with line-level synchronization.

use rustle_domain::lyrics::{LyricLineOwned, LyricWordOwned, MAX_LRC_TIMESTAMP, process_lyrics};

fn parse_time_text(time_str: &str) -> Option<u64> {
    fn parse_component(part: &str) -> Option<f64> {
        if part.is_empty() {
            return None;
        }
        let value: f64 = part.parse().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        Some(value)
    }

    fn parse_fraction(part: &str) -> Option<f64> {
        if part.is_empty() {
            return None;
        }
        let frac_value: f64 = format!("0.{part}").parse().ok()?;
        if !frac_value.is_finite() || frac_value < 0.0 {
            return None;
        }
        Some(frac_value)
    }

    let (whole, dot_fraction) = match time_str.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (time_str, None),
    };

    let parts: Vec<&str> = whole.split(':').collect();
    if parts.is_empty() {
        return None;
    }

    let mut total_seconds = if dot_fraction.is_none() && parts.len() == 3 {
        // Some local LRC files use `mm:ss:ff` instead of `mm:ss.ff`.
        let minutes = parse_component(parts[0])?;
        let seconds = parse_component(parts[1])?;
        let fraction = parse_fraction(parts[2])?;
        minutes * 60.0 + seconds + fraction
    } else {
        let mut total_seconds = 0.0_f64;
        for part in &parts {
            let value = parse_component(part)?;
            total_seconds = total_seconds * 60.0 + value;
        }
        total_seconds
    };

    if let Some(fraction) = dot_fraction {
        total_seconds += parse_fraction(fraction)?;
    }

    let time_ms = (total_seconds * 1000.0).round();
    if !time_ms.is_finite() || time_ms < 0.0 {
        return None;
    }

    Some((time_ms as u64).min(MAX_LRC_TIMESTAMP))
}

/// Parse timestamp from LRC format: `[mm:ss.xx]`.
fn parse_time(src: &str) -> Option<(usize, u64)> {
    if !src.starts_with('[') {
        return None;
    }

    let end_bracket = src.find(']')?;
    let time_str = &src[1..end_bracket];

    // Skip metadata tags like [ar:Artist], [ti:Title]
    if time_str.contains(':')
        && let Some(first_char) = time_str.chars().next()
        && first_char.is_alphabetic()
    {
        return None;
    }

    let time_ms = parse_time_text(time_str)?;

    Some((end_bracket + 1, time_ms))
}

fn strip_background_text(text: &str) -> (String, bool) {
    let trimmed = text.trim();

    let mut chars = trimmed.chars();
    if let (Some(open), Some(close)) = (chars.next(), trimmed.chars().next_back()) {
        let is_open = matches!(open, '(' | '（');
        let is_close = matches!(close, ')' | '）');
        if is_open && is_close && trimmed.len() >= open.len_utf8() + close.len_utf8() {
            let inner = &trimmed[open.len_utf8()..trimmed.len() - close.len_utf8()];
            return (inner.trim().to_string(), true);
        }
    }

    (trimmed.to_string(), false)
}

fn split_trailing_background_text(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    let last_char = trimmed.chars().next_back()?;
    if !matches!(last_char, ')' | '）') {
        return None;
    }

    let mut depth = 0usize;
    let mut open_index = None;
    let mut open_len = 0usize;

    for (idx, ch) in trimmed.char_indices().rev() {
        match ch {
            ')' | '）' => depth += 1,
            '(' | '（' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    open_index = Some(idx);
                    open_len = ch.len_utf8();
                    break;
                }
            }
            _ => {}
        }
    }

    let open_index = open_index?;
    let prefix = trimmed[..open_index].trim_end();
    if prefix.is_empty() {
        return None;
    }

    let bg = trimmed[open_index + open_len..trimmed.len() - last_char.len_utf8()].trim();
    if bg.is_empty() {
        return None;
    }

    Some((prefix.to_string(), bg.to_string()))
}

fn split_line_level_attr_text(text: &str) -> (String, Option<String>) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }

    if let Some((main, bg)) = split_trailing_background_text(trimmed) {
        return (main, Some(bg));
    }

    (trimmed.to_string(), None)
}

fn set_line_text(line: &mut LyricLineOwned, text: String) {
    if let Some(first_word) = line.words.first_mut() {
        first_word.word = text;
    } else {
        line.words.push(LyricWordOwned {
            start_time: line.start_time,
            end_time: line.end_time,
            word: text,
            roman_word: String::new(),
        });
    }
}

fn expand_trailing_background_lines(lines: Vec<LyricLineOwned>) -> Vec<LyricLineOwned> {
    let mut expanded = Vec::with_capacity(lines.len());

    for line in lines {
        if line.is_bg {
            expanded.push(line);
            continue;
        }

        let text = line_text(&line);
        let Some((main_text, bg_text)) = split_trailing_background_text(&text) else {
            expanded.push(line);
            continue;
        };

        let (main_translated, bg_translated) = split_line_level_attr_text(&line.translated_lyric);
        let (main_romanized, bg_romanized) = split_line_level_attr_text(&line.roman_lyric);

        let mut main_line = line.clone();
        set_line_text(&mut main_line, main_text);
        main_line.translated_lyric = main_translated;
        main_line.roman_lyric = main_romanized;

        let mut bg_line = line;
        bg_line.is_bg = true;
        set_line_text(&mut bg_line, bg_text);
        bg_line.translated_lyric = bg_translated.unwrap_or_default();
        bg_line.roman_lyric = bg_romanized.unwrap_or_default();

        expanded.push(main_line);
        expanded.push(bg_line);
    }

    expanded
}

fn line_text(line: &LyricLineOwned) -> String {
    line.words
        .iter()
        .map(|word| word.word.as_str())
        .collect::<String>()
        .trim()
        .to_string()
}

fn text_script_mask(text: &str) -> u8 {
    let mut mask = 0u8;

    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_ascii_punctuation() || ch.is_ascii_digit() {
            continue;
        }

        mask |= match ch {
            '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}' => 1,
            '\u{3040}'..='\u{309F}' => 1 << 1,
            '\u{30A0}'..='\u{30FF}' => 1 << 2,
            '\u{AC00}'..='\u{D7AF}' => 1 << 3,
            'A'..='Z' | 'a'..='z' | '\u{00C0}'..='\u{024F}' => 1 << 4,
            '\u{0400}'..='\u{04FF}' => 1 << 5,
            _ if ch.is_alphabetic() => 1 << 6,
            _ => 0,
        };
    }

    mask
}

fn should_merge_duplicate_group(group: &[LyricLineOwned]) -> bool {
    if !(2..=3).contains(&group.len()) {
        return false;
    }

    let first_is_bg = group[0].is_bg;
    if group.iter().any(|line| line.is_bg != first_is_bg) {
        return false;
    }

    let mut seen_masks = Vec::with_capacity(group.len());
    let mut seen_texts = Vec::with_capacity(group.len());

    for line in group {
        let text = line_text(line);
        if text.is_empty() {
            return false;
        }
        if seen_texts.iter().any(|existing| existing == &text) {
            return false;
        }

        let mask = text_script_mask(&text);
        if mask == 0 {
            return false;
        }

        seen_texts.push(text);
        if !seen_masks.contains(&mask) {
            seen_masks.push(mask);
        }
    }

    seen_masks.len() >= 2
}

fn merge_duplicate_timestamp_attrs(lines: Vec<LyricLineOwned>) -> Vec<LyricLineOwned> {
    let mut merged: Vec<LyricLineOwned> = Vec::with_capacity(lines.len());
    let mut index = 0usize;

    while index < lines.len() {
        let start_time = lines[index].start_time;
        let mut end = index + 1;
        while end < lines.len() && lines[end].start_time == start_time {
            end += 1;
        }

        let group = &lines[index..end];
        if should_merge_duplicate_group(group) {
            let mut main = group[0].clone();

            if let Some(translation) = group.get(1).map(line_text)
                && main.translated_lyric.is_empty()
            {
                main.translated_lyric = translation;
            }
            if let Some(romanization) = group.get(2).map(line_text)
                && main.roman_lyric.is_empty()
            {
                main.roman_lyric = romanization;
            }

            merged.push(main);
        } else {
            merged.extend(group.iter().cloned());
        }

        index = end;
    }

    merged
}

/// Parse a single LRC line, which may have multiple timestamps
fn parse_line(line: &str) -> Vec<LyricLineOwned> {
    let mut results = Vec::new();
    let mut timestamps = Vec::new();
    let mut pos = 0;
    let line = line.trim();

    // Extract all timestamps at the beginning
    while pos < line.len() {
        if let Some((consumed, time)) = parse_time(&line[pos..]) {
            timestamps.push(time);
            pos += consumed;
        } else {
            break;
        }
    }

    if timestamps.is_empty() {
        return results;
    }

    // Get the text after all timestamps
    let (text, is_bg) = strip_background_text(&line[pos..]);

    // Create a LyricLine for each timestamp
    for start_time in timestamps {
        results.push(LyricLineOwned {
            words: vec![LyricWordOwned {
                start_time,
                end_time: 0, // Will be calculated later
                word: text.clone(),
                roman_word: String::new(),
            }],
            start_time,
            end_time: 0,
            is_bg,
            ..Default::default()
        });
    }

    results
}

/// Parse LRC content into lyric lines
pub fn parse_lrc(src: &str) -> Vec<LyricLineOwned> {
    let lines = src.lines();
    let mut result = Vec::with_capacity(lines.size_hint().1.unwrap_or(128).min(1024));
    let mut line_order: Vec<usize> =
        Vec::with_capacity(lines.size_hint().1.unwrap_or(128).min(1024));

    for (line_index, line) in lines.enumerate() {
        let parsed = parse_line(line);
        line_order.extend(std::iter::repeat_n(line_index, parsed.len()));
        result.extend(parsed);
    }

    // Sort by time but preserve source order for equal timestamps so duplicate
    // timestamp translation/romanization lines remain attached to the intended
    // main line.
    let mut indexed: Vec<(usize, LyricLineOwned)> = line_order.into_iter().zip(result).collect();
    indexed.sort_by_key(|(source_index, line)| (line.start_time, *source_index));
    result = indexed.into_iter().map(|(_, line)| line).collect();
    result = merge_duplicate_timestamp_attrs(result);
    result = expand_trailing_background_lines(result);

    // Calculate end times based on the next distinct timestamp.
    // This preserves simultaneous same-timestamp main lines instead of collapsing
    // their duration to zero.
    let mut next_distinct_start = MAX_LRC_TIMESTAMP;
    for idx in (0..result.len()).rev() {
        let end_time = next_distinct_start;

        result[idx].end_time = end_time;
        if let Some(first_word) = result[idx].words.first_mut() {
            first_word.end_time = end_time;
        }

        let previous_has_same_timestamp =
            idx > 0 && result[idx - 1].start_time == result[idx].start_time;
        if !previous_has_same_timestamp {
            next_distinct_start = result[idx].start_time;
        }
    }

    process_lyrics(&mut result);
    result.retain(|line| line.words.first().is_some_and(|word| !word.word.is_empty()));

    result
}

/// Write timestamp in LRC format
#[cfg(test)]
pub fn write_timestamp(result: &mut String, time: u64) {
    use std::fmt::Write;
    let ms = time % 1000;
    let sec = (time / 1000) % 60;
    let min = time / 60000;
    write!(result, "[{:02}:{:02}.{:03}]", min, sec, ms).unwrap();
}

/// Convert lyrics to LRC format string
#[cfg(test)]
pub fn stringify_lrc(lines: &[LyricLineOwned]) -> String {
    let visible_lines: Vec<&LyricLineOwned> =
        lines.iter().filter(|line| !line.words.is_empty()).collect();
    let capacity: usize = visible_lines
        .iter()
        .map(|x| x.words.iter().map(|y| y.word.len()).sum::<usize>() + 13)
        .sum();
    let mut result = String::with_capacity(capacity);

    for (index, line) in visible_lines.iter().enumerate() {
        write_timestamp(&mut result, line.words[0].start_time);
        let text = line
            .words
            .iter()
            .map(|word| word.word.as_str())
            .collect::<String>();
        if line.is_bg {
            result.push('(');
            result.push_str(&text);
            result.push(')');
        } else {
            result.push_str(&text);
        }
        if index + 1 < visible_lines.len() {
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_time() {
        assert_eq!(parse_time("[00:01.12]"), Some((10, 1120)));
        assert_eq!(parse_time("[00:10.254]"), Some((11, 10254)));
        assert_eq!(parse_time("[01:10.1]"), Some((9, 70100)));
        assert_eq!(parse_time("[00:00.00]"), Some((10, 0)));
        assert_eq!(parse_time("[1:2:3.123]"), Some((11, 3723123)));
        assert_eq!(parse_time("[00:14:91]"), Some((10, 14910)));
        assert_eq!(parse_time("[00:02:31]"), Some((10, 2310)));
    }

    #[test]
    fn test_parse_line() {
        let lines = parse_line("[00:01.12] test LyRiC");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_time, 1120);
        assert_eq!(lines[0].words[0].word, "test LyRiC");
    }

    #[test]
    fn test_parse_lrc_colon_fraction_keeps_credit_lines_short() {
        let lines = parse_lrc(
            "[00:00.00] 作词 : z²\n\
[00:01.00] 作曲 : z²/yume.\n\
[00:02:31]♪\n\
[00:14:91]「死ぬまで一緒だから」\n",
        );

        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].start_time, 0);
        assert_eq!(lines[0].end_time, 1000);
        assert_eq!(lines[1].start_time, 1000);
        assert_eq!(lines[1].end_time, 2310);
        assert_eq!(lines[2].start_time, 2310);
        assert_eq!(lines[2].end_time, 14910);
        assert_eq!(lines[3].start_time, 14910);
    }

    #[test]
    fn test_parse_multiple_timestamps() {
        let lines = parse_line("[00:12.50][01:30.00]Repeated line");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_time, 12500);
        assert_eq!(lines[1].start_time, 90000);
    }

    #[test]
    fn test_parse_lrc() {
        let content = "[ti:Test Song]\n[ar:Test Artist]\n[00:00.00]First line\n[00:05.00]Second line\n[00:10.00]Third line";
        let lines = parse_lrc(content);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].words[0].word, "First line");
        assert_eq!(lines[1].words[0].word, "Second line");
        assert_eq!(lines[2].words[0].word, "Third line");
    }

    #[test]
    fn test_parse_background_lines() {
        let lines = parse_lrc("[00:01.120](Hello)\n[00:03.000]（Hi）\n[00:05.000]World");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].is_bg);
        assert_eq!(lines[0].words[0].word, "Hello");
        assert!(lines[1].is_bg);
        assert_eq!(lines[1].words[0].word, "Hi");
        assert!(!lines[2].is_bg);
    }

    #[test]
    fn test_empty_lines_are_filtered_but_preserve_end_times() {
        let lines = parse_lrc(
            "[00:00.000]\n[00:01.000]   \n[00:01.120] Hello   \n[00:02.333]\n[00:03.000] World \n[00:05.000]   \n",
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words[0].word, "Hello");
        assert_eq!(lines[0].end_time, 2333);
        assert_eq!(lines[0].words[0].end_time, 2333);
        assert_eq!(lines[1].words[0].word, "World");
        assert_eq!(lines[1].end_time, 5000);
    }

    #[test]
    fn test_same_timestamp_lines_become_translation() {
        let lines = parse_lrc("[00:01.000]Hello\n[00:01.000]你好\n[00:03.000]World");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words[0].word, "Hello");
        assert_eq!(lines[0].translated_lyric, "你好");
        assert_eq!(lines[0].end_time, 3000);
        assert_eq!(lines[1].words[0].word, "World");
    }

    #[test]
    fn test_third_same_timestamp_line_becomes_romanization() {
        let lines =
            parse_lrc("[00:01.000]Hello\n[00:01.000]你好\n[00:01.000]ni hao\n[00:03.000]World");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].translated_lyric, "你好");
        assert_eq!(lines[0].roman_lyric, "ni hao");
    }

    #[test]
    fn test_same_timestamp_preserves_source_order() {
        let lines = parse_lrc("[00:01.000]你好\n[00:01.000]Hello\n[00:03.000]World");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words[0].word, "你好");
        assert_eq!(lines[0].translated_lyric, "Hello");
    }

    #[test]
    fn test_same_timestamp_same_script_lines_stay_separate() {
        let lines = parse_lrc("[00:01.000]Hello\n[00:01.000]World\n[00:03.000]After");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].translated_lyric.is_empty());
        assert!(lines[1].translated_lyric.is_empty());
        assert_eq!(lines[0].end_time, 3000);
        assert_eq!(lines[1].end_time, 3000);
    }

    #[test]
    fn test_same_timestamp_background_lines_merge_translation() {
        let lines = parse_lrc("[00:01.000](Hello)\n[00:01.000](你好)\n[00:03.000]After");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].is_bg);
        assert_eq!(lines[0].words[0].word, "Hello");
        assert_eq!(lines[0].translated_lyric, "你好");
        assert_eq!(lines[0].end_time, 3000);
    }

    #[test]
    fn test_same_timestamp_mixed_main_and_background_stay_separate() {
        let lines = parse_lrc("[00:01.000]Hello\n[00:01.000](你好)\n[00:03.000]After");
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].is_bg);
        assert!(lines[1].is_bg);
        assert!(lines[0].translated_lyric.is_empty());
        assert!(lines[1].translated_lyric.is_empty());
    }

    #[test]
    fn test_mixed_parentheses_are_background_lines() {
        let lines = parse_lrc("[00:01.000](Hello）\n[00:02.000]（World)");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].is_bg);
        assert!(lines[1].is_bg);
        assert_eq!(lines[0].words[0].word, "Hello");
        assert_eq!(lines[1].words[0].word, "World");
    }

    #[test]
    fn test_trailing_parenthesized_suffix_becomes_background_line() {
        let lines = parse_lrc("[00:01.000]No (Woo-ohh)\n[00:03.000]Next");
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].is_bg);
        assert_eq!(lines[0].words[0].word, "No");
        assert!(lines[1].is_bg);
        assert_eq!(lines[1].words[0].word, "Woo-ohh");
        assert_eq!(lines[0].start_time, lines[1].start_time);
        assert_eq!(lines[0].end_time, lines[1].end_time);
    }

    #[test]
    fn test_trailing_mixed_parentheses_suffix_becomes_background_line() {
        let lines = parse_lrc("[00:01.000]原文（bg)");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words[0].word, "原文");
        assert!(!lines[0].is_bg);
        assert_eq!(lines[1].words[0].word, "bg");
        assert!(lines[1].is_bg);
    }

    #[test]
    fn test_duplicate_timestamp_translation_survives_trailing_background_split() {
        let lines = parse_lrc("[00:01.000]No (Woo-ohh)\n[00:01.000]不 (呜哦)\n[00:03.000]Next");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].words[0].word, "No");
        assert_eq!(lines[0].translated_lyric, "不");
        assert_eq!(lines[1].words[0].word, "Woo-ohh");
        assert!(lines[1].is_bg);
        assert_eq!(lines[1].translated_lyric, "呜哦");
    }

    #[test]
    fn test_stringify_lrc() {
        let lines = parse_lrc("[00:01.12] test LyRiC\n[00:10.254] sssxxx");
        let output = stringify_lrc(&lines);
        assert!(output.contains("[00:01.120]"));
        assert!(output.contains("[00:10.254]"));
    }

    #[test]
    fn test_stringify_background_lines() {
        let lines = parse_lrc("[00:01.120](Hello)\n[00:03.000]World");
        let output = stringify_lrc(&lines);
        assert!(output.contains("[00:01.120](Hello)"));
        assert!(output.contains("[00:03.000]World"));
    }
}
