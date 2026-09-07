//! Presentation-safe lyrics view models and projection.

use crate::domain::lyrics::{LyricLineOwned, normalize_lyric_text, optimize_lyrics_lines};

/// A single word in a projected lyric line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub word: String,
}

/// A display-ready lyric line without UI-framework state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub words: Vec<LyricWord>,
    pub translated: Option<String>,
    pub romanized: Option<String>,
    pub is_background: bool,
    pub is_duet: bool,
}

/// Project raw parsed lyrics into the stable presentation model.
pub fn project_lyrics(mut lines: Vec<LyricLineOwned>) -> Vec<LyricLine> {
    // Keep source/cache timestamps untouched and optimize only the render copy.
    optimize_lyrics_lines(&mut lines);

    lines
        .into_iter()
        .map(|line| {
            let words: Vec<LyricWord> = line
                .words
                .into_iter()
                .map(|word| LyricWord {
                    start_ms: word.start_time,
                    end_ms: word.end_time,
                    word: normalize_lyric_text(&word.word),
                })
                .collect();
            let text = words.iter().map(|word| word.word.as_str()).collect();

            LyricLine {
                start_ms: line.start_time,
                end_ms: line.end_time,
                text,
                words,
                translated: (!line.translated_lyric.is_empty())
                    .then(|| normalize_lyric_text(&line.translated_lyric)),
                romanized: (!line.roman_lyric.is_empty())
                    .then(|| normalize_lyric_text(&line.roman_lyric)),
                is_background: line.is_bg,
                is_duet: line.is_duet,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::lyrics::LyricWordOwned;

    #[test]
    fn projection_normalizes_a_clone_and_preserves_presentation_metadata() {
        let raw = LyricLineOwned {
            words: vec![LyricWordOwned {
                start_time: 1_000,
                end_time: 2_000,
                word: "I’m  here".to_string(),
                roman_word: String::new(),
            }],
            translated_lyric: "我在这里".to_string(),
            is_bg: false,
            is_duet: true,
            start_time: 1_000,
            end_time: 2_000,
            ..Default::default()
        };

        let projected = project_lyrics(vec![raw.clone()]);

        assert_eq!(raw.words[0].word, "I’m  here");
        assert_eq!(projected[0].text, "I'm here");
        assert_eq!(projected[0].translated.as_deref(), Some("我在这里"));
        assert!(!projected[0].is_background);
        assert!(projected[0].is_duet);
    }
}
