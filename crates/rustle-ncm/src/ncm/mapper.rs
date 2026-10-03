use serde_json::Value;

use super::error::{NcmError, NcmResult as Result};
use super::models::*;

fn code_ok(value: &Value) -> bool {
    value.get("code").and_then(as_i64).unwrap_or(200) == 200
}

pub(super) fn confirmed_code_ok(value: &Value) -> bool {
    value.get("code").and_then(as_i64) == Some(200)
}

fn as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
}

fn as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
}

fn as_u32(value: &Value) -> Option<u32> {
    as_u64(value).and_then(|v| u32::try_from(v).ok())
}

fn as_i32(value: &Value) -> Option<i32> {
    as_i64(value).and_then(|v| i32::try_from(v).ok())
}

fn as_bool(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| match value.as_str()? {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    })
}

fn str_value(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn first_str(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn optional_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalized_https_url(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        String::new()
    } else if value.starts_with("https://") {
        value.to_string()
    } else if let Some(rest) = value.strip_prefix("http://") {
        format!("https://{rest}")
    } else if value.starts_with("//") {
        format!("https:{value}")
    } else {
        String::new()
    }
}

/// Normalize the CDN URL returned by NCM's playback endpoint before it is
/// handed to the range streamer. These are URL identity rewrites only: the
/// requested quality and the server-selected track are never changed.
fn normalized_audio_url(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }

    let value = value
        .strip_prefix("http://")
        .map(|rest| format!("https://{rest}"))
        .unwrap_or_else(|| value.to_string());

    value
        .replace("m804.music.126.net", "m801.music.126.net")
        .replace("m704.music.126.net", "m701.music.126.net")
}

fn vip_info_from_nodes(profile: Option<&Value>, root: Option<&Value>) -> VipInfo {
    let root = root.unwrap_or(&Value::Null);
    let profile = profile.unwrap_or(root);
    let rights = root.get("vipRights").or_else(|| profile.get("vipRights"));
    let associator = rights.and_then(|value| value.get("associator"));

    let vip_type = profile
        .get("vipType")
        .or_else(|| root.get("vipType"))
        .and_then(as_i32)
        .unwrap_or_default();
    let red_vip_level = rights
        .and_then(|value| value.get("redVipLevel"))
        .or_else(|| profile.get("redVipLevel"))
        .or_else(|| profile.get("vipLevel"))
        .or_else(|| root.get("redVipLevel"))
        .or_else(|| root.get("vipLevel"))
        .and_then(as_u32)
        .unwrap_or_default();
    let annual_count = root
        .get("redVipAnnualCount")
        .or_else(|| profile.get("redVipAnnualCount"))
        .or_else(|| root.get("annualCount"))
        .or_else(|| profile.get("annualCount"))
        .and_then(as_u32)
        .unwrap_or_default();
    let black_vinyl_icon_url = associator
        .and_then(|value| value.get("iconUrl"))
        .or_else(|| profile.get("vipIconUrl"))
        .or_else(|| root.get("vipIconUrl"))
        .and_then(Value::as_str)
        .map(normalized_https_url)
        .unwrap_or_default();
    VipInfo {
        vip_type,
        red_vip_level,
        annual_count,
        black_vinyl_icon_url,
        svip_icon_url: String::new(),
    }
}

/// Merge the official membership image payload into the account projection.
/// The front VIP endpoint is authoritative for badge imagery: associator is
/// Black Vinyl VIP and redplus is SVIP. Missing imagery stays missing; no text
/// or legacy URL is synthesized.
pub fn merge_membership_vip(base: &VipInfo, value: &Value) -> Result<VipInfo> {
    if !code_ok(value) {
        return Err(NcmError::business("VIP membership request failed"));
    }
    let data = value
        .get("data")
        .ok_or_else(|| NcmError::protocol("VIP membership payload missing data"))?;
    let icon_url = |section: &str| {
        data.get(section)
            .and_then(|node| node.get("iconUrl"))
            .and_then(Value::as_str)
            .map(normalized_https_url)
            .unwrap_or_default()
    };

    let mut info = base.clone();
    info.red_vip_level = data
        .get("redVipLevel")
        .and_then(as_u32)
        .unwrap_or(info.red_vip_level);
    info.annual_count = data
        .get("redVipAnnualCount")
        .and_then(as_u32)
        .unwrap_or(info.annual_count);
    info.black_vinyl_icon_url = icon_url("associator");
    info.svip_icon_url = icon_url("redplus");
    Ok(info)
}

fn timestamp_to_year(ts_ms: u64) -> Option<u32> {
    if ts_ms == 0 {
        return None;
    }
    let secs = ts_ms / 1000;
    let days = secs / 86400;
    Some(1970 + (days as f64 / 365.25) as u32)
}

fn artists_from_array(items: Option<&Vec<Value>>) -> Vec<ArtistSummary> {
    items
        .into_iter()
        .flatten()
        .filter_map(artist_summary_from_value)
        .collect()
}

pub fn artist_summary_from_value(value: &Value) -> Option<ArtistSummary> {
    let id = value.get("id").and_then(as_u64).unwrap_or_default();
    let name = str_value(value, "name");
    if id == 0 && name.is_empty() {
        return None;
    }

    Some(ArtistSummary {
        id,
        name,
        image_url: first_str(value, &["picUrl", "img1v1Url", "avatar"]),
    })
}

fn user_summary_from_value(value: Option<&Value>) -> UserSummary {
    value
        .map(|user| UserSummary {
            id: user.get("userId").and_then(as_u64).unwrap_or_default(),
            nickname: str_value(user, "nickname"),
            avatar_url: str_value(user, "avatarUrl"),
            vip: vip_info_from_nodes(Some(user), Some(user)),
        })
        .unwrap_or_default()
}

fn quality_option_from_value(level: NcmQualityLevel, value: &Value) -> Option<SongQualityOption> {
    if value.is_null() || value.as_bool() == Some(false) {
        return None;
    }

    let bitrate = value.get("br").and_then(as_u32).filter(|value| *value > 0);
    let size = value
        .get("size")
        .and_then(as_u64)
        .filter(|value| *value > 0);
    if bitrate.is_none() && size.is_none() {
        return None;
    }

    Some(SongQualityOption {
        level,
        bitrate,
        size,
        format: optional_string(value, &["format"]),
        file_id: value.get("fid").and_then(as_u64).filter(|value| *value > 0),
        sample_rate: value.get("sr").and_then(as_u32).filter(|value| *value > 0),
        bit_depth: value
            .get("bitDepth")
            .or_else(|| value.get("depth"))
            .and_then(as_u32)
            .filter(|value| *value > 0),
        channels: value
            .get("channel")
            .and_then(as_u32)
            .filter(|value| *value > 0),
        volume_delta: value.get("vd").and_then(Value::as_f64),
        effect_type: optional_string(value, &["it"]),
    })
}

fn quality_options_from_object(value: &Value) -> Vec<SongQualityOption> {
    let fields = ["l", "m", "h", "sq", "hr", "je", "sk", "db", "jm"];
    let mut options = fields
        .into_iter()
        .filter_map(|field| {
            let level = super::models::quality_from_api_field(field)?;
            value
                .get(field)
                .and_then(|node| quality_option_from_value(level, node))
        })
        .collect::<Vec<_>>();
    options.sort_by_key(|option| option.level.priority());
    options
}

pub fn album_summary_from_value(album: &Value) -> Option<AlbumSummary> {
    let id = album.get("id").and_then(as_u64).unwrap_or_default();
    let name = str_value(album, "name");
    if id == 0 && name.is_empty() {
        return None;
    }

    let artists = album
        .get("artists")
        .and_then(Value::as_array)
        .map(|items| artists_from_array(Some(items)))
        .or_else(|| {
            album
                .get("artist")
                .and_then(artist_summary_from_value)
                .map(|artist| vec![artist])
        })
        .unwrap_or_default();

    Some(AlbumSummary {
        id,
        name,
        image_url: first_str(album, &["picUrl", "pic_url", "coverImgUrl"]),
        artists,
        publish_time: album
            .get("publishTime")
            .and_then(as_u64)
            .unwrap_or_default(),
        tags: str_value(album, "tags"),
    })
}

pub fn playlist_summary_from_value(
    item: &Value,
    default_subscribed: bool,
) -> Option<PlaylistSummary> {
    let id = item.get("id").and_then(as_u64)?;
    Some(PlaylistSummary {
        id,
        name: str_value(item, "name"),
        cover_url: first_str(item, &["coverImgUrl", "picUrl", "coverUrl"]),
        creator: user_summary_from_value(item.get("creator")),
        subscribed: item
            .get("subscribed")
            .and_then(as_bool)
            .unwrap_or(default_subscribed),
    })
}

pub fn track_from_value(track: &Value, album_override: Option<&Value>) -> Result<Track> {
    let artists = {
        let artists = artists_from_array(track.get("ar").and_then(Value::as_array));
        if artists.is_empty() {
            artists_from_array(track.get("artists").and_then(Value::as_array))
        } else {
            artists
        }
    };

    let album_value = album_override
        .or_else(|| track.get("al"))
        .or_else(|| track.get("album"));
    let mut album = album_value
        .and_then(album_summary_from_value)
        .unwrap_or_default();
    if album.artists.is_empty() {
        album.artists = artists.clone();
    }

    let publish_ts = track
        .get("publishTime")
        .and_then(as_u64)
        .or_else(|| {
            album_value
                .and_then(|album| album.get("publishTime"))
                .and_then(as_u64)
        })
        .unwrap_or(album.publish_time);
    let privilege = track.get("privilege");

    Ok(Track {
        id: track
            .get("id")
            .and_then(as_u64)
            .ok_or_else(|| NcmError::protocol("track id missing"))?,
        title: str_value(track, "name"),
        artists,
        album,
        duration_ms: track
            .get("dt")
            .or_else(|| track.get("duration"))
            .and_then(as_u64)
            .unwrap_or_default(),
        availability: privilege
            .map(|value| {
                let st = value.get("st").and_then(as_i32).unwrap_or_default();
                let fee = value.get("fee").and_then(as_i32).unwrap_or_default();
                TrackAvailability::from_privilege(st, fee)
            })
            .unwrap_or(TrackAvailability::Unknown),
        track_number: track.get("no").and_then(as_u32),
        year: timestamp_to_year(publish_ts),
        genre: album_value
            .and_then(|album| album.get("tags"))
            .and_then(Value::as_str)
            .filter(|tags| !tags.is_empty())
            .map(ToString::to_string),
        quality_options: quality_options_from_object(track),
    })
}

pub fn tracks_from_array(items: Option<&Vec<Value>>, album_override: Option<&Value>) -> Vec<Track> {
    items
        .into_iter()
        .flatten()
        .filter_map(|track| track_from_value(track, album_override).ok())
        .collect()
}

fn playback_quality(item: &Value) -> Option<NcmQualityLevel> {
    match item.get("level") {
        Some(Value::String(level)) if !level.is_empty() => {
            return super::models::quality_from_api_level(level);
        }
        None | Some(Value::Null) | Some(Value::String(_)) => {}
        _ => return None,
    }
    // DJ audio can return a usable URL with level=null. Infer only regular
    // lossy quality from the actual codec/bitrate, never from the user's
    // requested Hi-Res/Dolby level (which would also poison cache identity).
    let format = item.get("type")?.as_str()?;
    if !format.eq_ignore_ascii_case("mp3") && !format.eq_ignore_ascii_case("aac") {
        return None;
    }
    match item.get("br").and_then(as_u32)? {
        1..=128_000 => Some(NcmQualityLevel::Standard),
        128_001..=192_000 => Some(NcmQualityLevel::Higher),
        192_001..=320_000 => Some(NcmQualityLevel::ExHigh),
        _ => None,
    }
}

pub fn track_urls(value: &Value, requested_level: NcmQualityLevel) -> Result<Vec<TrackUrl>> {
    if !code_ok(value) {
        return Err(NcmError::business("track URL request failed"));
    }
    Ok(value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("id").and_then(as_u64)?;
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .map(normalized_audio_url)
                .unwrap_or_default();
            let level = playback_quality(item)?;
            (!url.is_empty()).then(|| TrackUrl {
                id,
                url,
                requested_level,
                level,
                rate: item.get("br").and_then(as_u32).unwrap_or_default(),
                size: item.get("size").and_then(as_u64).filter(|value| *value > 0),
                format: optional_string(item, &["type"]),
                sample_rate: item.get("sr").and_then(as_u32).filter(|value| *value > 0),
                bit_depth: item
                    .get("bitDepth")
                    .and_then(as_u32)
                    .filter(|value| *value > 0),
                channels: item
                    .get("channel")
                    .and_then(as_u32)
                    .filter(|value| *value > 0),
                channel_layout: optional_string(item, &["channelLayout"]),
                immerse_type: optional_string(item, &["immerseType"]),
            })
        })
        .collect())
}

pub fn song_quality_detail(value: &Value, song_id: u64) -> Result<SongQualityDetail> {
    if !code_ok(value) {
        return Err(NcmError::business("song quality detail request failed"));
    }
    let data = value.get("data").unwrap_or(value);
    let mut options = quality_options_from_object(data);

    options.sort_by_key(|option| option.level.priority());
    options.dedup_by_key(|option| option.level);
    let highest_available = options
        .iter()
        .max_by_key(|option| option.level.priority())
        .map(|option| option.level);
    Ok(SongQualityDetail {
        song_id,
        options,
        highest_available,
    })
}

pub fn track_detail(value: &Value) -> Result<Vec<Track>> {
    if !code_ok(value) {
        return Err(NcmError::business("track detail request failed"));
    }
    Ok(tracks_from_array(
        value.get("songs").and_then(Value::as_array),
        None,
    ))
}

pub fn lyrics(value: &Value) -> Result<Lyrics> {
    if !code_ok(value) {
        return Err(NcmError::business("lyrics request failed"));
    }

    let split_lyric = |key: &str| {
        value
            .get(key)
            .and_then(|node| node.get("lyric"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect()
    };

    Ok(Lyrics {
        lyric: split_lyric("lrc"),
        tlyric: split_lyric("tlyric"),
        yrc: split_lyric("yrc"),
        ytlrc: split_lyric("ytlrc"),
        romalrc: split_lyric("romalrc"),
        yromalrc: split_lyric("yromalrc"),
    })
}

#[cfg(test)]
mod lyric_tests {
    use super::lyrics;
    use serde_json::json;

    #[test]
    fn maps_v1_word_level_lyrics_and_attributes() {
        let value = json!({
            "code": 200,
            "lrc": { "lyric": "[00:00.00]fallback" },
            "tlyric": { "lyric": "[00:00.00]翻译" },
            "yrc": { "lyric": "[0,1000](0,500,0)逐(500,500,0)字" },
            "ytlrc": { "lyric": "[0,1000]translation" },
            "romalrc": { "lyric": "[00:00.00]roman" },
            "yromalrc": { "lyric": "[0,1000](0,1000,0)roman" }
        });

        let mapped = lyrics(&value).expect("valid lyrics response");
        assert_eq!(mapped.yrc, vec!["[0,1000](0,500,0)逐(500,500,0)字"]);
        assert_eq!(mapped.ytlrc, vec!["[0,1000]translation"]);
        assert_eq!(mapped.romalrc, vec!["[00:00.00]roman"]);
        assert_eq!(mapped.yromalrc, vec!["[0,1000](0,1000,0)roman"]);
    }

    #[test]
    fn tolerates_missing_word_level_fields() {
        let value = json!({
            "code": 200,
            "lrc": { "lyric": "[00:00.00]line" }
        });

        let mapped = lyrics(&value).expect("valid lyrics response");
        assert_eq!(mapped.lyric, vec!["[00:00.00]line"]);
        assert!(mapped.yrc.is_empty());
    }
}

/// Parse the legacy `/song/url` response used by SPlayer for Dolby playback.
/// The endpoint does not return a semantic quality level, so its requested
/// Dolby level is also the server-declared actual level for this API path.
pub fn legacy_track_urls(value: &Value, requested_level: NcmQualityLevel) -> Result<Vec<TrackUrl>> {
    if !code_ok(value) {
        return Err(NcmError::business("legacy track URL request failed"));
    }
    Ok(value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("id").and_then(as_u64)?;
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .map(normalized_audio_url)
                .unwrap_or_default();
            (!url.is_empty()).then(|| TrackUrl {
                id,
                url,
                requested_level,
                level: requested_level,
                rate: item.get("br").and_then(as_u32).unwrap_or_default(),
                size: item.get("size").and_then(as_u64).filter(|value| *value > 0),
                format: optional_string(item, &["type"]),
                sample_rate: item.get("sr").and_then(as_u32).filter(|value| *value > 0),
                bit_depth: item
                    .get("bitDepth")
                    .and_then(as_u32)
                    .filter(|value| *value > 0),
                channels: item
                    .get("channel")
                    .and_then(as_u32)
                    .filter(|value| *value > 0),
                channel_layout: optional_string(item, &["channelLayout"]),
                immerse_type: optional_string(item, &["immerseType"]),
            })
        })
        .collect())
}

#[cfg(test)]
mod quality_tests {
    use super::{
        legacy_track_urls, login_info, merge_membership_vip, normalized_audio_url,
        song_quality_detail, track_urls,
    };
    use crate::ncm::models::{NcmQualityLevel, VipTier};
    use serde_json::json;

    #[test]
    fn maps_all_quality_levels_and_selects_highest() {
        let value = json!({
            "code": 200,
            "data": {
                "l": {"br": 128000, "size": 1000},
                "m": {"br": 192000, "size": 2000},
                "h": {"br": 320000, "size": 3000},
                "sq": {"br": 999000, "size": 4000},
                "hr": {"br": 1900000, "size": 5000},
                "je": {"br": 2000000, "size": 6000},
                "sk": {"br": 2100000, "size": 7000},
                "db": {"br": 2200000, "size": 8000},
                "jm": {"br": 2400000, "size": 9000}
            }
        });
        let detail = song_quality_detail(&value, 42).expect("quality detail");
        assert_eq!(detail.options.len(), 9);
        assert_eq!(detail.highest_available, Some(NcmQualityLevel::JyMaster));
        assert_eq!(
            detail
                .best_for(NcmQualityLevel::Dolby)
                .expect("dolby")
                .level,
            NcmQualityLevel::Dolby
        );
    }

    #[test]
    fn ignores_empty_quality_and_preserves_server_negotiation() {
        let detail = song_quality_detail(
            &json!({
                "code": 200,
                "data": {
                    "sq": {"br": 0, "size": 0},
                    "h": {"br": 320000, "size": 1234}
                }
            }),
            7,
        )
        .expect("quality detail");
        assert_eq!(detail.options.len(), 1);

        let urls = track_urls(
            &json!({
                "code": 200,
                "data": [{"id": 7, "url": "https://cdn/song.mp3", "level": "exhigh", "br": 320000}]
            }),
            NcmQualityLevel::Lossless,
        )
        .expect("url response");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].requested_level, NcmQualityLevel::Lossless);
        assert_eq!(urls[0].level, NcmQualityLevel::ExHigh);

        let non_wire_fields = song_quality_detail(
            &json!({
                "code": 200,
                "data": {"lossless": {"br": 128000}, "master": {"br": 999000}}
            }),
            8,
        )
        .expect("quality detail");
        assert!(non_wire_fields.options.is_empty());
    }

    #[test]
    fn rejects_failed_quality_detail_and_unidentified_url() {
        assert!(song_quality_detail(&json!({"code": 500}), 7).is_err());

        let urls = track_urls(
            &json!({
                "code": 200,
                "data": [{"url": "https://cdn/song.mp3", "level": "exhigh"}]
            }),
            NcmQualityLevel::ExHigh,
        )
        .expect("url response");
        assert!(urls.is_empty());
    }

    #[test]
    fn podcast_urls_with_null_level_preserve_actual_lossy_quality() {
        for (bitrate, expected) in [
            (128_000, NcmQualityLevel::Standard),
            (192_000, NcmQualityLevel::Higher),
            (320_000, NcmQualityLevel::ExHigh),
        ] {
            let urls = track_urls(
                &json!({"code": 200, "data": [{
                    "id": 556929063, "code": 200, "url": "https://cdn/program.mp3",
                    "level": null, "br": bitrate, "type": "mp3", "size": 1024
                }]}),
                NcmQualityLevel::HiRes,
            )
            .unwrap();
            assert_eq!(urls.len(), 1);
            assert_eq!(urls[0].id, 556929063);
            assert_eq!(urls[0].requested_level, NcmQualityLevel::HiRes);
            assert_eq!(urls[0].level, expected);
            assert_eq!(urls[0].rate, bitrate);
        }
        let urls = track_urls(
            &json!({"code": 200, "data": [{
                "id": 7, "url": "https://cdn/program.aac", "br": 128000, "type": "aac"
            }]}),
            NcmQualityLevel::HiRes,
        )
        .unwrap();
        assert_eq!(urls[0].level, NcmQualityLevel::Standard);
    }

    #[test]
    fn missing_level_fallback_does_not_invent_urls_or_premium_quality() {
        let urls = track_urls(&json!({"code": 200, "data": [
            {"id": 1, "url": null, "level": null, "br": 320000, "type": "mp3"},
            {"id": 2, "url": "https://cdn/audio", "level": null, "br": 0, "type": "mp3"},
            {"id": 3, "url": "https://cdn/audio", "level": null, "br": 999000, "type": "flac"},
            {"id": 4, "url": "https://cdn/audio", "level": "future-quality", "br": 320000, "type": "mp3"}
        ]}), NcmQualityLevel::HiRes).unwrap();
        assert!(urls.is_empty());
    }

    #[test]
    fn normalizes_ncm_stream_cdn_without_changing_quality_contract() {
        assert_eq!(
            normalized_audio_url("http://m804.music.126.net/path/file.flac?auth=1"),
            "https://m801.music.126.net/path/file.flac?auth=1"
        );
        assert_eq!(
            normalized_audio_url("https://m704.music.126.net/path/file.flac"),
            "https://m701.music.126.net/path/file.flac"
        );
    }

    #[test]
    fn parses_legacy_dolby_urls_without_a_level_field() {
        let urls = legacy_track_urls(
            &json!({
                "code": 200,
                "data": [{"id": 42, "url": "http://m7.music.126.net/song.flac", "br": 999000}]
            }),
            NcmQualityLevel::Dolby,
        )
        .expect("legacy url response");
        assert_eq!(urls[0].requested_level, NcmQualityLevel::Dolby);
        assert_eq!(urls[0].level, NcmQualityLevel::Dolby);
        assert_eq!(urls[0].url, "https://m7.music.126.net/song.flac");
    }

    #[test]
    fn maps_vip_level_annual_flag_and_https_icon() {
        let login = login_info(&json!({
            "code": 200,
            "profile": {
                "userId": 9,
                "nickname": "tester",
                "avatarUrl": "https://avatar",
                "vipType": 11,
                "vipRights": {
                    "redVipLevel": 7,
                    "associator": {"iconUrl": "http://vip/icon.png"}
                },
                "redVipAnnualCount": 1
            }
        }))
        .expect("login info");
        assert_eq!(login.vip.red_vip_level, 7);
        assert_eq!(login.vip.annual_count, 1);
        assert_eq!(login.vip.black_vinyl_icon_url, "https://vip/icon.png");
        assert_eq!(login.vip.tier(), VipTier::Svip);
    }

    #[test]
    fn maps_membership_images_to_black_vinyl_and_svip_tiers() {
        let base = login_info(&json!({
            "code": 200,
            "profile": {"userId": 9, "vipType": 11}
        }))
        .expect("login info")
        .vip;
        let merged = merge_membership_vip(
            &base,
            &json!({
                "code": 200,
                "data": {
                    "redVipLevel": 7,
                    "associator": {"iconUrl": "http://vip/black.png"},
                    "redplus": {"iconUrl": "http://vip/svip.png"}
                }
            }),
        )
        .expect("membership info");
        assert_eq!(merged.tier(), VipTier::Svip);
        assert_eq!(merged.black_vinyl_icon_url, "https://vip/black.png");
        assert_eq!(merged.svip_icon_url, "https://vip/svip.png");
        assert_eq!(merged.badge_url(), Some("https://vip/svip.png"));
    }
}

pub fn playlist_detail(value: &Value) -> Result<PlaylistDetail> {
    if !code_ok(value) {
        return Err(NcmError::business("playlist detail request failed"));
    }
    let playlist = value
        .get("playlist")
        .ok_or_else(|| NcmError::protocol("playlist missing"))?;
    let tracks = playlist.get("tracks").and_then(Value::as_array);
    let privileges = value.get("privileges").and_then(Value::as_array);

    let mut mapped_tracks = Vec::new();
    for (idx, track) in tracks.into_iter().flatten().enumerate() {
        let mut mapped = track_from_value(track, None)?;
        if let Some(privilege) = privileges.and_then(|items| items.get(idx)) {
            let st = privilege.get("st").and_then(as_i32).unwrap_or_default();
            let fee = privilege.get("fee").and_then(as_i32).unwrap_or_default();
            mapped.availability = TrackAvailability::from_privilege(st, fee);
        }
        mapped_tracks.push(mapped);
    }

    Ok(PlaylistDetail {
        id: playlist.get("id").and_then(as_u64).unwrap_or_default(),
        name: str_value(playlist, "name"),
        cover_url: str_value(playlist, "coverImgUrl"),
        description: str_value(playlist, "description"),
        create_time: playlist
            .get("createTime")
            .and_then(as_u64)
            .unwrap_or_default(),
        track_update_time: playlist
            .get("trackUpdateTime")
            .and_then(as_u64)
            .unwrap_or_default(),
        creator: user_summary_from_value(playlist.get("creator")),
        track_count: playlist
            .get("trackCount")
            .and_then(as_u64)
            .unwrap_or(mapped_tracks.len() as u64),
        subscribed: playlist
            .get("subscribed")
            .and_then(as_bool)
            .unwrap_or_default(),
        tracks: mapped_tracks,
    })
}

pub fn playlist_track_ids(value: &Value) -> Vec<u64> {
    value
        .get("playlist")
        .and_then(|playlist| playlist.get("trackIds"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|track| track.get("id").and_then(as_u64))
        .collect()
}

pub fn artist_detail(value: &Value) -> Result<ArtistDetail> {
    if !code_ok(value) {
        return Err(NcmError::business("artist detail request failed"));
    }

    let artist = value
        .get("artist")
        .or_else(|| value.get("data").and_then(|data| data.get("artist")))
        .or_else(|| value.get("data"))
        .ok_or_else(|| NcmError::protocol("artist missing"))?;

    let tracks = tracks_from_array(value.get("hotSongs").and_then(Value::as_array), None);

    Ok(ArtistDetail {
        id: artist.get("id").and_then(as_u64).unwrap_or_default(),
        name: str_value(artist, "name"),
        image_url: first_str(artist, &["picUrl", "avatar", "img1v1Url"]),
        description: artist
            .get("briefDesc")
            .or_else(|| artist.get("desc"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        track_count: artist
            .get("musicSize")
            .and_then(as_u32)
            .unwrap_or(tracks.len() as u32),
        album_count: artist.get("albumSize").and_then(as_u32).unwrap_or_default(),
        mv_count: artist.get("mvSize").and_then(as_u32).unwrap_or_default(),
        followed: artist.get("followed").and_then(as_bool).unwrap_or_default(),
        top_tracks: tracks,
    })
}

pub fn album_detail(value: &Value) -> Result<AlbumDetail> {
    if !code_ok(value) {
        return Err(NcmError::business("album detail request failed"));
    }

    let album = value
        .get("album")
        .ok_or_else(|| NcmError::protocol("album missing"))?;
    let summary = album_summary_from_value(album).unwrap_or_default();
    let tracks = tracks_from_array(value.get("songs").and_then(Value::as_array), Some(album));

    Ok(AlbumDetail {
        id: summary.id,
        name: summary.name,
        image_url: summary.image_url,
        description: album
            .get("description")
            .or_else(|| album.get("briefDesc"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        artists: summary.artists,
        track_count: album
            .get("size")
            .and_then(as_u32)
            .unwrap_or(tracks.len() as u32),
        publish_time: summary.publish_time,
        tracks,
        tags: summary.tags,
    })
}

pub fn user_detail(value: &Value) -> Result<UserDetail> {
    if !code_ok(value) {
        return Err(NcmError::business("user detail request failed"));
    }
    let profile = value
        .get("profile")
        .ok_or_else(|| NcmError::protocol("profile missing"))?;

    Ok(UserDetail {
        user_id: profile.get("userId").and_then(as_u64).unwrap_or_default(),
        artist_id: profile.get("artistId").and_then(as_u64).unwrap_or_default(),
        nickname: str_value(profile, "nickname"),
        artist_name: str_value(profile, "artistName"),
        signature: str_value(profile, "signature"),
        follows: profile.get("follows").and_then(as_u64).unwrap_or_default(),
        followeds: profile
            .get("followeds")
            .and_then(as_u64)
            .unwrap_or_default(),
        avatar_url: str_value(profile, "avatarUrl"),
        background_url: str_value(profile, "backgroundUrl"),
        vip: vip_info_from_nodes(Some(profile), Some(value)),
    })
}

pub fn playlist_summaries(value: &Value, source: PlaylistSource) -> Result<Vec<PlaylistSummary>> {
    if !code_ok(value) {
        return Err(NcmError::business("playlist summary request failed"));
    }

    let items = match source {
        PlaylistSource::User => value.get("playlist").and_then(Value::as_array),
        PlaylistSource::Recommend => value
            .get("recommend")
            .or_else(|| value.get("data"))
            .and_then(Value::as_array),
        PlaylistSource::Top => value.get("playlists").and_then(Value::as_array),
        PlaylistSource::Search => value
            .get("result")
            .and_then(|result| result.get("playlists"))
            .and_then(Value::as_array),
    };

    Ok(items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            playlist_summary_from_value(item, matches!(source, PlaylistSource::User))
        })
        .collect())
}

#[derive(Debug, Clone, Copy)]
pub enum PlaylistSource {
    User,
    Recommend,
    Top,
    Search,
}

#[cfg(test)]
mod playlist_summary_tests {
    use serde_json::json;

    use super::{PlaylistSource, playlist_summaries};

    #[test]
    fn maps_high_quality_playlist_response_shape() {
        let value = json!({
            "code": 200,
            "playlists": [{
                "id": 42,
                "name": "Official Selection",
                "coverImgUrl": "https://example.com/cover.jpg",
                "creator": {
                    "userId": 7,
                    "nickname": "NCM Editor"
                }
            }]
        });

        let playlists = playlist_summaries(&value, PlaylistSource::Top)
            .expect("high-quality playlists should map");

        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].id, 42);
        assert_eq!(playlists[0].creator.nickname, "NCM Editor");
    }
}

pub fn album_summaries(value: &Value, source: AlbumSource) -> Result<Vec<AlbumSummary>> {
    if !code_ok(value) {
        return Err(NcmError::business("album summary request failed"));
    }

    let items = match source {
        AlbumSource::Search => value
            .get("result")
            .and_then(|result| result.get("albums"))
            .and_then(Value::as_array),
        AlbumSource::Artist => value.get("hotAlbums").and_then(Value::as_array),
    };

    Ok(items
        .into_iter()
        .flatten()
        .filter_map(album_summary_from_value)
        .collect())
}

#[derive(Debug, Clone, Copy)]
pub enum AlbumSource {
    Search,
    Artist,
}

pub fn artist_summaries(value: &Value) -> Result<Vec<ArtistSummary>> {
    if !code_ok(value) {
        return Err(NcmError::business("artist summary request failed"));
    }

    Ok(value
        .get("result")
        .and_then(|result| result.get("artists"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(artist_summary_from_value)
        .collect())
}

fn video_summary_from_value(value: &Value) -> Option<VideoSummary> {
    let id = value.get("id").and_then(as_u64).unwrap_or_default();
    let name = first_str(value, &["name", "title"]);
    if id == 0 && name.is_empty() {
        return None;
    }

    let artist_name = first_str(value, &["artistName", "artist"]);
    let duration_ms = value
        .get("duration")
        .or_else(|| value.get("durationms"))
        .and_then(as_u64)
        .unwrap_or_default();

    Some(VideoSummary {
        id,
        name,
        cover_url: first_str(value, &["coverUrl", "cover", "picUrl"]),
        artist_name,
        duration_ms,
        play_count: value
            .get("playCount")
            .or_else(|| value.get("playCountStr"))
            .and_then(as_u64)
            .unwrap_or_default(),
    })
}

fn video_summaries(value: &Value) -> Vec<VideoSummary> {
    value
        .get("result")
        .and_then(|result| result.get("mvs"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(video_summary_from_value)
        .collect()
}

fn radio_summary_from_value(value: &Value) -> Option<RadioSummary> {
    let id = value.get("id").and_then(as_u64).unwrap_or_default();
    let name = str_value(value, "name");
    if id == 0 && name.is_empty() {
        return None;
    }

    Some(RadioSummary {
        id,
        name,
        cover_url: first_str(value, &["picUrl", "coverUrl"]),
        creator: user_summary_from_value(value.get("dj").or_else(|| value.get("creator"))),
        category: first_str(value, &["category", "categoryName"]),
        program_count: value
            .get("programCount")
            .and_then(as_u32)
            .unwrap_or_default(),
    })
}

fn radio_summaries(value: &Value) -> Vec<RadioSummary> {
    value
        .get("result")
        .and_then(|result| result.get("djRadios"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(radio_summary_from_value)
        .collect()
}

pub fn liked_song_ids(value: &Value) -> Result<Vec<u64>> {
    if !code_ok(value) {
        return Err(NcmError::business("liked song IDs request failed"));
    }
    Ok(value
        .get("ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(as_u64)
        .collect())
}

pub fn login_info(value: &Value) -> Result<LoginInfo> {
    let code = value.get("code").and_then(as_i32).unwrap_or_default();
    if code != 200 {
        return Ok(LoginInfo {
            code,
            user_id: 0,
            nickname: String::new(),
            avatar_url: String::new(),
            vip_type: 0,
            vip: VipInfo::default(),
            msg: value
                .get("msg")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }

    let profile = value
        .get("profile")
        .ok_or_else(|| NcmError::protocol("login profile missing"))?;

    let vip = vip_info_from_nodes(Some(profile), Some(value));
    Ok(LoginInfo {
        code,
        user_id: profile.get("userId").and_then(as_u64).unwrap_or_default(),
        nickname: str_value(profile, "nickname"),
        avatar_url: str_value(profile, "avatarUrl"),
        vip_type: vip.vip_type,
        vip,
        msg: String::new(),
    })
}

pub fn account_info(value: &Value) -> Result<LoginInfo> {
    if !code_ok(value) {
        return Err(NcmError::business("account info request failed"));
    }
    let profile = value
        .get("profile")
        .or_else(|| value.get("userProfile"))
        .ok_or_else(|| NcmError::protocol("account profile missing"))?;
    let vip = vip_info_from_nodes(Some(profile), Some(value));
    Ok(LoginInfo {
        code: 200,
        user_id: profile.get("userId").and_then(as_u64).unwrap_or_default(),
        nickname: str_value(profile, "nickname"),
        avatar_url: str_value(profile, "avatarUrl"),
        vip_type: vip.vip_type,
        vip,
        msg: String::new(),
    })
}

pub fn msg(value: &Value) -> Msg {
    Msg {
        code: value.get("code").and_then(as_i32).unwrap_or_default(),
        msg: value
            .get("msg")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

pub fn login_status_has_authenticated_account(value: &Value) -> bool {
    if !confirmed_code_ok(value) || value.get("anonimous").and_then(as_bool).unwrap_or(false) {
        return false;
    }
    let account = value.get("account");
    let profile = value.get("profile");
    let anonymous = account
        .and_then(|value| value.get("anonimous"))
        .and_then(as_bool)
        .or_else(|| {
            profile
                .and_then(|value| value.get("anonimous"))
                .and_then(as_bool)
        })
        .unwrap_or(false);
    let user_id = account
        .and_then(|value| value.get("id").or_else(|| value.get("userId")))
        .and_then(as_u64)
        .or_else(|| {
            profile
                .and_then(|value| value.get("userId").or_else(|| value.get("id")))
                .and_then(as_u64)
        })
        .unwrap_or_default();
    !anonymous && user_id > 0
}

pub fn like_result(value: &Value, liked: bool) -> Result<LikeResult> {
    if !confirmed_code_ok(value) {
        return Err(NcmError::business("like mutation failed"));
    }
    Ok(LikeResult {
        liked,
        playlist_id: value.get("playlistId").and_then(as_u64),
        protocol: LikeProtocol::LegacyWeapi,
    })
}

pub fn scrobble_stage_confirmed(value: &Value) -> bool {
    confirmed_code_ok(value) || value.get("data").and_then(Value::as_str) == Some("success")
}

pub fn playlist_track_mutation(
    value: &Value,
    operation: PlaylistTrackOperation,
    playlist_id: u64,
    requested_track_ids: &[u64],
    retried_after_code_512: bool,
) -> Result<PlaylistTrackMutation> {
    if !confirmed_code_ok(value) {
        return Err(NcmError::business("playlist track mutation failed"));
    }
    let mut returned_track_ids = mutation_track_ids(value.get("trackIds"));
    returned_track_ids.sort_unstable();
    returned_track_ids.dedup();
    let reported_count = value.get("count").and_then(as_u64).unwrap_or_default();
    let changed_count = reported_count.max(returned_track_ids.len() as u64);
    if changed_count == 0 {
        return Err(NcmError::business(
            "playlist track mutation reported no changed tracks",
        ));
    }
    Ok(PlaylistTrackMutation {
        operation,
        playlist_id,
        requested_track_ids: requested_track_ids.to_vec(),
        returned_track_ids,
        changed_count,
        cloud_count: value.get("cloudCount").and_then(as_u64),
        retried_after_code_512,
    })
}

fn mutation_track_ids(value: Option<&Value>) -> Vec<u64> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(items) = value.as_array() {
        return items.iter().filter_map(as_u64).collect();
    }
    if let Some(serialized) = value.as_str()
        && let Ok(parsed) = serde_json::from_str::<Value>(serialized)
    {
        return mutation_track_ids(Some(&parsed));
    }
    Vec::new()
}

pub fn search(value: &Value, search_type: SearchType) -> Result<SearchResponse> {
    if !code_ok(value) {
        return Err(NcmError::business("search request failed"));
    }
    let result = value.get("result").unwrap_or(&Value::Null);
    let mut response = SearchResponse::default();

    match search_type {
        SearchType::Songs => {
            response.track_count = result.get("songCount").and_then(as_u32).unwrap_or_default();
            response.tracks =
                tracks_from_array(result.get("songs").and_then(Value::as_array), None);
        }
        SearchType::Albums => {
            response.album_count = result
                .get("albumCount")
                .and_then(as_u32)
                .unwrap_or_default();
            response.albums = album_summaries(value, AlbumSource::Search)?;
        }
        SearchType::Artists => {
            response.artist_count = result
                .get("artistCount")
                .and_then(as_u32)
                .unwrap_or_default();
            response.artists = artist_summaries(value)?;
        }
        SearchType::Playlists => {
            response.playlist_count = result
                .get("playlistCount")
                .and_then(as_u32)
                .unwrap_or_default();
            response.playlists = playlist_summaries(value, PlaylistSource::Search)?;
        }
        SearchType::Videos => {
            response.video_count = result.get("mvCount").and_then(as_u32).unwrap_or_default();
            response.videos = video_summaries(value);
        }
        SearchType::Radios => {
            response.radio_count = result
                .get("djRadiosCount")
                .and_then(as_u32)
                .unwrap_or_default();
            response.radios = radio_summaries(value);
        }
    }

    Ok(response)
}

#[cfg(test)]
mod endpoint_contract_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn login_status_requires_a_non_anonymous_account_identity() {
        assert!(login_status_has_authenticated_account(&json!({
            "code": 200,
            "account": { "id": 42 },
            "profile": { "userId": 42 }
        })));
        assert!(!login_status_has_authenticated_account(&json!({
            "code": 200,
            "account": { "id": 42, "anonimous": true }
        })));
        assert!(!login_status_has_authenticated_account(&json!({
            "code": 200,
            "account": null,
            "profile": null
        })));
        assert!(login_status_has_authenticated_account(&json!({
            "code": "200",
            "account": { "id": "42" }
        })));
        assert!(!login_status_has_authenticated_account(&json!({
            "account": { "id": 42 }
        })));
    }

    #[test]
    fn like_result_preserves_requested_state_and_playlist_identity() {
        let result = like_result(&json!({ "code": 200, "playlistId": "99" }), true).unwrap();
        assert!(result.liked);
        assert_eq!(result.playlist_id, Some(99));
        assert_eq!(result.protocol, LikeProtocol::LegacyWeapi);
        assert!(like_result(&json!({ "code": "200" }), false).is_ok());
        assert!(like_result(&json!({}), true).is_err());
        assert!(like_result(&json!({ "code": 502 }), true).is_err());
    }

    #[test]
    fn scrobble_stage_accepts_only_confirmed_success() {
        assert!(scrobble_stage_confirmed(&json!({ "code": 200 })));
        assert!(scrobble_stage_confirmed(&json!({ "code": "200" })));
        assert!(scrobble_stage_confirmed(&json!({ "data": "success" })));
        assert!(!scrobble_stage_confirmed(&json!({})));
        assert!(!scrobble_stage_confirmed(&json!({ "code": 502 })));
    }

    #[test]
    fn playlist_mutation_requires_actual_change_and_parses_string_ids() {
        let result = playlist_track_mutation(
            &json!({
                "code": 200,
                "count": 1,
                "trackIds": "[\"8\", 8]",
                "cloudCount": "3"
            }),
            PlaylistTrackOperation::Add,
            7,
            &[8],
            true,
        )
        .unwrap();
        assert_eq!(result.returned_track_ids, vec![8]);
        assert_eq!(result.changed_count, 1);
        assert_eq!(result.cloud_count, Some(3));
        assert!(result.retried_after_code_512);

        assert!(
            playlist_track_mutation(
                &json!({ "count": 1, "trackIds": [8] }),
                PlaylistTrackOperation::Add,
                7,
                &[8],
                false,
            )
            .is_err()
        );

        assert!(
            playlist_track_mutation(
                &json!({ "code": 200, "count": 0, "trackIds": [] }),
                PlaylistTrackOperation::Delete,
                7,
                &[8],
                false,
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod search_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn web_suggestions_preserve_category_identity_and_ignore_malformed_items() {
        let items = search_suggestions(&json!({"code": 200, "result": {
            "songs": [{"id": 7, "name": "Song", "artists": [{"name": "A"}, {"name": "B"}]},
                {"name": "missing ID"}, {"id": 0, "name": "invalid"}],
            "artists": [{"id": 7, "name": "Artist"}],
            "albums": [{"id": 8, "name": "Album", "artist": {"name": "A"}}],
            "playlists": null
        }}))
        .unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, SearchType::Songs);
        assert_eq!(items[0].subtitle, "A / B");
        assert_eq!(items[1].kind, SearchType::Artists);
        assert_eq!(items[2].subtitle, "A");
        assert!(
            search_suggestions(&json!({"code": 200}))
                .unwrap()
                .is_empty()
        );
        assert!(search_suggestions(&json!({"code": 500})).is_err());
    }

    #[test]
    fn radio_programs_use_audio_ids_and_count_unplayable_records_for_offsets() {
        let page = radio_programs(&json!({"code": 200, "more": true, "programs": [
            {"id": 10, "name": "Episode", "mainSong": {"id": 99, "duration": 123000},
             "coverUrl": "https://example.com/episode.jpg", "radio": {"name": "Radio"},
             "dj": {"userId": 33, "nickname": "Host"}},
            {"id": 11, "name": "Unavailable"}
        ]}))
        .unwrap();
        assert_eq!(page.received, 2);
        assert!(page.more);
        assert_eq!(page.tracks.len(), 1);
        let track = &page.tracks[0];
        assert_eq!(track.id, 99);
        assert_eq!(track.title, "Episode");
        assert_eq!(track.duration_ms, 123000);
        assert_eq!(track.cover_url(), "https://example.com/episode.jpg");
        assert_eq!(track.album.name, "Radio");
        assert_eq!(track.artists[0].name, "Host");
        assert_eq!(track.artists[0].id, 0);
        assert!(radio_programs(&json!({"code": 500})).is_err());
        assert!(radio_programs(&json!({"code": 200})).is_err());
    }

    #[test]
    fn radio_detail_uses_data_envelope_and_preserves_host() {
        let detail = radio_detail(&json!({"code": 200, "data": {
            "id": 12, "name": "Radio", "desc": "Description", "programCount": 201,
            "picUrl": "https://example.com/radio.jpg", "dj": {"userId": 5, "nickname": "Host"}
        }}))
        .unwrap();
        assert_eq!(detail.radio.id, 12);
        assert_eq!(detail.radio.program_count, 201);
        assert_eq!(detail.radio.creator.id, 5);
        assert_eq!(detail.description, "Description");
        assert!(radio_detail(&json!({"code": 200, "data": {}})).is_err());
    }

    #[test]
    fn maps_video_search_results() {
        let value = json!({
            "code": 200,
            "result": {
                "mvCount": "1",
                "mvs": [{
                    "id": 12,
                    "name": "Test MV",
                    "cover": "https://example.com/video.jpg",
                    "artistName": "Test Artist",
                    "duration": "1234",
                    "playCount": 99
                }]
            }
        });

        let response = search(&value, SearchType::Videos).expect("video search should map");

        assert_eq!(response.video_count, 1);
        assert_eq!(response.videos[0].id, 12);
        assert_eq!(
            response.videos[0].cover_url,
            "https://example.com/video.jpg"
        );
        assert_eq!(response.videos[0].duration_ms, 1234);
    }

    #[test]
    fn maps_radio_search_results() {
        let value = json!({
            "code": 200,
            "result": {
                "djRadiosCount": 2,
                "djRadios": [{
                    "id": 8,
                    "name": "Test Podcast",
                    "picUrl": "https://example.com/radio.jpg",
                    "category": "情感",
                    "programCount": "24",
                    "dj": {
                        "userId": 7,
                        "nickname": "Test Host",
                        "avatarUrl": "https://example.com/avatar.jpg"
                    }
                }]
            }
        });

        let response = search(&value, SearchType::Radios).expect("radio search should map");

        assert_eq!(response.radio_count, 2);
        assert_eq!(response.radios[0].id, 8);
        assert_eq!(response.radios[0].creator.nickname, "Test Host");
        assert_eq!(response.radios[0].program_count, 24);
    }
}

/// Desktop suggestions use the web contract, not Android's `allMatch` keywords.
pub fn search_suggestions(value: &Value) -> Result<Vec<SearchSuggestion>> {
    if !code_ok(value) {
        return Err(NcmError::business("search suggestions failed"));
    }
    let result = value.get("result").unwrap_or(&Value::Null);
    let mut suggestions = Vec::new();
    for (key, kind) in [
        ("songs", SearchType::Songs),
        ("artists", SearchType::Artists),
        ("albums", SearchType::Albums),
        ("playlists", SearchType::Playlists),
    ] {
        for item in result
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(5)
        {
            let Some(id) = item.get("id").and_then(as_u64).filter(|id| *id > 0) else {
                continue;
            };
            let title = str_value(item, "name");
            if title.trim().is_empty() {
                continue;
            }
            let subtitle = match kind {
                SearchType::Songs => {
                    artists_from_array(item.get("artists").and_then(Value::as_array))
                        .iter()
                        .map(|artist| artist.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                }
                SearchType::Albums => item
                    .get("artist")
                    .map(|artist| str_value(artist, "name"))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            suggestions.push(SearchSuggestion {
                id,
                kind,
                title,
                subtitle,
            });
        }
    }
    Ok(suggestions)
}

pub fn radio_detail(value: &Value) -> Result<RadioDetail> {
    if !code_ok(value) {
        return Err(NcmError::business("radio detail failed"));
    }
    let data = value
        .get("data")
        .ok_or_else(|| NcmError::protocol("radio detail missing"))?;
    let radio =
        radio_summary_from_value(data).ok_or_else(|| NcmError::protocol("radio id missing"))?;
    Ok(RadioDetail {
        radio,
        description: str_value(data, "desc"),
    })
}

pub fn radio_programs(value: &Value) -> Result<RadioPrograms> {
    if !code_ok(value) {
        return Err(NcmError::business("radio programs failed"));
    }
    let programs = value
        .get("programs")
        .and_then(Value::as_array)
        .ok_or_else(|| NcmError::protocol("radio programs missing"))?;
    let tracks = programs
        .iter()
        .filter_map(|program| {
            // Program IDs identify episodes; only mainSong.id identifies playable audio.
            let mut track = track_from_value(program.get("mainSong")?, None).ok()?;
            if track.id == 0 {
                return None;
            }
            let title = str_value(program, "name");
            if !title.is_empty() {
                track.title = title;
            }
            let cover = str_value(program, "coverUrl");
            if !cover.is_empty() {
                track.album.image_url = cover;
            }
            if let Some(radio) = program.get("radio") {
                track.album.name = str_value(radio, "name");
            }
            if let Some(dj) = program.get("dj") {
                // DJ user IDs must never be passed to artist-detail endpoints.
                track.artists = vec![ArtistSummary {
                    id: 0,
                    name: str_value(dj, "nickname"),
                    image_url: String::new(),
                }];
            }
            Some(track)
        })
        .collect();
    Ok(RadioPrograms {
        tracks,
        received: programs.len() as u32,
        more: value.get("more").and_then(Value::as_bool).unwrap_or(false),
    })
}
