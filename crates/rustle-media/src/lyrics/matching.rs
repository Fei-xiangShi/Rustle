//! Metadata identity checks performed before comparing lyric quality.

#[derive(Clone, Copy)]
pub struct SongIdentity<'a> {
    pub title: &'a str,
    pub artists: &'a str,
    pub album: &'a str,
    /// Zero denotes an unknown duration.
    pub duration_ms: u64,
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn contains_either(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.contains(b) || b.contains(a))
}

fn artists(text: &str) -> Vec<String> {
    text.split(['、', '&', ';', '，', ',', '/', '|', '·', '・'])
        .map(normalize)
        .filter(|part| !part.is_empty())
        .collect()
}

/// SPlayer-style identity gates followed by title/artist/album/duration ranking.
/// A high lyric quality score must never rescue a rejected song identity.
pub fn match_score(wanted: SongIdentity<'_>, candidate: SongIdentity<'_>) -> Option<u32> {
    let title = normalize(wanted.title);
    let other_title = normalize(candidate.title);
    if title.is_empty() || other_title.is_empty() {
        return None;
    }
    let exact_title = title == other_title;
    if !exact_title {
        let lengths = (title.chars().count(), other_title.chars().count());
        if !contains_either(&title, &other_title)
            || lengths.0.min(lengths.1) * 100 < lengths.0.max(lengths.1) * 34
        {
            return None;
        }
    }

    let duration_known = wanted.duration_ms > 0 && candidate.duration_ms > 0;
    let duration_diff = wanted.duration_ms.abs_diff(candidate.duration_ms);
    if duration_known && duration_diff > 20_000 {
        return None;
    }
    let close_duration = duration_known && duration_diff <= 5_000;

    let wanted_artists = artists(wanted.artists);
    let candidate_artists = artists(candidate.artists);
    let exact_artist = wanted_artists
        .iter()
        .any(|artist| candidate_artists.contains(artist));
    let contains_artist = !exact_artist
        && wanted_artists.iter().any(|artist| {
            candidate_artists.iter().any(|other| {
                artist.chars().count().min(other.chars().count()) >= 2
                    && contains_either(artist, other)
            })
        });
    if !wanted_artists.is_empty() && !exact_artist && !contains_artist {
        return None;
    }
    if !exact_title && !exact_artist && !contains_artist && !close_duration {
        return None;
    }

    let album = normalize(wanted.album);
    let mut score = if exact_title { 10 } else { 4 };
    score += if exact_artist {
        5
    } else if contains_artist {
        2
    } else {
        0
    };
    if !album.is_empty() && album == normalize(candidate.album) {
        score += 2;
    }
    if close_duration {
        score += 3;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> SongIdentity<'static> {
        SongIdentity {
            title: "夜曲",
            artists: "周杰伦",
            album: "十一月的萧邦",
            duration_ms: 226_000,
        }
    }

    #[test]
    fn rejects_same_title_from_unrelated_or_missing_artist() {
        for artist in ["其他歌手", "", "周"] {
            assert_eq!(
                match_score(
                    song(),
                    SongIdentity {
                        artists: artist,
                        ..song()
                    }
                ),
                None
            );
        }
    }

    #[test]
    fn matches_any_artist_regardless_of_order_and_punctuation() {
        let wanted = SongIdentity {
            artists: "甲歌手 / 乙歌手",
            ..song()
        };
        for candidate in ["乙歌手", "乙歌手、甲歌手", "甲歌手 & 另一位"] {
            assert!(
                match_score(
                    wanted,
                    SongIdentity {
                        artists: candidate,
                        ..song()
                    }
                )
                .is_some()
            );
        }
    }

    #[test]
    fn duration_is_a_gate_and_a_ranking_signal() {
        let exact = match_score(song(), song()).unwrap();
        for difference in [5_001, 12_000, 20_000] {
            let candidate = SongIdentity {
                duration_ms: song().duration_ms + difference,
                ..song()
            };
            assert!(match_score(song(), candidate).unwrap() < exact);
        }
        assert!(
            match_score(
                song(),
                SongIdentity {
                    duration_ms: 246_001,
                    ..song()
                }
            )
            .is_none()
        );
        assert!(
            match_score(
                song(),
                SongIdentity {
                    duration_ms: 0,
                    ..song()
                }
            )
            .is_some()
        );
    }

    #[test]
    fn title_substrings_need_length_and_corroboration() {
        assert!(
            match_score(
                song(),
                SongIdentity {
                    title: "夜曲只是这张专辑的一首歌曲",
                    ..song()
                }
            )
            .is_none()
        );
        let wanted = SongIdentity {
            title: "Hello",
            artists: "",
            duration_ms: 0,
            ..song()
        };
        let candidate = SongIdentity {
            title: "Hello Live",
            ..wanted
        };
        assert!(match_score(wanted, candidate).is_none());
        assert!(
            match_score(
                SongIdentity {
                    duration_ms: 100_000,
                    ..wanted
                },
                SongIdentity {
                    duration_ms: 101_000,
                    ..candidate
                }
            )
            .is_some()
        );
        assert!(
            match_score(
                SongIdentity {
                    title: "",
                    ..song()
                },
                song()
            )
            .is_none()
        );
    }
}
