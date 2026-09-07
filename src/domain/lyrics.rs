//! Framework-free lyrics domain types and normalization rules.
//!
//! Owned variants for easier use.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const MAX_LRC_TIMESTAMP: u64 = 60_039_999;

pub fn normalize_lyric_text(text: &str) -> String {
    static SMART_APOSTROPHE_RE: OnceLock<Regex> = OnceLock::new();
    SMART_APOSTROPHE_RE
        .get_or_init(|| Regex::new("’").expect("valid smart apostrophe regex"))
        .replace_all(text, "'")
        .into_owned()
}

/// Lyrics format enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsFormat {
    /// Standard LRC format [mm:ss.xx]text
    Lrc,
    /// Lyricify Quick Export container
    Lqe,
    /// NetEase Cloud Music YRC format (word-level)
    Yrc,
    /// QQ Music QRC format (word-level)
    Qrc,
    /// Foobar2000 ESLyric format (word-level)
    EsLrc,
    /// Lyricify Syllable format
    Lys,
    /// Apple Music TTML format
    Ttml,
    /// Unknown format
    Unknown,
}

/// A single word in a lyric line
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricWordOwned {
    /// Start time in milliseconds
    pub start_time: u64,
    /// End time in milliseconds
    pub end_time: u64,
    /// The word text
    pub word: String,
    /// Romanized/phonetic version of the word
    pub roman_word: String,
}

/// A single line of lyrics
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLineOwned {
    /// Words in this line (for word-level sync)
    pub words: Vec<LyricWordOwned>,
    /// Translated lyric text
    pub translated_lyric: String,
    /// Romanized/phonetic lyric text
    pub roman_lyric: String,
    /// Whether this is a background vocal line
    #[serde(default, rename = "isBG")]
    pub is_bg: bool,
    /// Whether this is a duet line (different singer)
    #[serde(default)]
    pub is_duet: bool,
    /// Start time in milliseconds
    #[serde(default)]
    pub start_time: u64,
    /// End time in milliseconds
    #[serde(default)]
    pub end_time: u64,
}

/// Process lyrics lines: sort by time and clamp values
pub fn process_lyrics(lines: &mut [LyricLineOwned]) {
    // Sort by start time
    lines.sort_by(|a, b| {
        a.words
            .first()
            .map(|x| x.start_time)
            .cmp(&b.words.first().map(|x| x.start_time))
    });

    for line in lines.iter_mut() {
        // Update start_time from first word
        line.start_time = line
            .words
            .first()
            .map(|x| x.start_time)
            .unwrap_or(line.start_time)
            .clamp(0, MAX_LRC_TIMESTAMP);

        // Update end_time from last word, but only if word has valid end_time
        // This preserves end_time calculated by parsers (like LRC's next-line-start logic)
        let word_end_time = line.words.last().map(|x| x.end_time).unwrap_or(0);
        if word_end_time > 0 {
            line.end_time = word_end_time.clamp(0, MAX_LRC_TIMESTAMP);
        } else if line.end_time == 0 {
            // If both are 0, we'll fix it in the next pass
            line.end_time = 0;
        } else {
            line.end_time = line.end_time.clamp(0, MAX_LRC_TIMESTAMP);
        }

        for word in line.words.iter_mut() {
            word.start_time = word.start_time.clamp(0, MAX_LRC_TIMESTAMP);
            word.end_time = word.end_time.clamp(0, MAX_LRC_TIMESTAMP);
        }
    }

    // Second pass: fix any remaining 0 end_times using next line's start_time
    let len = lines.len();
    for i in 0..len {
        if lines[i].end_time == 0 {
            let next_start = if i + 1 < len {
                lines[i + 1].start_time
            } else {
                // Last line: add 5 seconds
                lines[i].start_time.saturating_add(5000)
            };
            lines[i].end_time = next_start.clamp(0, MAX_LRC_TIMESTAMP);

            // Also fix word end_time
            if let Some(word) = lines[i].words.last_mut()
                && word.end_time == 0
            {
                word.end_time = next_start.clamp(0, MAX_LRC_TIMESTAMP);
            }
        }
    }
}

fn normalize_spaces(lines: &mut [LyricLineOwned]) {
    for line in lines {
        for word in &mut line.words {
            let source = normalize_lyric_text(&word.word);
            let mut normalized = String::with_capacity(source.len());
            let mut previous_was_whitespace = false;

            for ch in source.chars() {
                if ch.is_whitespace() {
                    if !previous_was_whitespace {
                        normalized.push(' ');
                    }
                    previous_was_whitespace = true;
                } else {
                    normalized.push(ch);
                    previous_was_whitespace = false;
                }
            }

            word.word = normalized;
        }

        line.translated_lyric = normalize_lyric_text(&line.translated_lyric);
        line.roman_lyric = normalize_lyric_text(&line.roman_lyric);
    }
}

fn reset_line_timestamps(lines: &mut [LyricLineOwned]) {
    for line in lines {
        if line.words.len() == 1
            && line.words[0].start_time == 0
            && line.words[0].end_time == 0
            && (line.start_time != 0 || line.end_time != 0)
        {
            line.words[0].start_time = line.start_time;
            line.words[0].end_time = line.end_time;
        } else if !line.words.is_empty() {
            if let Some(first_word) = line.words.first() {
                line.start_time = first_word.start_time;
            }
            if let Some(last_word) = line.words.last() {
                line.end_time = last_word.end_time;
            }
        }
    }
}

fn convert_excessive_background_lines(lines: &mut [LyricLineOwned]) {
    let mut consecutive_bg_count = 0usize;

    for (index, line) in lines.iter_mut().enumerate() {
        if line.is_bg {
            consecutive_bg_count += 1;
            if index == 0 || consecutive_bg_count > 1 {
                line.is_bg = false;
            }
        } else {
            consecutive_bg_count = 0;
        }
    }
}

fn sync_main_and_background_lines(lines: &mut [LyricLineOwned]) {
    for i in (0..lines.len()).rev() {
        if lines[i].is_bg {
            continue;
        }

        let Some(next_line) = lines.get(i + 1) else {
            continue;
        };
        if !next_line.is_bg {
            continue;
        }

        let mut min_start = u64::MAX;
        let mut max_end = 0u64;

        for word in lines[i]
            .words
            .iter()
            .chain(lines[i + 1].words.iter())
            .filter(|w| !w.word.trim().is_empty())
        {
            min_start = min_start.min(word.start_time);
            max_end = max_end.max(word.end_time);
        }

        if min_start == u64::MAX {
            continue;
        }

        let final_start = min_start
            .min(lines[i].start_time)
            .min(lines[i + 1].start_time);
        let final_end = max_end.max(lines[i].end_time).max(lines[i + 1].end_time);

        lines[i].start_time = final_start;
        lines[i].end_time = final_end;
        lines[i + 1].start_time = final_start;
        lines[i + 1].end_time = final_end;
    }
}

fn clean_unintentional_overlaps(lines: &mut [LyricLineOwned]) {
    for i in 0..lines.len().saturating_sub(1) {
        if lines[i].is_bg {
            continue;
        }

        // Continue through later main lines so an accidental overlap after an
        // intentional overlap is still clipped, matching AMLL.
        for j in (i + 1)..lines.len() {
            if lines[j].is_bg {
                continue;
            }

            if lines[i].end_time <= lines[j].start_time {
                break;
            }

            let overlap = lines[i].end_time - lines[j].start_time;
            let next_duration = lines[j].end_time.saturating_sub(lines[j].start_time);
            let percentage_threshold = ((next_duration as f64) * 0.1).round() as u64;
            let is_intentional_overlap =
                overlap >= 500 || (overlap > 100 && overlap > percentage_threshold);

            if !is_intentional_overlap {
                lines[i].end_time = lines[j].start_time;

                if lines.get(i + 1).is_some_and(|line| line.is_bg) {
                    lines[i + 1].end_time = lines[j].start_time;
                }
                break;
            }
        }
    }
}

/// Sort main lyric lines by start time while keeping an attached background
/// line immediately after its main line.
fn sort_lyric_lines(lines: &mut [LyricLineOwned]) {
    let mut groups: Vec<(u64, usize, Vec<LyricLineOwned>)> = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let main_line = lines[index].clone();
        let start_time = main_line.start_time;
        let original_index = groups.len();
        let mut group = vec![main_line];
        index += 1;

        if !group[0].is_bg && lines.get(index).is_some_and(|line| line.is_bg) {
            group.push(lines[index].clone());
            index += 1;
        }

        groups.push((start_time, original_index, group));
    }

    groups.sort_by_key(|(start_time, original_index, _)| (*start_time, *original_index));

    let mut output_index = 0usize;
    for (_, _, group) in groups {
        for line in group {
            lines[output_index] = line;
            output_index += 1;
        }
    }
}

/// Try to make each line visible up to 600ms before its first word. This is
/// the AMLL algorithm: traverse forward, retain original line/group bounds,
/// and only shift the display line start (word timestamps stay untouched).
fn try_advance_start_time(lines: &mut [LyricLineOwned]) {
    let mut prev_line_start_time = 0u64;
    let mut prev_line_end_time = 0u64;
    let mut prev_group_start_time = 0u64;
    let mut prev_group_end_time = 0u64;
    let mut has_previous_line = false;

    for i in 0..lines.len() {
        if lines[i].is_bg {
            continue;
        }

        let original_start_time = lines[i].start_time;
        let original_end_time = lines[i].end_time;

        let (target_advance_amount, safe_boundary) = if has_previous_line {
            if original_start_time >= prev_line_end_time {
                (600u64, prev_group_end_time)
            } else {
                let overlap_duration = prev_line_end_time.saturating_sub(original_start_time);
                let advance = if overlap_duration < 400 {
                    overlap_duration * 70 / 100
                } else {
                    400
                };
                (advance, prev_line_start_time)
            }
        } else {
            (600u64, 0u64)
        };

        let target_time = original_start_time.saturating_sub(target_advance_amount);
        let new_start_time = safe_boundary.max(target_time);

        if new_start_time < original_start_time {
            lines[i].start_time = new_start_time;
        }

        if lines.get(i + 1).is_some_and(|line| line.is_bg) {
            lines[i + 1].start_time = lines[i].start_time;
        }

        if has_previous_line {
            let overlaps_previous_group = original_start_time < prev_group_end_time
                && original_end_time > prev_group_start_time;
            if overlaps_previous_group {
                prev_group_start_time = prev_group_start_time.min(original_start_time);
                prev_group_end_time = prev_group_end_time.max(original_end_time);
            } else {
                prev_group_start_time = original_start_time;
                prev_group_end_time = original_end_time;
            }
        } else {
            prev_group_start_time = original_start_time;
            prev_group_end_time = original_end_time;
        }

        prev_line_start_time = lines[i].start_time;
        prev_line_end_time = original_end_time;
        has_previous_line = true;
    }
}

pub fn optimize_lyrics_lines(lines: &mut [LyricLineOwned]) {
    normalize_spaces(lines);
    reset_line_timestamps(lines);
    convert_excessive_background_lines(lines);
    sync_main_and_background_lines(lines);
    sort_lyric_lines(lines);
    clean_unintentional_overlaps(lines);
    try_advance_start_time(lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_word(start_time: u64, end_time: u64, word: &str) -> LyricWordOwned {
        LyricWordOwned {
            start_time,
            end_time,
            word: word.to_string(),
            roman_word: String::new(),
        }
    }

    #[test]
    fn optimize_syncs_main_and_background_ranges() {
        let mut lines = vec![
            LyricLineOwned {
                words: vec![make_word(1_000, 2_000, "main")],
                start_time: 1_000,
                end_time: 2_000,
                ..Default::default()
            },
            LyricLineOwned {
                words: vec![make_word(900, 2_200, "bg")],
                start_time: 900,
                end_time: 2_200,
                is_bg: true,
                ..Default::default()
            },
        ];

        optimize_lyrics_lines(&mut lines);

        assert_eq!(lines[0].start_time, 300);
        assert_eq!(lines[0].end_time, 2_200);
        assert_eq!(lines[1].start_time, 300);
        assert_eq!(lines[1].end_time, 2_200);
    }

    #[test]
    fn optimize_advances_display_start_but_keeps_word_timestamp() {
        let mut lines = vec![
            LyricLineOwned {
                words: vec![make_word(1_120, 5_000, "first")],
                start_time: 1_120,
                end_time: 5_000,
                ..Default::default()
            },
            LyricLineOwned {
                words: vec![make_word(5_000, 8_000, "second")],
                start_time: 5_000,
                end_time: 8_000,
                ..Default::default()
            },
        ];

        optimize_lyrics_lines(&mut lines);

        assert_eq!(lines[0].start_time, 520);
        assert_eq!(lines[0].words[0].start_time, 1_120);
    }

    #[test]
    fn normalize_lyric_text_replaces_smart_apostrophes() {
        assert_eq!(
            normalize_lyric_text("I’m fine, don’t worry"),
            "I'm fine, don't worry"
        );
    }
}
