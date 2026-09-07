use futures_util::stream::{self, StreamExt};
use ncm_api_rs::{ApiClient, CryptoType, Query, RequestOption, create_client};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{fs, io, path::PathBuf};
use tracing::error;

use super::mapper::{self, AlbumSource, PlaylistSource};
use super::models::*;
use super::{NcmError, NcmResult as Result};

const COOKIE_FILE: &str = "cookies.json";
const DEFAULT_QUALITY: u32 = 2;
const PLAYLIST_DETAIL_CHUNK_SIZE: usize = 500;

fn image_download_url(url: String, resize: Option<(u16, u16)>) -> String {
    resize.map_or_else(
        || url.clone(),
        |(width, height)| {
            let separator = if url.contains('?') { '&' } else { '?' };
            format!("{url}{separator}param={width}y{height}")
        },
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedSession {
    cookie: String,
}

#[derive(Clone)]
pub struct NcmClient {
    client: ApiClient,
    cookie: Arc<parking_lot::RwLock<String>>,
    proxy: Option<String>,
    quality: Arc<AtomicU32>,
}

impl std::fmt::Debug for NcmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NcmClient")
            .field("client", &"<ncm_api_rs::ApiClient>")
            .field("has_cookie", &!self.cookie.read().is_empty())
            .field("proxy", &self.proxy)
            .finish()
    }
}

impl Default for NcmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NcmClient {
    pub fn new() -> Self {
        Self::from_cookie_and_proxy(None, None)
    }

    pub fn with_proxy(proxy_url: Option<String>) -> Self {
        Self::from_cookie_and_proxy(None, proxy_url)
    }

    pub fn from_cookie(cookie: String) -> Self {
        Self::from_cookie_and_proxy(Some(cookie), None)
    }

    pub fn from_cookie_with_proxy(cookie: String, proxy_url: Option<String>) -> Self {
        Self::from_cookie_and_proxy(Some(cookie), proxy_url)
    }

    fn from_cookie_and_proxy(cookie: Option<String>, proxy: Option<String>) -> Self {
        let client = create_client(cookie.clone());
        Self {
            client,
            cookie: Arc::new(parking_lot::RwLock::new(cookie.unwrap_or_default())),
            proxy,
            quality: Arc::new(AtomicU32::new(DEFAULT_QUALITY)),
        }
    }

    fn data_dir() -> PathBuf {
        directories::ProjectDirs::from("life", "fxs", "rustle")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn cache_dir() -> PathBuf {
        directories::ProjectDirs::from("life", "fxs", "rustle")
            .map(|dirs| dirs.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("~/.cache/rustle"))
    }

    pub fn cookie_file_path() -> PathBuf {
        let data_dir = Self::data_dir();
        fs::create_dir_all(&data_dir).ok();
        data_dir.join(COOKIE_FILE)
    }

    pub fn load_cookie_from_file() -> Option<String> {
        match fs::File::open(Self::cookie_file_path()) {
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => None,
                other => {
                    error!("{:?}", other);
                    None
                }
            },
            Ok(file) => match serde_json::from_reader::<_, PersistedSession>(file) {
                Ok(session) if !session.cookie.trim().is_empty() => Some(session.cookie),
                Ok(_) => None,
                Err(err) => {
                    error!("{:?}", err);
                    None
                }
            },
        }
    }

    pub fn save_cookie_to_file(&self) {
        let cookie = self.cookie.read().clone();
        if cookie.trim().is_empty() {
            return;
        }

        match fs::File::create(Self::cookie_file_path()) {
            Ok(file) => {
                if let Err(err) = serde_json::to_writer(file, &PersistedSession { cookie }) {
                    error!("Failed to save cookies: {:?}", err);
                }
            }
            Err(err) => error!("{:?}", err),
        }
    }

    pub fn clean_cookie_file() {
        if let Err(err) = fs::remove_file(Self::cookie_file_path()) {
            match err.kind() {
                io::ErrorKind::NotFound => (),
                other => error!("{:?}", other),
            }
        }
    }

    pub fn set_proxy(&mut self, proxy: String) -> Result<()> {
        self.proxy = Some(proxy);
        Ok(())
    }

    pub fn set_quality(&self, quality: u32) {
        self.quality.store(quality, Ordering::Relaxed);
        tracing::info!(
            "Music quality set to: {} ({})",
            quality,
            Self::quality_to_level(quality)
        );
    }

    pub fn quality(&self) -> u32 {
        self.quality.load(Ordering::Relaxed)
    }

    fn quality_to_level(quality: u32) -> &'static str {
        match quality {
            0 => "standard",
            1 => "higher",
            2 => "exhigh",
            3 => "lossless",
            4 => "hires",
            5 => "jyeffect",
            6 => "sky",
            7 => "dolby",
            8 => "jymaster",
            _ => "invalid",
        }
    }

    pub fn current_quality_level(&self) -> NcmQualityLevel {
        NcmQualityLevel::from_api_rate(self.quality())
            .expect("NCM quality setting must be one of the canonical API rates")
    }

    fn query(&self) -> Query {
        let mut query = Query::new();
        let cookie = self.cookie.read().clone();
        if !cookie.trim().is_empty() {
            query = query.cookie(&cookie);
        }
        if let Some(proxy) = &self.proxy {
            query.proxy = Some(proxy.clone());
        }
        query
    }

    /// Build request options for the small number of endpoints where the
    /// bundled SDK method does not expose all request parameters.  Keeping
    /// this here still routes the request through ncm-api-rs' encryption,
    /// cookie, proxy, and device handling instead of introducing a second
    /// HTTP client.
    fn request_options(query: &Query) -> RequestOption {
        RequestOption {
            crypto: CryptoType::default(),
            cookie: query.cookie.clone(),
            ua: query.ua.clone(),
            proxy: query.proxy.clone(),
            real_ip: query.real_ip.clone(),
            random_cn_ip: query.random_cn_ip,
            e_r: query.e_r,
            domain: query.domain.clone(),
            check_token: false,
        }
    }

    fn remember_cookies(&self, cookies: Vec<String>) {
        if cookies.is_empty() {
            return;
        }

        let mut stored = self.cookie.write();
        let mut pairs: Vec<(String, String)> = stored.split(';').filter_map(cookie_pair).collect();

        for raw in cookies {
            if let Some((name, value)) = cookie_pair(&raw) {
                if let Some((_, existing)) = pairs.iter_mut().find(|(key, _)| key == &name) {
                    *existing = value;
                } else {
                    pairs.push((name, value));
                }
            }
        }

        *stored = pairs
            .into_iter()
            .map(|(name, value)| format!("{}={}", name, value))
            .collect::<Vec<_>>()
            .join("; ");
    }

    pub async fn create_qrcode(&self) -> Result<(PathBuf, String)> {
        let response = self.client.login_qr_key(&self.query()).await?;
        let unikey = response
            .body
            .get("data")
            .and_then(|data| data.get("unikey"))
            .or_else(|| response.body.get("unikey"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| NcmError::protocol("QR key missing"))?
            .to_string();
        let qr_url = format!("https://music.163.com/login?codekey={}", unikey);

        let cache_dir = Self::cache_dir();
        fs::create_dir_all(&cache_dir)?;

        if let Ok(entries) = fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                if name.starts_with("qrimage_") && name.ends_with(".png") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = cache_dir.join(format!("qrimage_{}.png", timestamp));
        let symbol = qrcode_generator::qr::Encoder::new(qrcode_generator::qr::ErrorCorrection::Low)
            .encode_text(&qr_url)
            .map_err(|error| NcmError::protocol_source("encode QR login", error))?;
        qrcode_generator::Renderer::new(&symbol, 200)
            .save_png(&path)
            .map_err(|error| NcmError::protocol_source("render QR login", error))?;
        Ok((path, unikey))
    }

    pub async fn login_qr_check(&self, key: String) -> Result<Msg> {
        let query = self.query().param("key", &key);
        let response = self.client.login_qr_check(&query).await?;
        self.remember_cookies(response.cookie);
        Ok(mapper::msg(&response.body))
    }

    pub async fn login_status(&self) -> Result<LoginInfo> {
        let response = self.client.login_status(&self.query()).await?;
        self.remember_cookies(response.cookie);
        let mut login = mapper::login_info(&response.body)?;
        // `/login/status` is intentionally sparse on newer accounts. Merge the
        // richer account payload when it is available, but never make login
        // fail only because membership metadata could not be refreshed.
        if login.code == 200
            && let Ok(account) = self.account_info().await
        {
            if login.user_id == 0 {
                login.user_id = account.user_id;
            }
            if login.nickname.is_empty() {
                login.nickname = account.nickname;
            }
            if login.avatar_url.is_empty() {
                login.avatar_url = account.avatar_url;
            }
            login.vip_type = account.vip_type;
            login.vip = account.vip;
        }
        if login.code == 200 && login.user_id > 0 {
            match self.membership_info(login.user_id, &login.vip).await {
                Ok(vip) => login.vip = vip,
                Err(error) => {
                    tracing::warn!(
                        user_id = login.user_id,
                        "Authoritative NCM membership image request failed: {}; hiding badge",
                        error
                    );
                    login.vip = login.vip.without_badges();
                }
            }
        }
        Ok(login)
    }

    pub async fn account_info(&self) -> Result<LoginInfo> {
        let response = self.client.user_account(&self.query()).await?;
        self.remember_cookies(response.cookie);
        mapper::account_info(&response.body)
    }

    /// Fetch the official membership image projection. Account/status
    /// endpoints do not reliably include badge URLs on current NCM accounts.
    pub async fn membership_info(&self, user_id: u64, base: &VipInfo) -> Result<VipInfo> {
        let query = self.query().param("uid", &user_id.to_string());
        let response = self.client.vip_info(&query).await?;
        self.remember_cookies(response.cookie);
        mapper::merge_membership_vip(base, &response.body)
    }

    pub async fn logout(&self) {
        let _ = self.client.logout(&self.query()).await;
        *self.cookie.write() = String::new();
    }

    pub async fn user_song_id_list(&self, user_id: u64) -> Result<Vec<u64>> {
        let query = self.query().param("uid", &user_id.to_string());
        let response = self.client.likelist(&query).await?;
        mapper::liked_song_ids(&response.body)
    }

    pub async fn user_playlists(
        &self,
        user_id: u64,
        offset: u16,
        limit: u16,
    ) -> Result<Vec<PlaylistSummary>> {
        let query = self
            .query()
            .param("uid", &user_id.to_string())
            .param("offset", &offset.to_string())
            .param("limit", &limit.to_string());
        let response = self.client.user_playlist(&query).await?;
        mapper::playlist_summaries(&response.body, PlaylistSource::User)
    }

    pub async fn playlist_detail(&self, playlist_id: u64) -> Result<PlaylistDetail> {
        let query = self.query().param("id", &playlist_id.to_string());
        let response = self.client.playlist_detail(&query).await?;
        let body = response.body;
        let mut detail = mapper::playlist_detail(&body)?;

        if detail.track_count > detail.tracks.len() as u64 {
            let track_ids = mapper::playlist_track_ids(&body);
            if track_ids.is_empty() {
                return Err(NcmError::protocol("playlist track ids missing"));
            }

            let fetch_limit = detail.track_count.min(track_ids.len() as u64) as usize;
            let chunks = track_ids[..fetch_limit]
                .chunks(PLAYLIST_DETAIL_CHUNK_SIZE)
                .map(|chunk| chunk.to_vec())
                .collect::<Vec<_>>();
            // Keep a small amount of parallelism so large playlists do not
            // make the first page wait for a long serial chain of requests.
            // `buffered` preserves input order while limiting in-flight calls.
            let results = stream::iter(chunks.into_iter().map(|chunk| {
                let client = self.clone();
                async move { client.track_detail(&chunk).await }
            }))
            .buffered(3)
            .collect::<Vec<_>>()
            .await;
            let mut tracks = Vec::with_capacity(fetch_limit);
            for result in results {
                tracks.extend(result?);
            }

            if tracks.len() < fetch_limit {
                return Err(NcmError::protocol(format!(
                    "playlist tracks incomplete: expected {fetch_limit}, got {}",
                    tracks.len()
                )));
            }

            detail.tracks = tracks;
        }

        Ok(detail)
    }

    /// Fetch only playlist metadata and track IDs.
    ///
    /// This mirrors SPlayer's first `/playlist/detail` request: callers can
    /// render the playlist header immediately and hydrate track details in a
    /// separate, progressive phase.
    pub async fn playlist_detail_preview(
        &self,
        playlist_id: u64,
    ) -> Result<(PlaylistDetail, Vec<u64>)> {
        let query = self.query().param("id", &playlist_id.to_string());
        // ncm-api-rs' playlist_detail helper currently hard-codes `n=100000`.
        // Use its public raw request method with n=0 so the metadata response
        // contains trackIds without eagerly materializing hundreds of songs.
        let response = self
            .client
            .request(
                "/api/v6/playlist/detail",
                json!({
                    "id": playlist_id.to_string(),
                    "n": 0,
                    "s": 8,
                }),
                Self::request_options(&query),
            )
            .await?;
        self.remember_cookies(response.cookie);
        let body = response.body;
        let detail = mapper::playlist_detail(&body)?;
        let mut track_ids = mapper::playlist_track_ids(&body);
        if track_ids.is_empty() {
            track_ids = detail.tracks.iter().map(|track| track.id).collect();
        }
        if detail.track_count > 0 && track_ids.is_empty() {
            return Err(NcmError::protocol("playlist track ids missing"));
        }
        Ok((detail, track_ids))
    }

    pub async fn artist_detail(&self, artist_id: u64) -> Result<ArtistDetail> {
        let query = self.query().param("id", &artist_id.to_string());
        let response = self.client.artists(&query).await?;
        mapper::artist_detail(&response.body)
    }

    pub async fn album_detail(&self, album_id: u64) -> Result<AlbumDetail> {
        let query = self.query().param("id", &album_id.to_string());
        let response = self.client.album(&query).await?;
        mapper::album_detail(&response.body)
    }

    pub async fn user_detail(&self, user_id: u64) -> Result<UserDetail> {
        let query = self.query().param("uid", &user_id.to_string());
        let response = self.client.user_detail(&query).await?;
        mapper::user_detail(&response.body)
    }

    async fn track_urls_for_level(
        &self,
        ids: &[u64],
        level: NcmQualityLevel,
    ) -> Result<Vec<TrackUrl>> {
        let id_list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let query = self
            .query()
            .param("id", &id_list)
            .param("level", super::models::quality_api_level(level));
        let response = self.client.song_url_v1(&query).await?;
        mapper::track_urls(&response.body, level)
    }

    async fn legacy_dolby_urls(&self, ids: &[u64]) -> Result<Vec<TrackUrl>> {
        let id_values = ids.iter().map(u64::to_string).collect::<Vec<_>>();
        let id_json = serde_json::to_string(&id_values)?;
        let query = self
            .query()
            .param("id", &id_values.join(","))
            .param("br", "999000")
            .param("immerseType", "c51");
        let response = self
            .client
            .request(
                "/api/song/enhance/player/url",
                json!({
                    "ids": id_json,
                    "br": 999000,
                    "immerseType": "c51",
                }),
                Self::request_options(&query),
            )
            .await?;
        self.remember_cookies(response.cookie);
        mapper::legacy_track_urls(&response.body, NcmQualityLevel::Dolby)
    }

    pub async fn song_quality(&self, song_id: u64) -> Result<SongQualityDetail> {
        let query = self.query().param("id", &song_id.to_string());
        let response = self.client.song_music_detail(&query).await?;
        mapper::song_quality_detail(&response.body, song_id)
    }

    /// Resolve a batch of URLs using SPlayer's quality negotiation policy:
    /// regular levels ask `/song/url/v1` and preserve the server-returned
    /// level, while Dolby uses the legacy endpoint and the documented
    /// hires/lossless/exhigh adaptation ladder when Dolby is unavailable.
    pub async fn resolve_track_urls(
        &self,
        ids: &[u64],
        requested: NcmQualityLevel,
    ) -> Result<Vec<TrackUrl>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        if requested == NcmQualityLevel::Dolby {
            let results = stream::iter(ids.iter().copied().map(|song_id| {
                let client = self.clone();
                async move { client.resolve_track_url(song_id, requested).await }
            }))
            .buffered(3)
            .collect::<Vec<_>>()
            .await;
            return results.into_iter().collect();
        }

        let urls = self.track_urls_for_level(ids, requested).await?;
        ids.iter()
            .map(|song_id| {
                urls.iter()
                    .find(|url| url.id == *song_id)
                    .cloned()
                    .ok_or_else(|| {
                        NcmError::protocol(format!(
                            "official URL response omitted song {} for quality preference {}",
                            song_id,
                            super::models::quality_api_level(requested)
                        ))
                    })
            })
            .collect()
    }

    /// Resolve one playable URL while preserving both the requested and actual
    /// quality. This follows SPlayer's policy: only Dolby preflights the
    /// compact quality detail response; other levels accept the server's
    /// returned level from `/song/url/v1`.
    pub async fn resolve_track_url(
        &self,
        song_id: u64,
        requested: NcmQualityLevel,
    ) -> Result<TrackUrl> {
        if requested == NcmQualityLevel::Dolby {
            let detail = self.song_quality(song_id).await?;
            let selected = if detail.best_for(NcmQualityLevel::Dolby).is_some() {
                NcmQualityLevel::Dolby
            } else {
                [
                    NcmQualityLevel::HiRes,
                    NcmQualityLevel::Lossless,
                    NcmQualityLevel::ExHigh,
                ]
                .into_iter()
                .find(|level| detail.best_for(*level).is_some())
                .ok_or_else(|| {
                    NcmError::business(format!(
                        "Dolby and the SPlayer adaptation ladder are unavailable for song {song_id}"
                    ))
                })?
            };

            let mut urls = if selected == NcmQualityLevel::Dolby {
                self.legacy_dolby_urls(&[song_id]).await?
            } else {
                self.track_urls_for_level(&[song_id], selected).await?
            };
            let mut url = urls
                .drain(..)
                .find(|track| track.id == song_id)
                .ok_or_else(|| NcmError::business(format!("no playable URL for song {song_id}")))?;
            url.requested_level = requested;
            return Ok(url);
        }

        self.track_urls_for_level(&[song_id], requested)
            .await?
            .into_iter()
            .find(|track| track.id == song_id)
            .ok_or_else(|| {
                NcmError::protocol(format!(
                    "official URL response omitted song {song_id} for quality preference {}",
                    super::models::quality_api_level(requested)
                ))
            })
    }

    pub async fn track_detail(&self, ids: &[u64]) -> Result<Vec<Track>> {
        let id_list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let query = self.query().param("ids", &id_list);
        let response = self.client.song_detail(&query).await?;
        mapper::track_detail(&response.body)
    }

    pub async fn song_lyric(&self, music_id: u64) -> Result<Lyrics> {
        let query = self.query().param("id", &music_id.to_string());
        // Use the v1 endpoint, which is the only standard NCM endpoint that
        // returns the word-level YRC payload. The legacy endpoint only
        // reliably returns line-level LRC data.
        let response = self.client.lyric_new(&query).await?;
        mapper::lyrics(&response.body)
    }

    pub async fn scrobble_song(
        &self,
        song_id: u64,
        source_id: Option<u64>,
        time_secs: u64,
    ) -> Result<()> {
        let mut query = self
            .query()
            .param("id", &song_id.to_string())
            .param("time", &time_secs.to_string());
        let source_id = source_id.map(|id| id.to_string()).unwrap_or_default();
        if !source_id.is_empty() {
            query = query.param("sourceid", &source_id);
        }
        self.client.scrobble(&query).await?;
        Ok(())
    }

    pub async fn recommend_playlists(&self) -> Result<Vec<PlaylistSummary>> {
        let response = self.client.recommend_resource(&self.query()).await?;
        mapper::playlist_summaries(&response.body, PlaylistSource::Recommend)
    }

    pub async fn recommend_tracks(&self) -> Result<Vec<Track>> {
        let response = self.client.recommend_songs(&self.query()).await?;
        Ok(mapper::tracks_from_array(
            response
                .body
                .get("data")
                .and_then(|data| data.get("dailySongs"))
                .and_then(serde_json::Value::as_array),
            None,
        ))
    }

    pub async fn high_quality_playlists(
        &self,
        cat: &str,
        before: Option<u64>,
        limit: u16,
    ) -> Result<Vec<PlaylistSummary>> {
        let mut query = self
            .query()
            .param("cat", cat)
            .param("limit", &limit.to_string());
        let before_value;
        if let Some(before) = before {
            before_value = before.to_string();
            query = query.param("before", &before_value);
        }
        let response = self.client.top_playlist_highquality(&query).await?;
        mapper::playlist_summaries(&response.body, PlaylistSource::Top)
    }

    pub async fn hot_playlists(
        &self,
        cat: &str,
        offset: u16,
        limit: u16,
    ) -> Result<Vec<PlaylistSummary>> {
        let query = self
            .query()
            .param("cat", cat)
            .param("order", "hot")
            .param("offset", &offset.to_string())
            .param("limit", &limit.to_string());
        let response = self.client.top_playlist(&query).await?;
        mapper::playlist_summaries(&response.body, PlaylistSource::Top)
    }

    pub async fn like_song(&self, track_id: u64, like: bool) -> Result<()> {
        let query = self
            .query()
            .param("id", &track_id.to_string())
            .param("like", if like { "true" } else { "false" });
        self.client.like(&query).await?;
        Ok(())
    }

    pub async fn download_img<I>(
        &self,
        url: I,
        path: PathBuf,
        resize: Option<(u16, u16)>,
    ) -> Result<()>
    where
        I: Into<String>,
    {
        if !path.exists() {
            let image_url = image_download_url(url.into(), resize);
            let response = reqwest::Client::new().get(&image_url).send().await?;
            if response.status().is_success() {
                let bytes = response.bytes().await?;
                std::fs::write(&path, bytes)?;
            }
        }
        Ok(())
    }

    pub async fn playlist_subscribe(&self, subscribe: bool, playlist_id: u64) -> Result<()> {
        let query = self
            .query()
            .param("id", &playlist_id.to_string())
            .param("t", if subscribe { "1" } else { "0" });
        self.client.playlist_subscribe(&query).await?;
        Ok(())
    }

    pub async fn personal_fm_tracks(&self) -> Result<Vec<Track>> {
        let response = self.client.personal_fm(&self.query()).await?;
        Ok(mapper::tracks_from_array(
            response
                .body
                .get("data")
                .and_then(serde_json::Value::as_array),
            None,
        ))
    }

    pub async fn search(
        &self,
        keywords: &str,
        search_type: SearchType,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResponse> {
        let query = self
            .query()
            .param("keywords", keywords)
            .param("type", search_type.as_str())
            .param("limit", &limit.to_string())
            .param("offset", &offset.to_string());
        let response = self.client.cloudsearch(&query).await?;
        mapper::search(&response.body, search_type)
    }

    pub async fn artist_albums(&self, artist_id: u64, limit: u32) -> Result<Vec<AlbumSummary>> {
        let query = self
            .query()
            .param("id", &artist_id.to_string())
            .param("limit", &limit.to_string())
            .param("offset", "0");
        let response = self.client.artist_album(&query).await?;
        mapper::album_summaries(&response.body, AlbumSource::Artist)
    }

    pub async fn playlist_add_tracks(&self, pid: u64, track_ids: &str, op: &str) -> Result<()> {
        let query = self
            .query()
            .param("pid", &pid.to_string())
            .param("tracks", track_ids)
            .param("op", op);
        self.client.playlist_tracks(&query).await?;
        Ok(())
    }
}

fn cookie_pair(raw: &str) -> Option<(String, String)> {
    let pair = raw.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), value.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::image_download_url;

    #[test]
    fn image_download_url_preserves_original_when_resize_is_absent() {
        let original = "https://p1.music.126.net/vip.png?auth=1";
        assert_eq!(image_download_url(original.to_string(), None), original);
        assert_eq!(
            image_download_url(original.to_string(), Some((200, 200))),
            "https://p1.music.126.net/vip.png?auth=1&param=200y200"
        );
    }
}
