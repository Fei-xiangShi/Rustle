//! ASS 字幕导出
//!
//! 导出精度 10ms 以下会丢失
//!
//! 主唱名称为 `v1`，对唱为 `v2`
//! Background lyrics get `-bg` suffix
//! Translation gets `-trans` suffix
//! Romanization gets `-roman` suffix

use super::{LyricLineOwned, LyricWordOwned, srt::parse_clock, timing::finish_timed_lines};

fn unescape(text: &str) -> String {
    text.replace("\\N", "\n")
        .replace("\\n", "\n")
        .replace("\\h", "\u{a0}")
}

fn karaoke(text: &str, start: u64, end: u64) -> Vec<LyricWordOwned> {
    let mut words = Vec::new();
    let mut cursor = start;
    let mut duration = None;
    let mut buffer = String::new();
    let mut rest = text;
    let flush = |words: &mut Vec<LyricWordOwned>,
                 buffer: &mut String,
                 cursor: u64,
                 duration: Option<u64>| {
        if !buffer.is_empty() {
            words.push(LyricWordOwned {
                start_time: cursor.min(end),
                end_time: duration.map_or(end, |d| cursor.saturating_add(d).min(end)),
                word: unescape(buffer),
                roman_word: String::new(),
            });
            buffer.clear();
        }
    };
    while let Some(open) = rest.find('{') {
        buffer.push_str(&rest[..open]);
        let Some(close) = rest[open + 1..].find('}').map(|i| open + 1 + i) else {
            buffer.push_str(&rest[open..]);
            rest = "";
            break;
        };
        for command in rest[open + 1..close].split('\\') {
            let command = command.trim();
            let time = ["kf", "ko", "kt", "k", "K"].iter().find_map(|prefix| {
                command
                    .strip_prefix(prefix)
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|value| (*prefix, value.saturating_mul(10)))
            });
            if let Some((kind, value)) = time {
                flush(&mut words, &mut buffer, cursor, duration);
                cursor = cursor.saturating_add(duration.unwrap_or(0));
                if kind == "kt" {
                    cursor = start.saturating_add(value);
                    duration = None;
                } else {
                    duration = Some(value);
                }
            }
        }
        rest = &rest[close + 1..];
    }
    buffer.push_str(rest);
    flush(&mut words, &mut buffer, cursor, duration);
    words
}

fn role(style: &str) -> (bool, bool, u8) {
    let style = style.trim().to_ascii_lowercase();
    let bg = style.contains("-bg");
    let duet = style.starts_with("v2");
    let attribute = if matches!(style.as_str(), "ts" | "translate" | "translation")
        || style.ends_with("-trans")
    {
        1
    } else if matches!(style.as_str(), "roma" | "roman" | "romaji") || style.ends_with("-roman") {
        2
    } else {
        0
    };
    (bg, duet, attribute)
}

pub fn parse_ass(src: &str) -> Vec<LyricLineOwned> {
    let mut format: Vec<String> = "layer,start,end,style,name,marginl,marginr,marginv,effect,text"
        .split(',')
        .map(str::to_owned)
        .collect();
    let mut events = false;
    let mut main = Vec::new();
    let mut attributes = Vec::new();
    for raw in src.lines().map(str::trim) {
        if raw.starts_with('[') {
            events = raw.eq_ignore_ascii_case("[Events]");
            continue;
        }
        if events && let Some(value) = raw.strip_prefix("Format:") {
            format = value
                .split(',')
                .map(|field| field.trim().to_ascii_lowercase())
                .collect();
            continue;
        }
        let Some(value) = raw.strip_prefix("Dialogue:") else {
            continue;
        };
        if format.last().map(String::as_str) != Some("text") {
            continue;
        }
        let values: Vec<_> = value.trim_start().splitn(format.len(), ',').collect();
        let get = |name: &str| {
            format
                .iter()
                .position(|field| field == name)
                .and_then(|i| values.get(i))
                .copied()
        };
        let Some(start_time) = get("start").and_then(parse_clock) else {
            continue;
        };
        let Some(end_time) = get("end").and_then(parse_clock) else {
            continue;
        };
        if end_time < start_time {
            continue;
        }
        let (is_bg, is_duet, attribute) = role(get("style").unwrap_or("default"));
        let words = karaoke(get("text").unwrap_or_default(), start_time, end_time);
        if words.is_empty() {
            continue;
        }
        let line = LyricLineOwned {
            words,
            start_time,
            end_time,
            is_bg,
            is_duet,
            ..Default::default()
        };
        if attribute == 0 {
            main.push(line);
        } else {
            attributes.push((attribute, line));
        }
    }
    for (attribute, line) in attributes {
        let text = line
            .words
            .iter()
            .map(|word| word.word.as_str())
            .collect::<String>();
        if let Some(main) = main.iter_mut().find(|main| {
            main.start_time == line.start_time
                && main.end_time == line.end_time
                && main.is_bg == line.is_bg
                && main.is_duet == line.is_duet
        }) {
            let target = if attribute == 1 {
                &mut main.translated_lyric
            } else {
                &mut main.roman_lyric
            };
            if target.is_empty() {
                *target = text;
            }
        }
    }
    finish_timed_lines(&mut main);
    main
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_format_karaoke_style_changes_and_vocals_are_preserved() {
        let src = "[Events]\nFormat: Start, End, Style, Text\nDialogue: 0:00:01.00,0:00:04.00,v1,{\\kf50}你{\\i1}好{\\k100}世界,你好\nDialogue: 0:00:01.00,0:00:04.00,v1-trans,Hello\nDialogue: 0:00:01.00,0:00:04.00,v2-bg,伴唱";
        let lines = parse_ass(src);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words[0].word, "你好");
        assert_eq!(lines[0].words[1].start_time, 1500);
        assert_eq!(lines[0].words[1].word, "世界,你好");
        assert_eq!(lines[0].end_time, 4000);
        assert_eq!(lines[0].translated_lyric, "Hello");
        assert!(lines[1].is_bg && lines[1].is_duet);
    }
}
