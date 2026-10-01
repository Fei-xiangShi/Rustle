//! TTML lyric projection. Each paragraph owns its main and background words;
//! XML structure, rather than the most recently emitted line, controls routing.

use super::xml::{self, Content, Node};
use rustle_domain::lyrics::{LyricLineOwned, LyricWordOwned, MAX_LRC_TIMESTAMP};
use std::{collections::HashMap, io::BufRead};

#[derive(Debug, Default, Clone)]
pub struct TTMLLyric {
    pub lines: Vec<LyricLineOwned>,
    pub metadata: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Copy, Default)]
struct Timing {
    start: u64,
    end: u64,
}

fn timing(node: &Node, parent: Timing) -> Timing {
    let start = node
        .attr("begin")
        .and_then(parse_timestamp)
        .unwrap_or(parent.start);
    let end = node
        .attr("end")
        .and_then(parse_timestamp)
        .or_else(|| {
            node.attr("dur")
                .and_then(parse_timestamp)
                .map(|dur| start.saturating_add(dur))
        })
        .unwrap_or(parent.end)
        .max(start)
        .min(MAX_LRC_TIMESTAMP);
    Timing { start, end }
}

/// Parse common TTML clock and offset times without byte slicing or overflow.
fn parse_timestamp(value: &str) -> Option<u64> {
    let value = value.trim();
    let (number, scale) = if let Some(v) = value.strip_suffix("ms") {
        (v, 1u64)
    } else if let Some(v) = value.strip_suffix('s') {
        (v, 1_000)
    } else if let Some(v) = value.strip_suffix('m') {
        (v, 60_000)
    } else if let Some(v) = value.strip_suffix('h') {
        (v, 3_600_000)
    } else {
        (value, 1_000)
    };
    if number.is_empty()
        || !number
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == ':' || ch == '.')
    {
        return None;
    }
    let parts: Vec<_> = number.split(':').collect();
    if parts.len() > 3 || (parts.len() > 1 && scale != 1_000) {
        return None;
    }
    let mut total = 0u64;
    for (index, part) in parts.iter().enumerate() {
        if index + 1 == parts.len() {
            let (whole, fraction) = part.split_once('.').unwrap_or((part, ""));
            total = total
                .checked_mul(60)?
                .checked_add(whole.parse::<u64>().ok()?)?;
            total = total.checked_mul(scale)?;
            if !fraction.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            if !fraction.is_empty() {
                // Nine digits are enough even for the largest supported scale.
                let significant = &fraction[..fraction.len().min(9)];
                let frac = significant.parse::<u64>().ok()?;
                total = total
                    .checked_add(frac.checked_mul(scale)? / 10u64.pow(significant.len() as u32))?;
            }
        } else {
            total = total
                .checked_mul(60)?
                .checked_add(part.parse::<u64>().ok()?)?;
        }
    }
    Some(total.min(MAX_LRC_TIMESTAMP))
}

fn role(node: &Node) -> &str {
    node.attr("role").unwrap_or("")
}

fn word_text(node: &Node) -> String {
    let mut text = String::new();
    for child in &node.children {
        match child {
            Content::Text(value) => text.push_str(value),
            Content::Element(child)
                if !matches!(role(child), "x-bg" | "x-translation" | "x-roman") =>
            {
                text.push_str(&word_text(child));
            }
            _ => {}
        }
    }
    text
}

fn strip_parens(text: &str) -> String {
    text.trim()
        .trim_start_matches(['(', '（'])
        .trim_end_matches([')', '）'])
        .trim()
        .to_owned()
}

fn add_text(line: &mut LyricLineOwned, text: &str, time: Timing) {
    if !text.trim().is_empty() {
        line.words.push(LyricWordOwned {
            word: text.to_owned(),
            start_time: time.start,
            end_time: time.end,
            ..Default::default()
        });
    } else if text.contains(' ')
        && !text.contains(['\n', '\r'])
        && let Some(last) = line.words.last()
    {
        line.words.push(LyricWordOwned {
            word: " ".into(),
            start_time: last.end_time,
            end_time: last.end_time,
            ..Default::default()
        });
    }
}

fn has_nested_timing(node: &Node) -> bool {
    node.elements().any(|child| {
        !matches!(role(child), "x-translation" | "x-roman")
            && (child.attr("begin").is_some() || role(child) == "x-bg" || has_nested_timing(child))
    })
}

fn ruby_word(node: &Node, parent: Timing) -> Option<LyricWordOwned> {
    let mut base = String::new();
    let mut roman = String::new();
    let mut bounds = Vec::new();
    node.visit(&mut |child| match child.attr("ruby") {
        Some("base") => base.push_str(&child.text()),
        Some("text") => {
            roman.push_str(&child.text());
            if child.attr("begin").is_some() {
                bounds.push(timing(child, parent));
            }
        }
        _ => {}
    });
    if base.is_empty() {
        return None;
    }
    Some(LyricWordOwned {
        word: base,
        roman_word: roman,
        start_time: bounds
            .iter()
            .map(|time| time.start)
            .min()
            .unwrap_or(parent.start),
        end_time: bounds
            .iter()
            .map(|time| time.end)
            .max()
            .unwrap_or(parent.end),
    })
}

fn collect_content(
    node: &Node,
    time: Timing,
    line: &mut LyricLineOwned,
    backgrounds: &mut Vec<LyricLineOwned>,
) {
    for child in &node.children {
        match child {
            Content::Text(text) => add_text(line, text, time),
            Content::Element(child) => match role(child) {
                "x-bg" => backgrounds.extend(paragraph(child, time, true, line.is_duet)),
                "x-translation" => {
                    if line.translated_lyric.is_empty() {
                        line.translated_lyric = child.text().trim().into();
                    }
                }
                "x-roman" => {
                    if line.roman_lyric.is_empty() {
                        line.roman_lyric = child.text().trim().into();
                    }
                }
                _ => {
                    let child_time = timing(child, time);
                    if child.attr("ruby") == Some("container") {
                        if let Some(word) = ruby_word(child, child_time) {
                            line.words.push(word);
                        }
                    } else if child.attr("begin").is_some() && !has_nested_timing(child) {
                        let text = word_text(child);
                        if !text.is_empty() {
                            let roman = child
                                .elements()
                                .find(|node| role(node) == "x-roman")
                                .map(|node| node.text())
                                .unwrap_or_default();
                            line.words.push(LyricWordOwned {
                                word: text,
                                start_time: child_time.start,
                                end_time: child_time.end,
                                roman_word: roman,
                            });
                        }
                    } else if child.name == "br" {
                        add_text(line, " ", child_time);
                    } else {
                        collect_content(child, child_time, line, backgrounds);
                    }
                }
            },
        }
    }
}

fn paragraph(node: &Node, parent: Timing, is_bg: bool, is_duet: bool) -> Vec<LyricLineOwned> {
    let time = timing(node, parent);
    let mut line = LyricLineOwned {
        start_time: time.start,
        end_time: time.end,
        is_bg,
        is_duet,
        ..Default::default()
    };
    let mut backgrounds = Vec::new();
    collect_content(node, time, &mut line, &mut backgrounds);
    if node.attr("begin").is_none() {
        line.start_time = line
            .words
            .iter()
            .filter(|word| !word.word.trim().is_empty())
            .map(|word| word.start_time)
            .min()
            .unwrap_or(time.start);
    }
    if node.attr("end").is_none() && node.attr("dur").is_none() {
        line.end_time = line
            .words
            .iter()
            .filter(|word| !word.word.trim().is_empty())
            .map(|word| word.end_time)
            .max()
            .unwrap_or(time.end);
    }
    if is_bg {
        if let Some(first) = line.words.first_mut() {
            first.word = first.word.trim_start_matches(['(', '（']).to_owned();
        }
        if let Some(last) = line.words.last_mut() {
            last.word = last.word.trim_end_matches([')', '）']).to_owned();
        }
        line.words.retain(|word| !word.word.is_empty());
    }
    let mut lines = Vec::new();
    if line.words.iter().any(|word| !word.word.trim().is_empty()) {
        lines.push(line);
    }
    lines.extend(backgrounds);
    lines
}

#[derive(Default)]
struct Attributes {
    translation: String,
    roman: String,
    roman_words: Vec<LyricWordOwned>,
}

#[derive(Default)]
struct VocalAttributes {
    main: Attributes,
    background: Attributes,
}

fn collect_keyed_metadata(node: &Node, kind: &str, index: &mut HashMap<String, VocalAttributes>) {
    let kind = match node.name.as_str() {
        "translations" | "translation" => "translation",
        "transliterations" | "transliteration" => "roman",
        _ => kind,
    };
    if node.name == "text"
        && !kind.is_empty()
        && let Some(key) = node.attr("for")
    {
        let attrs = index
            .entry(key.trim_start_matches('#').to_owned())
            .or_default();
        for line in paragraph(node, Timing::default(), false, false) {
            let target = if line.is_bg {
                &mut attrs.background
            } else {
                &mut attrs.main
            };
            let text: String = line.words.iter().map(|word| word.word.as_str()).collect();
            if kind == "translation" && target.translation.is_empty() {
                target.translation = if line.is_bg {
                    strip_parens(&text)
                } else {
                    text.trim().to_owned()
                };
            } else if kind == "roman" && target.roman.is_empty() {
                target.roman = text.trim().to_owned();
                target.roman_words = line
                    .words
                    .into_iter()
                    .filter(|word| !word.word.trim().is_empty() && word.end_time > word.start_time)
                    .collect();
                target.roman_words.sort_by_key(|word| word.start_time);
            }
        }
        return;
    }
    for child in node.elements() {
        collect_keyed_metadata(child, kind, index);
    }
}

/// Match word romanization by exact onset, then by interval overlap (SPlayer).
fn align_roman_words(words: &mut [LyricWordOwned], roman: &[LyricWordOwned]) {
    let mut next = 0;
    for word in words.iter_mut().filter(|word| !word.word.trim().is_empty()) {
        let mut best = None;
        let mut best_overlap = 0.1f64;
        for (index, candidate) in roman.iter().enumerate().skip(next) {
            if word.start_time.abs_diff(candidate.start_time) <= 2 {
                best = Some(index);
                break;
            }
            let intersection = word
                .end_time
                .min(candidate.end_time)
                .saturating_sub(word.start_time.max(candidate.start_time));
            let union = word
                .end_time
                .max(candidate.end_time)
                .saturating_sub(word.start_time.min(candidate.start_time))
                .max(1);
            let overlap = intersection as f64 / union as f64;
            if intersection > 0 && overlap >= best_overlap {
                best_overlap = overlap;
                best = Some(index);
            }
            if candidate.start_time >= word.end_time {
                break;
            }
        }
        if let Some(index) = best {
            if word.roman_word.is_empty() {
                word.roman_word.clone_from(&roman[index].word);
            }
            next = index + 1;
        }
    }
}

fn collect_paragraphs(
    node: &Node,
    parent: Timing,
    main_agent: &str,
    agents: &HashMap<String, String>,
    attributes: &HashMap<String, VocalAttributes>,
    lines: &mut Vec<LyricLineOwned>,
) {
    let time = timing(node, parent);
    if node.name == "p" {
        let agent = node.attr("agent").unwrap_or(main_agent);
        let duet = agent != main_agent && agents.get(agent).is_none_or(|kind| kind != "group");
        let mut parsed = paragraph(node, parent, false, duet);
        if let Some(attrs) = node
            .attr("key")
            .or_else(|| node.attr("id"))
            .and_then(|key| attributes.get(key))
        {
            for line in &mut parsed {
                let attrs = if line.is_bg {
                    &attrs.background
                } else {
                    &attrs.main
                };
                if line.translated_lyric.is_empty() {
                    line.translated_lyric.clone_from(&attrs.translation);
                }
                if line.roman_lyric.is_empty() {
                    line.roman_lyric.clone_from(&attrs.roman);
                }
                align_roman_words(&mut line.words, &attrs.roman_words);
            }
        }
        lines.extend(parsed);
    } else {
        for child in node.elements() {
            collect_paragraphs(child, time, main_agent, agents, attributes, lines);
        }
    }
}

pub fn parse_ttml(data: impl BufRead) -> Result<TTMLLyric, String> {
    let root = xml::parse(data)?;
    if root.name != "tt" {
        return Err("Expected TTML root".into());
    }
    let mut result = TTMLLyric::default();
    let mut agents = HashMap::new();
    let mut main_agent = None;
    let mut invalid_time = false;
    root.visit(&mut |node| {
        for name in ["begin", "end", "dur"] {
            if node
                .attr(name)
                .is_some_and(|value| parse_timestamp(value).is_none())
            {
                invalid_time = true;
            }
        }
        if node.name == "agent"
            && let Some(id) = node.attr("id")
        {
            let kind = node.attr("type").unwrap_or("");
            if kind == "person" && main_agent.is_none() {
                main_agent = Some(id.to_owned());
            }
            agents.insert(id.to_owned(), kind.to_owned());
        }
        if node.name == "meta"
            && let (Some(key), Some(value)) = (node.attr("key"), node.attr("value"))
        {
            if let Some((_, values)) = result.metadata.iter_mut().find(|(name, _)| name == key) {
                values.push(value.to_owned());
            } else {
                result
                    .metadata
                    .push((key.to_owned(), vec![value.to_owned()]));
            }
        }
    });
    if invalid_time {
        return Err("Invalid or unsupported TTML time".into());
    }
    let mut attributes = HashMap::new();
    collect_keyed_metadata(&root, "", &mut attributes);
    for body in root.elements().filter(|node| node.name == "body") {
        collect_paragraphs(
            body,
            Timing::default(),
            main_agent.as_deref().unwrap_or("v1"),
            &agents,
            &attributes,
            &mut result.lines,
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
