//! SubRip lyrics. Line breaks alone do not identify translation or romanization.

use super::{LyricLineOwned, LyricWordOwned, timing::finish_timed_lines};

pub(super) fn parse_clock(value: &str) -> Option<u64> {
    let (whole, fraction) = value.trim().split_once(['.', ','])?;
    if fraction.is_empty() || fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parts = whole
        .split(':')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if parts.len() != 3 || parts[1] >= 60 || parts[2] >= 60 {
        return None;
    }
    parts[0]
        .checked_mul(3_600_000)?
        .checked_add(parts[1] * 60_000)?
        .checked_add(parts[2] * 1_000)?
        .checked_add(fraction.parse::<u64>().ok()? * 10_u64.pow(3 - fraction.len() as u32))
}

pub(super) fn parse_range(line: &str) -> Option<(u64, u64)> {
    let (start, end) = line.split_once("-->")?;
    let start = parse_clock(start)?;
    let end = parse_clock(end)?;
    (end >= start).then_some((start, end))
}

pub fn parse_srt(src: &str) -> Vec<LyricLineOwned> {
    let mut result = Vec::new();
    let source_lines: Vec<_> = src.lines().collect();
    for block in source_lines.split(|line| line.trim().is_empty()) {
        let mut lines = block.iter().copied();
        let Some(first) = lines.next() else {
            continue;
        };
        let time = if first.trim().parse::<u64>().is_ok() {
            lines.next().unwrap_or("")
        } else {
            first
        };
        let Some((start_time, end_time)) = parse_range(time) else {
            continue;
        };
        let text = strip_formatting(&lines.collect::<Vec<_>>().join("\n"));
        if text.trim().is_empty() {
            continue;
        }
        result.push(LyricLineOwned {
            words: vec![LyricWordOwned {
                start_time,
                end_time,
                word: text,
                roman_word: String::new(),
            }],
            start_time,
            end_time,
            ..Default::default()
        });
    }
    finish_timed_lines(&mut result);
    result
}

fn strip_formatting(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        result.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>').map(|i| open + i) else {
            result.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = rest[open + 1..close]
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(tag.as_str(), "b" | "i" | "u" | "s" | "font") {
            result.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    result.push_str(rest);
    quick_xml::escape::unescape(&result).map_or_else(|_| result.clone(), |value| value.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_multiline_text_and_rejects_invalid_intervals() {
        let lines = parse_srt(
            "1\n00:00:01,250 --> 00:00:03.500\nHello\nworld\n\n2\n00:00:05,000 --> 00:00:04,000\ninvalid",
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words[0].word, "Hello\nworld");
        assert_eq!((lines[0].start_time, lines[0].end_time), (1250, 3500));
        assert!(parse_clock("99999999999999999:00:00.0").is_none());
    }

    #[test]
    fn whitespace_separators_and_text_formatting_are_handled() {
        let lines = parse_srt(
            "1\n00:00:01,000 --> 00:00:02,000\n<i>A &amp; B</i>\n  \n2\n00:00:03,000 --> 00:00:04,000\nC",
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words[0].word, "A & B");
    }
}
