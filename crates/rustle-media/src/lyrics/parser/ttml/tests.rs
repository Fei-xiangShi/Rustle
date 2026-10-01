use super::*;

fn parse(body: &str) -> Vec<LyricLineOwned> {
    parse_ttml(format!("<tt xmlns:ttm=\"http://www.w3.org/ns/ttml#metadata\"><body><div>{body}</div></body></tt>").as_bytes()).unwrap().lines
}

fn text(line: &LyricLineOwned) -> String {
    line.words.iter().map(|word| word.word.as_str()).collect()
}

#[test]
fn empty_head_and_div_do_not_swallow_body() {
    let input = r#"<tt><head/><body><div/><div><p begin="1s" end="2s"><span begin="1s" end="2s">Hello</span></p></div></body></tt>"#;
    let lines = parse_ttml(input.as_bytes()).unwrap().lines;
    assert_eq!(lines.len(), 1);
    assert_eq!(text(&lines[0]), "Hello");
}

#[test]
fn xml_references_and_cdata_are_decoded_once() {
    let lines = parse(
        r#"<p begin="1s" end="2s"><span>You &amp; I &#x4F60; &#20320; &amp;amp; <![CDATA[&amp;]]></span></p>"#,
    );
    assert_eq!(text(&lines[0]), "You & I 你 你 &amp; &amp;");
    assert_eq!(lines[0].words[0].start_time, 1_000);
    assert_eq!(lines[0].words[0].end_time, 2_000);
}

#[test]
fn main_words_after_background_keep_their_owner() {
    let lines = parse(
        r#"<p begin="1s" end="4s"><span begin="1s" end="2s">A</span><span ttm:role="x-bg"><span begin="2s" end="3s">（B）</span><span ttm:role="x-translation">乙</span></span><span begin="3s" end="4s">C</span><span ttm:role="x-translation">甲丙</span></p>"#,
    );
    assert_eq!(lines.len(), 2);
    assert_eq!(text(&lines[0]), "AC");
    assert_eq!(text(&lines[1]), "B");
    assert!(!lines[0].is_bg);
    assert!(lines[1].is_bg);
    assert_eq!(lines[0].translated_lyric, "甲丙");
    assert_eq!(lines[1].translated_lyric, "乙");
    assert_eq!((lines[1].start_time, lines[1].end_time), (2_000, 3_000));
}

#[test]
fn namespace_prefixes_nested_spans_and_significant_spaces() {
    let input = r#"<t:tt xmlns:t="http://www.w3.org/ns/ttml"><t:head/><t:body><t:div><t:div><t:p begin="1s" end="3s"><t:span><t:span begin="1s" end="2s">Hello</t:span> <t:span begin="2s" end="3s">world</t:span></t:span></t:p></t:div></t:div></t:body></t:tt>"#;
    let lines = parse_ttml(input.as_bytes()).unwrap().lines;
    assert_eq!(text(&lines[0]), "Hello world");
    assert_eq!(lines[0].words[2].start_time, 2_000);
}

#[test]
fn itunes_translation_and_word_romanization_are_keyed_to_vocals() {
    let input = r#"<tt xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:i="http://music.apple.com/lyric-ttml-internal">
    <head><metadata>
    <i:translations><i:translation xml:lang="zh-CN"><i:text for="L1">你好<span ttm:role="x-bg">（和声）</span></i:text></i:translation></i:translations>
    <i:transliterations><i:transliteration><i:text for="L1"><span begin="1s" end="2s">ni</span> <span begin="2s" end="3s">hao</span><span ttm:role="x-bg"><span begin="2s" end="3s">la</span></span></i:text></i:transliteration></i:transliterations>
    </metadata></head><body><div>
    <p i:key="L1" begin="1s" end="3s"><span begin="1s" end="2s">你</span><span ttm:role="x-bg"><span begin="2s" end="3s">啦</span></span><span begin="2s" end="3s">好</span></p>
    <p i:key="L2" begin="4s" end="5s">另一行</p></div></body></tt>"#;
    let lines = parse_ttml(input.as_bytes()).unwrap().lines;
    assert_eq!(lines[0].translated_lyric, "你好");
    assert_eq!(lines[1].translated_lyric, "和声");
    assert_eq!(lines[0].roman_lyric, "ni hao");
    assert_eq!(lines[0].words[0].roman_word, "ni");
    assert_eq!(lines[0].words[1].roman_word, "hao");
    assert_eq!(lines[1].words[0].roman_word, "la");
    assert!(lines[2].translated_lyric.is_empty());
}

#[test]
fn group_agents_are_not_duets_and_background_inherits_lead() {
    let input = r#"<tt xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><head><metadata><ttm:agent xml:id="v1" type="person"/><ttm:agent xml:id="v2" type="person"/><ttm:agent xml:id="v3" type="group"/></metadata></head><body><div><p ttm:agent="v2" begin="1s" end="2s">Solo<span ttm:role="x-bg">BG</span></p><p ttm:agent="v3" begin="3s" end="4s">Choir</p></div></body></tt>"#;
    let lines = parse_ttml(input.as_bytes()).unwrap().lines;
    assert!(lines[0].is_duet && lines[1].is_duet);
    assert!(!lines[2].is_duet);
}

#[test]
fn malformed_xml_and_times_fail_without_partial_lyrics_or_panics() {
    for input in [
        "<tt><body></tt>",
        "<tt><body>",
        "<tt/><tt/>",
        "<tt><body><p begin=\"NaNs\">bad</p></body></tt>",
        "<tt><body><p begin=\"18446744073709551615:59:59\">bad</p></body></tt>",
    ] {
        assert!(parse_ttml(input.as_bytes()).is_err(), "{input}");
    }
    let deep = format!("<tt>{}{}</tt>", "<div>".repeat(130), "</div>".repeat(130));
    assert!(parse_ttml(deep.as_bytes()).is_err());
}

#[test]
fn timestamps_and_duration_inheritance() {
    for (input, expected) in [
        ("01:02.345", 62_345),
        ("1.25s", 1_250),
        ("1250ms", 1_250),
        ("0.5m", 30_000),
        ("01:00:00", 3_600_000),
    ] {
        assert_eq!(parse_timestamp(input), Some(expected));
    }
    for input in ["1.中文", "1.2.3", "-1s", "NaN", ""] {
        assert!(parse_timestamp(input).is_none());
    }
    let lines = parse(r#"<p begin="1s" dur="2s"><span>Inherited</span></p>"#);
    assert_eq!(
        (lines[0].words[0].start_time, lines[0].words[0].end_time),
        (1_000, 3_000)
    );
}

#[test]
fn romanization_uses_overlap_when_onsets_differ() {
    let mut words = vec![LyricWordOwned {
        word: "你".into(),
        start_time: 1000,
        end_time: 1600,
        ..Default::default()
    }];
    let roman = vec![LyricWordOwned {
        word: "ni".into(),
        start_time: 1100,
        end_time: 1700,
        ..Default::default()
    }];
    align_roman_words(&mut words, &roman);
    assert_eq!(words[0].roman_word, "ni");
}

#[test]
fn ruby_annotations_are_word_romanization_not_duplicate_lyrics() {
    let lines = parse(
        r#"<p begin="1s" end="2s"><span xmlns:tts="http://www.w3.org/ns/ttml#styling" tts:ruby="container"><span tts:ruby="base">所</span><span tts:ruby="textContainer"><span tts:ruby="text" begin="1.2s" end="1.8s">しょ</span></span></span></p>"#,
    );
    assert_eq!(text(&lines[0]), "所");
    assert_eq!(lines[0].words[0].roman_word, "しょ");
    assert_eq!(
        (lines[0].words[0].start_time, lines[0].words[0].end_time),
        (1200, 1800)
    );
}
