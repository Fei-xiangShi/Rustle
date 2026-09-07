//! Lyricify Quick Export (LQE) parser.
//!
//! translation and pronunciation sections are plain LRC blocks.

use super::{LyricLineOwned, merge_romanization, merge_translation};
use super::{lrc::parse_lrc, lys::parse_lys};

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderType {
    Lyric,
    Translation,
    Romanization,
    Unknown,
}

fn parse_header(line: &str) -> Option<HeaderType> {
    let line = line.trim();
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }

    let body = &line[1..line.len() - 1];
    let (key, _) = body.split_once(':')?;

    Some(match key {
        "lyrics" => HeaderType::Lyric,
        "translation" => HeaderType::Translation,
        "pronunciation" => HeaderType::Romanization,
        _ => HeaderType::Unknown,
    })
}

fn parse_attr_section(
    lines: &[String],
    headers: &[(usize, HeaderType)],
    section_type: HeaderType,
) -> Vec<LyricLineOwned> {
    let Some(header_index) = headers.iter().position(|(_, kind)| *kind == section_type) else {
        return Vec::new();
    };

    let start = headers[header_index].0 + 1;
    let end = headers
        .get(header_index + 1)
        .map(|(idx, _)| *idx)
        .unwrap_or(lines.len());

    parse_lrc(&lines[start..end].join("\n"))
}

/// Parse LQE content into lyric lines.
pub fn parse_lqe(src: &str) -> Vec<LyricLineOwned> {
    let lines: Vec<String> = src
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    let mut headers: Vec<(usize, HeaderType)> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| parse_header(line).map(|kind| (index, kind)))
        .collect();

    let Some(lyric_header_index) = headers
        .iter()
        .position(|(_, kind)| *kind == HeaderType::Lyric)
    else {
        return Vec::new();
    };

    headers.push((lines.len(), HeaderType::Unknown));

    let lyric_start = headers[lyric_header_index].0 + 1;
    let lyric_end = headers[lyric_header_index + 1].0;
    let mut parsed_lines = parse_lys(&lines[lyric_start..lyric_end].join("\n"));

    let translation_lines = parse_attr_section(&lines, &headers, HeaderType::Translation);
    merge_translation(&mut parsed_lines, &translation_lines);

    let roman_lines = parse_attr_section(&lines, &headers, HeaderType::Romanization);
    merge_romanization(&mut parsed_lines, &roman_lines);

    parsed_lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lqe() {
        let content = r#"
[Lyricify Quick Export]
[version:1.0]

[lyrics: format@Lyricify Syllable]
[0]Hello(1000,1000)World(2000,1000)

[translation: format@LRC]
[00:01.000]你好世界

[pronunciation: format@LRC, language@romaji]
[00:01.000]ni hao shi jie
"#;

        let lines = parse_lqe(content);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].translated_lyric, "你好世界");
        assert_eq!(lines[0].roman_lyric, "ni hao shi jie");
    }
}
