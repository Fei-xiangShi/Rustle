//! Built-in multi-source online lyrics fetching and automatic selection.

use anyhow::{Result, anyhow};
use base64::Engine as _;
use futures_util::{StreamExt, stream::FuturesUnordered};
use lyrics_crypto::decrypter::qrc::decrypter::decrypt_lyrics as decrypt_qrc_lyrics;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{
    LyricLineOwned, LyricsFormat, merge_romanization, merge_translation, parse_lyrics,
    parse_lyrics_with_format,
};
use crate::api::NcmClient;

// v2 invalidates selections made before strict song matching and XML fixes.
const BEST_CACHE_VERSION: u32 = 3;
const HTTP_TIMEOUT: Duration = Duration::from_secs(9);
const QQ_API_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const AMLL_TTML_URLS: &[&str] = &[
    "https://amlldb.bikonoo.com/{platform}/{id}.ttml",
    "https://raw.githubusercontent.com/Steve-xmh/amll-ttml-db/refs/heads/main/{platform}/{id}.ttml",
    "https://amll.mirror.dimeta.top/api/db/{platform}/{id}.ttml",
];

/// Metadata used to match a NetEase track to QQ Music without exposing a
/// source selector to the user.
#[derive(Debug, Clone, Default)]
pub struct OnlineLyricsMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LyricsSource {
    AmllTtml,
    QqMusic,
    NcmOfficial,
}

impl LyricsSource {
    const fn label(self) -> &'static str {
        match self {
            Self::AmllTtml => "amll_ttml",
            Self::QqMusic => "qq_music",
            Self::NcmOfficial => "ncm_official",
        }
    }

    const fn tie_breaker(self) -> u32 {
        match self {
            // Source is a small tie preference after content quality. Song
            // identity has already been checked before constructing candidates.
            Self::AmllTtml => 30,
            Self::QqMusic => 20,
            Self::NcmOfficial => 10,
        }
    }
}

#[derive(Debug, Clone)]
struct LyricsCandidate {
    source: LyricsSource,
    lines: Vec<LyricLineOwned>,
    score: u32,
}

impl LyricsCandidate {
    fn new(source: LyricsSource, lines: Vec<LyricLineOwned>, duration_ms: u64) -> Option<Self> {
        let score = lyrics_quality_score(&lines, duration_ms, source)?;
        Some(Self {
            source,
            lines,
            score,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BestLyricsCache {
    version: u32,
    source: LyricsSource,
    score: u32,
    lines: Vec<LyricLineOwned>,
}

#[derive(Debug, Clone)]
struct QqSong {
    id: u64,
    mid: String,
    title: String,
    artist: String,
    album: String,
    duration_ms: u64,
}

fn get_cache_path(ncm_id: u64, suffix: &str) -> PathBuf {
    crate::utils::lyrics_cache_dir().join(format!("{ncm_id}{suffix}"))
}

fn best_cache_path(ncm_id: u64) -> PathBuf {
    get_cache_path(ncm_id, ".best.json")
}

/// Read and fully validate the selected cache. Call from a blocking worker.
pub fn load_cached_best_lyrics(ncm_id: u64) -> Option<Vec<LyricLineOwned>> {
    let bytes = std::fs::read(best_cache_path(ncm_id)).ok()?;
    decode_cached_best_lyrics(&bytes)
}

fn decode_cached_best_lyrics(bytes: &[u8]) -> Option<Vec<LyricLineOwned>> {
    let cache: BestLyricsCache = serde_json::from_slice(bytes).ok()?;
    (cache.version == BEST_CACHE_VERSION && !cache.lines.is_empty()).then_some(cache.lines)
}

fn load_cached_yrc_lyrics(ncm_id: u64) -> Option<Vec<LyricLineOwned>> {
    let content = std::fs::read_to_string(get_cache_path(ncm_id, ".yrc")).ok()?;
    let mut lines = parse_lyrics(&content);
    if lines.is_empty() {
        return None;
    }
    merge_cached_sidecar(ncm_id, ".tlrc", &mut lines, merge_translation);
    Some(lines)
}

fn load_cached_lrc_lyrics(ncm_id: u64) -> Option<Vec<LyricLineOwned>> {
    let content = std::fs::read_to_string(get_cache_path(ncm_id, ".lrc")).ok()?;
    let mut lines = parse_lyrics(&content);
    if lines.is_empty() {
        return None;
    }
    merge_cached_sidecar(ncm_id, ".tlrc", &mut lines, merge_translation);
    Some(lines)
}

fn merge_cached_sidecar(
    ncm_id: u64,
    suffix: &str,
    lines: &mut [LyricLineOwned],
    merge: fn(&mut [LyricLineOwned], &[LyricLineOwned]),
) {
    let Ok(content) = std::fs::read_to_string(get_cache_path(ncm_id, suffix)) else {
        return;
    };
    let sidecar = parse_lyrics(&content);
    if !sidecar.is_empty() {
        merge(lines, &sidecar);
    }
}

/// Cheap metadata-only probe for any cached lyrics file. Content is validated
/// later by [`load_cached_lyrics`] off the UI thread.
pub fn has_cached_lyrics(ncm_id: u64) -> bool {
    [".best.json", ".yrc", ".lrc"].iter().any(|suffix| {
        std::fs::metadata(get_cache_path(ncm_id, suffix))
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
    })
}

/// Load the selected multi-source result, with old Rustle caches retained as
/// an offline compatibility fallback.
pub fn load_cached_lyrics(ncm_id: u64) -> Option<Vec<LyricLineOwned>> {
    load_cached_best_lyrics(ncm_id)
        .or_else(|| load_cached_yrc_lyrics(ncm_id))
        .or_else(|| load_cached_lrc_lyrics(ncm_id))
}

fn write_cache_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = crate::cache::unique_temp_path(path);
    if let Err(error) = std::fs::write(&temp, bytes) {
        crate::cache::cleanup_temp_file(&temp);
        return Err(error.into());
    }
    crate::cache::publish_replace(&temp, path).map_err(Into::into)
}

fn save_best_lyrics_cache(candidate: &LyricsCandidate, ncm_id: u64) -> Result<()> {
    std::fs::create_dir_all(crate::utils::lyrics_cache_dir())?;
    let cache = BestLyricsCache {
        version: BEST_CACHE_VERSION,
        source: candidate.source,
        score: candidate.score,
        lines: candidate.lines.clone(),
    };
    write_cache_file(&best_cache_path(ncm_id), &serde_json::to_vec(&cache)?)
}

fn visible_text(line: &LyricLineOwned) -> String {
    line.words.iter().map(|word| word.word.as_str()).collect()
}

fn lyrics_quality_score(
    lines: &[LyricLineOwned],
    duration_ms: u64,
    source: LyricsSource,
) -> Option<u32> {
    let visible = lines
        .iter()
        .filter(|line| !visible_text(line).trim().is_empty())
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return None;
    }

    let count = visible.len() as u64;
    let dynamic = visible
        .iter()
        .filter(|line| {
            let mut timed = line
                .words
                .iter()
                .filter(|word| !word.word.trim().is_empty() && word.end_time > word.start_time);
            timed
                .next()
                .is_some_and(|first| timed.any(|word| word.start_time != first.start_time))
        })
        .count() as u64;
    let translated = visible
        .iter()
        .filter(|line| !line.translated_lyric.trim().is_empty())
        .count() as u64;
    let romanized = visible
        .iter()
        .filter(|line| !line.roman_lyric.trim().is_empty())
        .count() as u64;
    let special = visible
        .iter()
        .filter(|line| line.is_bg || line.is_duet)
        .count() as u64;
    let last_end = visible
        .iter()
        .flat_map(|line| {
            line.words
                .iter()
                .map(|word| word.end_time)
                .chain([line.end_time])
        })
        .max()
        .unwrap_or_default();

    let mut score = (count.min(250) * 4) as i64;
    score += ((dynamic * 6_000) / count) as i64;
    score += ((translated * 1_200) / count) as i64;
    score += ((romanized * 600) / count) as i64;
    score += (special.min(20) * 20) as i64;
    score += source.tie_breaker() as i64;

    if duration_ms > 0 && last_end > 0 && last_end != u64::MAX {
        let coverage_per_mille = last_end.saturating_mul(1_000) / duration_ms;
        score += match coverage_per_mille {
            700..=1_150 => 700,
            500..=1_300 => 300,
            350..=1_500 => 0,
            _ => -700,
        };
    }

    Some(score.max(0) as u32)
}

fn attach_sidecars(
    lines: &mut [LyricLineOwned],
    translation: Option<&str>,
    romanization: Option<&str>,
) {
    if let Some(content) = translation.filter(|content| !content.trim().is_empty()) {
        let sidecar = parse_sidecar_lines(content);
        merge_translation(lines, &sidecar);
    }
    if let Some(content) = romanization.filter(|content| !content.trim().is_empty()) {
        let sidecar = parse_sidecar_lines(content);
        merge_romanization(lines, &sidecar);
    }
}

fn parse_sidecar_lines(content: &str) -> Vec<LyricLineOwned> {
    let parsed = parse_lyrics(content);
    if !parsed.is_empty() {
        return parsed;
    }

    // NCM's YTLRC sidecar can use `[start,duration]text` without per-word
    // markers. It is neither regular LRC nor YRC, but still has a stable line
    // anchor that can be merged with the main word-level lyrics.
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with('[') {
                return None;
            }
            let close = line.find(']')?;
            let mut timing = line.get(1..close)?.split(',');
            let start_time = timing.next()?.parse::<u64>().ok()?;
            let duration = timing.next()?.parse::<u64>().ok()?;
            if timing.next().is_some() {
                return None;
            }
            let text = line.get(close + 1..)?.trim();
            if text.is_empty() {
                return None;
            }
            let end_time = start_time.saturating_add(duration);
            Some(LyricLineOwned {
                words: vec![super::LyricWordOwned {
                    start_time,
                    end_time,
                    word: text.to_string(),
                    roman_word: String::new(),
                }],
                start_time,
                end_time,
                ..Default::default()
            })
        })
        .collect()
}

fn joined(lines: &[String]) -> Option<String> {
    (!lines.is_empty()).then(|| lines.join("\n"))
}

async fn fetch_ncm_candidates(
    client: &NcmClient,
    ncm_id: u64,
    duration_ms: u64,
) -> Result<Vec<LyricsCandidate>> {
    let lyrics = client.song_lyric(ncm_id).await?;
    let translation = joined(&lyrics.ytlrc).or_else(|| joined(&lyrics.tlyric));
    let romanization = joined(&lyrics.yromalrc).or_else(|| joined(&lyrics.romalrc));
    let mut candidates = Vec::new();

    if let Some(content) = joined(&lyrics.yrc) {
        let mut lines = parse_lyrics_with_format(&content, LyricsFormat::Yrc);
        attach_sidecars(&mut lines, translation.as_deref(), romanization.as_deref());
        if let Some(candidate) = LyricsCandidate::new(LyricsSource::NcmOfficial, lines, duration_ms)
        {
            candidates.push(candidate);
        }
    }

    if let Some(content) = joined(&lyrics.lyric) {
        let mut lines = parse_lyrics_with_format(&content, LyricsFormat::Lrc);
        let lrc_translation = joined(&lyrics.tlyric);
        let lrc_romanization = joined(&lyrics.romalrc);
        attach_sidecars(
            &mut lines,
            lrc_translation.as_deref(),
            lrc_romanization.as_deref(),
        );
        if let Some(candidate) = LyricsCandidate::new(LyricsSource::NcmOfficial, lines, duration_ms)
        {
            candidates.push(candidate);
        }
    }

    Ok(candidates)
}

fn build_http_client(proxy_url: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent("Rustle/0.5 multi-source-lyrics");
    if let Some(proxy_url) = proxy_url
        && let Ok(proxy) = reqwest::Proxy::all(proxy_url)
    {
        builder = builder.proxy(proxy);
    }
    Ok(builder.build()?)
}

async fn fetch_amll_candidate(
    http: &reqwest::Client,
    platform: &str,
    id: &str,
    duration_ms: u64,
) -> Option<LyricsCandidate> {
    let responses = FuturesUnordered::new();
    for template in AMLL_TTML_URLS {
        let http = http.clone();
        let mut url = reqwest::Url::parse(&template.replace("{platform}", platform)).ok()?;
        url.path_segments_mut()
            .ok()?
            .pop()
            .push(&format!("{id}.ttml"));
        responses.push(async move {
            let response = http.get(url).send().await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            response.text().await.ok()
        });
    }

    first_amll_candidate(responses, duration_ms).await
}

async fn first_amll_candidate(
    mut responses: impl futures_util::Stream<Item = Option<String>> + Unpin,
    duration_ms: u64,
) -> Option<LyricsCandidate> {
    while let Some(response) = responses.next().await {
        let Some(content) = response else {
            continue;
        };
        let lines = parse_lyrics_with_format(&content, LyricsFormat::Ttml);
        if let Some(candidate) = LyricsCandidate::new(LyricsSource::AmllTtml, lines, duration_ms) {
            return Some(candidate);
        }
    }
    None
}

fn qq_common_params() -> Value {
    json!({
        "ct": 11,
        "cv": "1003006",
        "v": "1003006",
        "os_ver": "15",
        "phonetype": "24122RKC7C",
        "tmeAppID": "qqmusiclight",
        "nettype": "NETWORK_WIFI",
        "udid": "0"
    })
}

async fn qq_request(
    http: &reqwest::Client,
    method: &str,
    module: &str,
    param: Value,
) -> Result<Value> {
    let response = http
        .post(QQ_API_URL)
        .header("Content-Type", "application/json")
        .header("User-Agent", "okhttp/3.14.9")
        .header("Cookie", "tmeLoginType=-1;")
        .json(&json!({
            "comm": qq_common_params(),
            "request": { "method": method, "module": module, "param": param }
        }))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(anyhow!("QQ Music returned HTTP {}", response.status()));
    }
    let payload: Value = response.json().await?;
    if value_i64(payload.get("code")) != Some(0)
        || value_i64(payload.pointer("/request/code")) != Some(0)
    {
        return Err(anyhow!("QQ Music rejected the request"));
    }
    Ok(payload
        .pointer("/request/data")
        .cloned()
        .unwrap_or(Value::Null))
}

fn value_i64(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|number| number.parse().ok()))
}

fn value_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|number| number.parse().ok()))
}

fn metadata_match_score(song: &QqSong, metadata: &OnlineLyricsMetadata) -> Option<u32> {
    use rustle_media::lyrics::matching::{SongIdentity, match_score};
    match_score(
        SongIdentity {
            title: &metadata.title,
            artists: &metadata.artist,
            album: &metadata.album,
            duration_ms: metadata.duration_ms,
        },
        SongIdentity {
            title: &song.title,
            artists: &song.artist,
            album: &song.album,
            duration_ms: song.duration_ms,
        },
    )
}

async fn search_qq_song(
    http: &reqwest::Client,
    metadata: &OnlineLyricsMetadata,
) -> Result<Option<QqSong>> {
    if metadata.title.trim().is_empty() {
        return Ok(None);
    }
    let keyword = format!("{} {}", metadata.title, metadata.artist);
    let search_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();
    let data = qq_request(
        http,
        "DoSearchForQQMusicLite",
        "music.search.SearchCgiService",
        json!({
            "search_id": search_id,
            "remoteplace": "search.android.keyboard",
            "query": keyword,
            "search_type": 0,
            "num_per_page": 25,
            "page_num": 1,
            "highlight": 0,
            "nqc_flag": 0,
            "page_id": 1,
            "grp": 1
        }),
    )
    .await?;

    let songs = data
        .pointer("/body/item_song")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let singers = value
                .get("singer")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|singer| singer.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" / ");
            Some(QqSong {
                id: value_u64(value.get("id"))?,
                mid: value
                    .get("mid")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                title: value.get("title")?.as_str()?.to_string(),
                artist: singers,
                album: value
                    .pointer("/album/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                duration_ms: value_u64(value.get("interval"))
                    .unwrap_or_default()
                    .saturating_mul(1_000),
            })
        });

    Ok(songs
        .enumerate()
        .filter_map(|(index, song)| {
            metadata_match_score(&song, metadata).map(|score| (score, index, song))
        })
        .max_by_key(|(score, index, _)| (*score, std::cmp::Reverse(*index)))
        .map(|(_, _, song)| song))
}

fn decrypt_qrc(encrypted: &str) -> Result<String> {
    decrypt_qrc_lyrics(encrypted).ok_or_else(|| anyhow!("could not decrypt QRC lyrics"))
}

fn decrypted_field(data: &Value, key: &str) -> Option<String> {
    let encrypted = data.get(key)?.as_str()?.trim();
    (!encrypted.is_empty())
        .then(|| decrypt_qrc(encrypted).ok())
        .flatten()
}

async fn qq_lyric_data(http: &reqwest::Client, song: &QqSong, qrc: bool) -> Result<Value> {
    let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value);
    qq_request(
        http,
        "GetPlayLyricInfo",
        "music.musichallSong.PlayLyricInfo",
        json!({
            "albumName": encode(&song.album),
            "crypt": 1,
            "ct": 19,
            "cv": 2111,
            "interval": song.duration_ms / 1_000,
            "lrc_t": 0,
            "qrc": u8::from(qrc),
            "qrc_t": 0,
            "roma": 1,
            "roma_t": 0,
            "singerName": encode(&song.artist),
            "songID": song.id,
            "songName": encode(&song.title),
            "trans": 1,
            "trans_t": 0,
            "type": 0
        }),
    )
    .await
}

async fn fetch_qq_candidates(
    http: &reqwest::Client,
    metadata: &OnlineLyricsMetadata,
) -> Result<Vec<LyricsCandidate>> {
    let Some(song) = search_qq_song(http, metadata).await? else {
        return Ok(Vec::new());
    };
    // A matched QQ identity also unlocks QQ-only AMLL entries. Its TTML request
    // remains independent from the official lyric endpoint's availability.
    let (official, ttml) = tokio::join!(
        fetch_qq_official_candidates(http, &song, metadata.duration_ms),
        fetch_qq_amll_candidate(http, &song, metadata.duration_ms),
    );
    combine_qq_candidates(official, ttml)
}

fn combine_qq_candidates(
    official: Result<Vec<LyricsCandidate>>,
    ttml: Option<LyricsCandidate>,
) -> Result<Vec<LyricsCandidate>> {
    match (official, ttml) {
        (Ok(mut candidates), ttml) => {
            candidates.extend(ttml);
            Ok(candidates)
        }
        (Err(_), Some(ttml)) => Ok(vec![ttml]),
        (Err(error), None) => Err(error),
    }
}

async fn fetch_qq_amll_candidate(
    http: &reqwest::Client,
    song: &QqSong,
    duration_ms: u64,
) -> Option<LyricsCandidate> {
    let id = song.id.to_string();
    let mut requests = FuturesUnordered::new();
    requests.push(fetch_amll_candidate(http, "qq-lyrics", &id, duration_ms));
    if !song.mid.is_empty() && song.mid != id {
        requests.push(fetch_amll_candidate(
            http,
            "qq-lyrics",
            &song.mid,
            duration_ms,
        ));
    }
    while let Some(candidate) = requests.next().await {
        if candidate.is_some() {
            return candidate;
        }
    }
    None
}

async fn fetch_qq_official_candidates(
    http: &reqwest::Client,
    song: &QqSong,
    duration_ms: u64,
) -> Result<Vec<LyricsCandidate>> {
    let data = qq_lyric_data(http, song, true).await?;
    let translation = decrypted_field(&data, "trans");
    let romanization = decrypted_field(&data, "roma");
    let main = decrypted_field(&data, "lyric");
    let mut candidates = Vec::new();

    if let Some(content) = main.as_deref() {
        let mut lines = parse_lyrics_with_format(content, LyricsFormat::Qrc);
        attach_sidecars(&mut lines, translation.as_deref(), romanization.as_deref());
        if let Some(candidate) = LyricsCandidate::new(LyricsSource::QqMusic, lines, duration_ms) {
            candidates.push(candidate);
        }
    }

    let lrc_content = if value_i64(data.get("qrc_t")) == Some(0) {
        main
    } else {
        match qq_lyric_data(http, song, false).await {
            Ok(data) => decrypted_field(&data, "lyric"),
            Err(error) if candidates.is_empty() => return Err(error),
            Err(error) => {
                tracing::debug!(error = %error, "optional_qq_lrc_unavailable");
                None
            }
        }
    };
    if let Some(content) = lrc_content {
        let mut lines = parse_lyrics_with_format(&content, LyricsFormat::Lrc);
        attach_sidecars(&mut lines, translation.as_deref(), romanization.as_deref());
        if let Some(candidate) = LyricsCandidate::new(LyricsSource::QqMusic, lines, duration_ms) {
            candidates.push(candidate);
        }
    }

    Ok(candidates)
}

/// Fetch all built-in sources and cache the highest-quality parsed result.
pub async fn fetch_lyrics(
    client: &NcmClient,
    ncm_id: u64,
    metadata: OnlineLyricsMetadata,
    proxy_url: Option<String>,
) -> Result<Vec<LyricLineOwned>> {
    if let Some(cached) =
        tokio::task::spawn_blocking(move || load_cached_best_lyrics(ncm_id)).await?
    {
        tracing::debug!(ncm_id, "loaded multi-source lyrics cache");
        return Ok(cached);
    }

    let http = build_http_client(proxy_url.as_deref())?;
    let ncm_key = ncm_id.to_string();
    let (ncm_result, amll_candidate, qq_result) = tokio::join!(
        fetch_ncm_candidates(client, ncm_id, metadata.duration_ms),
        fetch_amll_candidate(&http, "ncm-lyrics", &ncm_key, metadata.duration_ms),
        fetch_qq_candidates(&http, &metadata),
    );

    let mut candidates = match &ncm_result {
        Ok(candidates) => candidates.clone(),
        Err(_) => Vec::new(),
    };
    if let Some(candidate) = amll_candidate {
        candidates.push(candidate);
    }
    match qq_result {
        Ok(mut qq_candidates) => candidates.append(&mut qq_candidates),
        Err(error) => tracing::debug!(ncm_id, error = %error, "qq_music_lyrics_unavailable"),
    }

    let Some(best) = candidates
        .into_iter()
        .max_by_key(|candidate| candidate.score)
    else {
        if let Some(cached) =
            tokio::task::spawn_blocking(move || load_cached_lyrics(ncm_id)).await?
        {
            tracing::debug!(ncm_id, "falling back to legacy lyrics cache");
            return Ok(cached);
        }
        return match ncm_result {
            Err(error) => Err(error),
            Ok(_) => Err(anyhow!("No lyrics found from any built-in source")),
        };
    };

    tracing::info!(
        ncm_id,
        source = best.source.label(),
        score = best.score,
        lines = best.lines.len(),
        "selected_best_lyrics_source"
    );
    let (best, saved) = tokio::task::spawn_blocking(move || {
        let saved = save_best_lyrics_cache(&best, ncm_id);
        (best, saved)
    })
    .await?;
    if let Err(error) = saved {
        tracing::warn!(ncm_id, error = %error, "lyrics_cache_write_failed");
    }
    Ok(best.lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mirror_failure_and_invalid_xml_do_not_hide_later_success() {
        let responses = futures_util::stream::iter([
            None,
            Some("<html>not lyrics</html>".into()),
            Some(
                "<tt><head/><body><div><p begin=\"1s\" end=\"2s\">Good</p></div></body></tt>"
                    .into(),
            ),
        ]);
        let candidate = first_amll_candidate(responses, 2_000).await.unwrap();
        assert_eq!(candidate.lines[0].words[0].word, "Good");
    }

    #[test]
    fn old_parsed_cache_is_invalidated() {
        let cache = BestLyricsCache {
            version: 1,
            source: LyricsSource::QqMusic,
            score: 10,
            lines: vec![line(2, false, false)],
        };
        assert!(decode_cached_best_lyrics(&serde_json::to_vec(&cache).unwrap()).is_none());
        let cache = BestLyricsCache {
            version: BEST_CACHE_VERSION,
            ..cache
        };
        assert!(decode_cached_best_lyrics(&serde_json::to_vec(&cache).unwrap()).is_some());
    }

    #[test]
    fn cache_header_alone_cannot_validate_corrupt_or_empty_lyrics() {
        for lines in [
            serde_json::json!([null]),
            serde_json::json!([{}]),
            serde_json::json!([]),
        ] {
            let cache = serde_json::json!({
                "version": BEST_CACHE_VERSION,
                "source": "qq_music",
                "score": 10,
                "lines": lines,
            });
            assert!(decode_cached_best_lyrics(&serde_json::to_vec(&cache).unwrap()).is_none());
        }
    }

    #[test]
    fn qq_ttml_survives_official_source_failure() {
        let candidate =
            LyricsCandidate::new(LyricsSource::AmllTtml, vec![line(2, false, false)], 1000)
                .unwrap();
        let candidates =
            combine_qq_candidates(Err(anyhow!("official unavailable")), Some(candidate)).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source, LyricsSource::AmllTtml);
    }

    #[test]
    fn identical_word_times_are_not_counted_as_word_level_sync() {
        let plain = line(1, false, false);
        let fake = LyricLineOwned {
            words: vec![plain.words[0].clone(); 4],
            ..plain.clone()
        };
        assert_eq!(
            lyrics_quality_score(&[plain], 1000, LyricsSource::QqMusic),
            lyrics_quality_score(&[fake], 1000, LyricsSource::QqMusic)
        );
    }

    fn line(words: usize, translated: bool, romanized: bool) -> LyricLineOwned {
        let words = (0..words)
            .map(|index| super::super::LyricWordOwned {
                start_time: index as u64 * 500,
                end_time: index as u64 * 500 + 500,
                word: format!("word{index}"),
                roman_word: String::new(),
            })
            .collect();
        LyricLineOwned {
            words,
            translated_lyric: if translated { "translation" } else { "" }.into(),
            roman_lyric: if romanized { "roman" } else { "" }.into(),
            end_time: 1_000,
            ..Default::default()
        }
    }

    #[test]
    fn richer_word_level_candidate_beats_plain_line_lyrics() {
        let plain = vec![line(1, false, false); 20];
        let rich = vec![line(4, true, true); 20];
        let plain_score = lyrics_quality_score(&plain, 20_000, LyricsSource::NcmOfficial).unwrap();
        let rich_score = lyrics_quality_score(&rich, 20_000, LyricsSource::QqMusic).unwrap();
        assert!(rich_score > plain_score);
    }

    #[test]
    fn qq_matching_rejects_wrong_duration_and_prefers_exact_metadata() {
        let metadata = OnlineLyricsMetadata {
            title: "Test Song".into(),
            artist: "Test Artist".into(),
            duration_ms: 180_000,
            ..Default::default()
        };
        let exact = QqSong {
            id: 1,
            mid: String::new(),
            title: "Test Song".into(),
            artist: "Test Artist".into(),
            album: String::new(),
            duration_ms: 179_500,
        };
        let wrong = QqSong {
            duration_ms: 200_001,
            ..exact.clone()
        };
        assert!(metadata_match_score(&exact, &metadata).is_some());
        assert!(metadata_match_score(&wrong, &metadata).is_none());
    }

    #[test]
    fn parses_ncm_millisecond_translation_sidecar() {
        let lines = parse_sidecar_lines("[0,1000]翻译一\n[1000,800]翻译二");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_time, 0);
        assert_eq!(lines[0].words[0].word, "翻译一");
        assert_eq!(lines[1].end_time, 1_800);
    }

    #[test]
    fn decrypts_real_qq_music_ciphertext() {
        let encrypted = "6523D74F97F58193307F22C086407DEFA77AD0701318A77B0EBB14AA766102FF88EAD2D3516B2E3E469C3AC06108BB991BBA2DE81BBC31D9CFBE9E5AC8178D67ED45356B838CE5A0";
        let decrypted = decrypt_qrc(encrypted).unwrap();

        assert_eq!(decrypted, "[00:00:00]此歌曲为没有填词的纯音乐，请您欣赏");
    }
}
