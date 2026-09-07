//! TTML (Apple Music) lyrics format parser

use std::io::BufRead;

use crate::domain::lyrics::{LyricLineOwned, LyricWordOwned};

/// TTML lyrics with metadata
#[derive(Debug, Default, Clone)]
pub struct TTMLLyric {
    pub lines: Vec<LyricLineOwned>,
    pub metadata: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseStatus {
    None,
    InTtml,
    InHead,
    InMetadata,
    InBody,
    InDiv,
    InP,
    InSpan,
    InTranslationSpan,
    InRomanSpan,
    InBackgroundSpan,
    InSpanInBackgroundSpan,
    InTranslationSpanInBackgroundSpan,
    InRomanSpanInBackgroundSpan,
}

/// Parse TTML format lyrics
pub fn parse_ttml(data: impl BufRead) -> Result<TTMLLyric, String> {
    use quick_xml::{Reader, events::Event};

    let mut reader = Reader::from_reader(data);
    let mut buf = Vec::with_capacity(256);
    let mut str_buf = String::with_capacity(256);
    let mut status = ParseStatus::None;
    let mut result = TTMLLyric::default();
    let mut main_agent = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.name();
                match name.as_ref() {
                    "tt" if status == ParseStatus::None => {
                        status = ParseStatus::InTtml;
                    }
                    "head" if status == ParseStatus::InTtml => {
                        status = ParseStatus::InHead;
                    }
                    "metadata" if status == ParseStatus::InHead => {
                        status = ParseStatus::InMetadata;
                    }
                    "ttm:agent" if main_agent.is_empty() && status == ParseStatus::InMetadata => {
                        let mut agent_type = String::new();
                        let mut agent_id = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                "type" => agent_type = attr.value.into_owned(),
                                "xml:id" => agent_id = attr.value.into_owned(),
                                _ => {}
                            }
                        }
                        if agent_type == "person" {
                            main_agent = agent_id;
                        }
                    }
                    "amll:meta" if status == ParseStatus::InMetadata => {
                        let mut meta_key = String::new();
                        let mut meta_value = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                "key" => {
                                    meta_key = attr.value.into_owned();
                                }
                                "value" => {
                                    meta_value = attr.value.into_owned();
                                }
                                _ => {}
                            }
                        }
                        if !meta_key.is_empty() {
                            if let Some(values) =
                                result.metadata.iter_mut().find(|x| x.0 == meta_key)
                            {
                                values.1.push(meta_value);
                            } else {
                                result.metadata.push((meta_key, vec![meta_value]));
                            }
                        }
                    }
                    "body" if status == ParseStatus::InTtml => {
                        status = ParseStatus::InBody;
                    }
                    "div" if status == ParseStatus::InBody => {
                        status = ParseStatus::InDiv;
                    }
                    "p" if status == ParseStatus::InDiv => {
                        status = ParseStatus::InP;
                        let mut new_line = LyricLineOwned::default();
                        configure_line(&e, &main_agent, &mut new_line);
                        result.lines.push(new_line);
                    }
                    "span" => match status {
                        ParseStatus::InP => {
                            status = ParseStatus::InSpan;
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == "ttm:role" {
                                    match attr.value.as_ref() {
                                        "x-bg" => {
                                            status = ParseStatus::InBackgroundSpan;
                                            let mut new_bg_line = LyricLineOwned {
                                                is_bg: true,
                                                is_duet: result
                                                    .lines
                                                    .last()
                                                    .map(|l| l.is_duet)
                                                    .unwrap_or(false),
                                                ..Default::default()
                                            };
                                            configure_line(&e, &main_agent, &mut new_bg_line);
                                            result.lines.push(new_bg_line);
                                            break;
                                        }
                                        "x-translation" => {
                                            status = ParseStatus::InTranslationSpan;
                                            break;
                                        }
                                        "x-roman" => {
                                            status = ParseStatus::InRomanSpan;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            if status == ParseStatus::InSpan {
                                let mut new_word = LyricWordOwned::default();
                                configure_word(&e, &mut new_word);
                                if let Some(line) = result.lines.last_mut() {
                                    line.words.push(new_word);
                                }
                            }
                        }
                        ParseStatus::InBackgroundSpan => {
                            status = ParseStatus::InSpanInBackgroundSpan;
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == "ttm:role" {
                                    match attr.value.as_ref() {
                                        "x-translation" => {
                                            status = ParseStatus::InTranslationSpanInBackgroundSpan;
                                            break;
                                        }
                                        "x-roman" => {
                                            status = ParseStatus::InRomanSpanInBackgroundSpan;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            if status == ParseStatus::InSpanInBackgroundSpan {
                                let mut new_word = LyricWordOwned::default();
                                configure_word(&e, &mut new_word);
                                if let Some(line) = result.lines.iter_mut().rev().find(|l| l.is_bg)
                                {
                                    line.words.push(new_word);
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "tt" => status = ParseStatus::None,
                "head" if status == ParseStatus::InHead => {
                    status = ParseStatus::InTtml;
                }
                "metadata" if status == ParseStatus::InMetadata => {
                    status = ParseStatus::InHead;
                }
                "body" if status == ParseStatus::InBody => {
                    status = ParseStatus::InTtml;
                }
                "div" if status == ParseStatus::InDiv => {
                    status = ParseStatus::InBody;
                }
                "p" if status == ParseStatus::InP => {
                    status = ParseStatus::InDiv;
                }
                "span" => match status {
                    ParseStatus::InSpan => {
                        status = ParseStatus::InP;
                        if let Some(line) = result.lines.last_mut()
                            && let Some(word) = line.words.last_mut()
                        {
                            word.word = str_buf.clone();
                        }
                        str_buf.clear();
                    }
                    ParseStatus::InBackgroundSpan => {
                        status = ParseStatus::InP;
                        str_buf.clear();
                    }
                    ParseStatus::InSpanInBackgroundSpan => {
                        status = ParseStatus::InBackgroundSpan;
                        if let Some(line) = result.lines.iter_mut().rev().find(|l| l.is_bg)
                            && let Some(word) = line.words.last_mut()
                        {
                            word.word = str_buf.clone();
                        }
                        str_buf.clear();
                    }
                    ParseStatus::InTranslationSpan => {
                        status = ParseStatus::InP;
                        if let Some(line) = result.lines.iter_mut().rev().find(|l| !l.is_bg)
                            && line.translated_lyric.is_empty()
                        {
                            line.translated_lyric = str_buf.clone();
                        }
                        str_buf.clear();
                    }
                    ParseStatus::InRomanSpan => {
                        status = ParseStatus::InP;
                        if let Some(line) = result.lines.iter_mut().rev().find(|l| !l.is_bg) {
                            line.roman_lyric = str_buf.clone();
                        }
                        str_buf.clear();
                    }
                    ParseStatus::InTranslationSpanInBackgroundSpan => {
                        status = ParseStatus::InBackgroundSpan;
                        if let Some(line) = result.lines.iter_mut().rev().find(|l| l.is_bg) {
                            line.translated_lyric = str_buf.clone();
                        }
                        str_buf.clear();
                    }
                    ParseStatus::InRomanSpanInBackgroundSpan => {
                        status = ParseStatus::InBackgroundSpan;
                        if let Some(line) = result.lines.iter_mut().rev().find(|l| l.is_bg) {
                            line.roman_lyric = str_buf.clone();
                        }
                        str_buf.clear();
                    }
                    _ => {}
                },
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if let Ok(txt) = quick_xml::escape::unescape(e.as_ref()) {
                    match status {
                        ParseStatus::InP => {
                            if let Some(line) = result.lines.iter_mut().rev().find(|l| !l.is_bg) {
                                line.words.push(LyricWordOwned {
                                    word: txt.to_string(),
                                    ..Default::default()
                                });
                            }
                        }
                        ParseStatus::InBackgroundSpan => {
                            if let Some(line) = result.lines.iter_mut().rev().find(|l| l.is_bg) {
                                line.words.push(LyricWordOwned {
                                    word: txt.to_string(),
                                    ..Default::default()
                                });
                            }
                        }
                        ParseStatus::InSpan
                        | ParseStatus::InTranslationSpan
                        | ParseStatus::InRomanSpan
                        | ParseStatus::InSpanInBackgroundSpan
                        | ParseStatus::InTranslationSpanInBackgroundSpan
                        | ParseStatus::InRomanSpanInBackgroundSpan => {
                            str_buf.push_str(&txt);
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    // Post-process: strip parentheses from background vocals
    for line in result.lines.iter_mut() {
        if line.is_bg {
            if let Some(first) = line.words.first_mut()
                && let Some(stripped) = first.word.strip_prefix('(')
            {
                first.word = stripped.to_string();
            }
            if let Some(last) = line.words.last_mut()
                && let Some(stripped) = last.word.strip_suffix(')')
            {
                last.word = stripped.to_string();
            }
        }
        // Update line timing from words
        if let Some(first) = line.words.first()
            && line.start_time == 0
        {
            line.start_time = first.start_time;
        }
        if let Some(last) = line.words.last()
            && line.end_time == 0
        {
            line.end_time = last.end_time;
        }
    }

    Ok(result)
}

fn configure_line(
    e: &quick_xml::events::BytesStart<'_>,
    main_agent: &str,
    line: &mut LyricLineOwned,
) {
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            "ttm:agent" => {
                line.is_duet = attr.value.as_ref() != main_agent;
            }
            "begin" => {
                if let Some(time) = parse_timestamp(&attr.value) {
                    line.start_time = time;
                }
            }
            "end" => {
                if let Some(time) = parse_timestamp(&attr.value) {
                    line.end_time = time;
                }
            }
            _ => {}
        }
    }
}

fn configure_word(e: &quick_xml::events::BytesStart<'_>, word: &mut LyricWordOwned) {
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            "begin" => {
                if let Some(time) = parse_timestamp(&attr.value) {
                    word.start_time = time;
                }
            }
            "end" => {
                if let Some(time) = parse_timestamp(&attr.value) {
                    word.end_time = time;
                }
            }
            _ => {}
        }
    }
}

/// Parse TTML timestamp format (HH:MM:SS.MS or MM:SS.MS or SS.MS)
fn parse_timestamp(data: &str) -> Option<u64> {
    let s = data.trim().trim_end_matches('s');

    let parts: Vec<&str> = s.split(':').collect();

    match parts.len() {
        1 => {
            // SS.MS format
            parse_seconds_ms(parts[0])
        }
        2 => {
            // MM:SS.MS format
            let min: u64 = parts[0].parse().ok()?;
            let sec_ms = parse_seconds_ms(parts[1])?;
            Some(min * 60 * 1000 + sec_ms)
        }
        3 => {
            // HH:MM:SS.MS format
            let hour: u64 = parts[0].parse().ok()?;
            let min: u64 = parts[1].parse().ok()?;
            let sec_ms = parse_seconds_ms(parts[2])?;
            Some(hour * 60 * 60 * 1000 + min * 60 * 1000 + sec_ms)
        }
        _ => None,
    }
}

fn parse_seconds_ms(s: &str) -> Option<u64> {
    if let Some(dot_pos) = s.find('.') {
        let sec: u64 = s[..dot_pos].parse().ok()?;
        let frac_str = &s[dot_pos + 1..];
        let ms = match frac_str.len() {
            0 => 0,
            1 => frac_str.parse::<u64>().ok()? * 100,
            2 => frac_str.parse::<u64>().ok()? * 10,
            3 => frac_str.parse::<u64>().ok()?,
            _ => frac_str[..3].parse::<u64>().ok()?,
        };
        Some(sec * 1000 + ms)
    } else {
        let sec: u64 = s.parse().ok()?;
        Some(sec * 1000)
    }
}
