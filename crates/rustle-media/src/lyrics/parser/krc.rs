//! Raw Kugou and decoded LX KRC. Word offsets are relative to the line onset.

use super::{LyricLineOwned, LyricWordOwned, timing::finish_timed_lines};
use base64::Engine;

fn header(src: &str) -> Option<(u64, Option<u64>, &str)> {
    let (value, rest) = src.strip_prefix('[')?.split_once(']')?;
    if let Some((start, duration)) = value.split_once(',') {
        let start: u64 = start.parse().ok()?;
        return Some((
            start,
            Some(start.saturating_add(duration.parse().ok()?)),
            rest,
        ));
    }
    // LX exports use integer milliseconds, so .3 means 3 ms, not 300 ms.
    let (minutes, seconds) = value.split_once(':')?;
    let (seconds, millis) = seconds.split_once(['.', ':'])?;
    let start = minutes
        .parse::<u64>()
        .ok()?
        .checked_mul(60_000)?
        .checked_add(seconds.parse::<u64>().ok()?.checked_mul(1_000)?)?
        .checked_add(millis.parse::<u64>().ok()?)?;
    Some((start, None, rest))
}

fn marker(src: &str) -> Option<(usize, u64, u64)> {
    let tail = src.strip_prefix('<')?;
    let length = tail
        .bytes()
        .take_while(|b| b.is_ascii_digit() || *b == b',')
        .take(64)
        .count();
    if tail.as_bytes().get(length) != Some(&b'>') {
        return None;
    }
    let value = &tail[..length];
    let parts = value
        .split(',')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    Some((value.len() + 2, parts[0], parts[1]))
}

pub(super) fn is_krc_line(src: &str) -> bool {
    header(src).is_some_and(|(_, _, rest)| marker(rest.trim_start()).is_some())
}

pub fn parse_krc(src: &str) -> Vec<LyricLineOwned> {
    let mut lines = Vec::new();
    for raw in src.lines() {
        let Some((start_time, end_time, rest)) = header(raw.trim()) else {
            continue;
        };
        let markers: Vec<_> = rest
            .match_indices('<')
            .filter_map(|(index, _)| {
                marker(&rest[index..])
                    .map(|(length, offset, duration)| (index, length, offset, duration))
            })
            .collect();
        let mut words = Vec::new();
        for (i, &(index, length, offset, duration)) in markers.iter().enumerate() {
            let end = markers.get(i + 1).map_or(rest.len(), |marker| marker.0);
            let text = &rest[index + length..end];
            if text.is_empty() {
                continue;
            }
            let start = start_time.saturating_add(offset);
            words.push(LyricWordOwned {
                start_time: start,
                end_time: start.saturating_add(duration),
                word: text.to_owned(),
                roman_word: String::new(),
            });
        }
        if words.is_empty() {
            continue;
        }
        lines.push(LyricLineOwned {
            end_time: end_time.unwrap_or_else(|| {
                words
                    .iter()
                    .map(|word| word.end_time)
                    .max()
                    .unwrap_or(start_time)
            }),
            start_time,
            words,
            ..Default::default()
        });
    }
    // Language rows belong to source order, before sorting by timestamps.
    for raw in src.lines() {
        let Some(encoded) = raw
            .trim()
            .strip_prefix("[language:")
            .and_then(|v| v.strip_suffix(']'))
        else {
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(contents) = value.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for item in contents {
            let kind = item.get("type").and_then(|v| v.as_u64());
            let Some(rows) = item.get("lyricContent").and_then(|v| v.as_array()) else {
                continue;
            };
            for (line, row) in lines.iter_mut().zip(rows) {
                let Some(values) = row.as_array() else {
                    continue;
                };
                let text = values.iter().filter_map(|v| v.as_str()).collect::<String>();
                match kind {
                    Some(1) if line.translated_lyric.is_empty() => line.translated_lyric = text,
                    Some(0) if line.roman_lyric.is_empty() => {
                        line.roman_lyric = text;
                        if values.len() == line.words.len() {
                            for (word, roman) in line.words.iter_mut().zip(values) {
                                word.roman_word = roman.as_str().unwrap_or_default().to_owned();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    finish_timed_lines(&mut lines);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_and_lx_preserve_relative_word_timing() {
        let raw = parse_krc("[1000,3000]<500,500,0>你<1000,900,0>好");
        assert_eq!((raw[0].start_time, raw[0].end_time), (1000, 4000));
        assert_eq!(raw[0].words[0].start_time, 1500);
        let lx = parse_krc("[00:01.3]<0,100>字");
        assert_eq!(lx[0].start_time, 1003);
    }
    #[test]
    fn language_metadata_survives_projection() {
        let json = r#"{"content":[{"type":1,"lyricContent":[["你好"]]},{"type":0,"lyricContent":[["ni ","hao"]]}]}"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(json);
        let lines = parse_krc(&format!(
            "[language:{encoded}]\n[0,1000]<0,500,0>你<500,500,0>好"
        ));
        assert_eq!(lines[0].translated_lyric, "你好");
        assert_eq!(lines[0].words[0].roman_word, "ni ");
    }
}
