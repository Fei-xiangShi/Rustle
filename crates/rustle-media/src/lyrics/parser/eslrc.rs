//! Foobar2000 ESLyric format parser
//!
//! ESLrc is a word-level lyrics format used by the ESLyric plugin for Foobar2000.
//! It uses standard LRC timestamps but interleaves them with words.
//! Format: [mm:ss.xx]word[mm:ss.xx]word[mm:ss.xx]...
//! Each timestamp marks the END time of the preceding word.

#[cfg(test)]
use super::lrc;
use rustle_domain::lyrics::LyricLineOwned;

/// Share LRC metadata, bracket/angle word timing and repeated-line handling.
pub fn parse_eslrc(src: &str) -> Vec<LyricLineOwned> {
    super::lrc::parse_lrc(src)
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
    fn metadata_literal_brackets_and_unicode_never_stall() {
        let lines = parse_eslrc("[ar:Artist]\n[00:01]中文[注释]正文[00:02]");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words[0].word, "中文[注释]正文");
        assert_eq!(lines[0].end_time, 2000);
    }

    #[test]
    fn angle_words_keep_line_anchor_and_explicit_last_end() {
        let lines = parse_eslrc("[00:01]<00:01.5>你<00:02>好<00:03>\n[00:05]结束");
        assert_eq!(lines[0].start_time, 1000);
        assert_eq!(lines[0].words[0].start_time, 1500);
        assert_eq!(lines[0].words[1].end_time, 3000);
        assert_eq!(lines[0].end_time, 3000);
    }

    #[test]
    fn timed_background_parentheses_are_metadata_not_visible_words() {
        let lines = parse_eslrc("[00:01](<00:01>伴<00:02>唱<00:03>)");
        assert!(lines[0].is_bg);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[0].word, "伴");
        assert_eq!(lines[0].words[1].word, "唱");
    }

    #[test]
    fn repeated_word_lines_shift_each_copy_without_losing_translation() {
        let lines = parse_eslrc("[00:01][00:05]<00:01>Hello<00:02> world<00:03>\n[00:01]你好世界");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].translated_lyric, "你好世界");
        assert_eq!(lines[1].words[1].start_time, 6000);
        assert_eq!(lines[1].end_time, 7000);
    }

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
