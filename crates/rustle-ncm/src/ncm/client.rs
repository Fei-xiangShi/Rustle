use base64::Engine;
use futures_util::stream::{self, StreamExt};
use md5::{Digest, Md5};
use ncm_api_rs::{ApiClient, CryptoType, Query, RequestOption, create_client};
use serde_json::json;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};

use super::mapper::{self, AlbumSource, PlaylistSource};
use super::models::*;
use super::session::SessionState;
use super::{NcmError, NcmResult as Result};

const COOKIE_FILE: &str = "cookies.json";
const DEFAULT_QUALITY: u32 = 2;
const PLAYLIST_DETAIL_CHUNK_SIZE: usize = 500;
const NCM_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const IMAGE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const IMAGE_DOWNLOAD_ATTEMPTS: usize = 2;
const LOGIN_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(60);
const CLIENT_LOG_DOMAIN: &str = "https://clientlog.music.163.com";

// Keep the forwarded address stable across clones, logins, and client rebuilds.
static SESSION_REAL_IP: OnceLock<String> = OnceLock::new();

fn is_ncm_image_host(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("music.126.net")
            || host.to_ascii_lowercase().ends_with(".music.126.net")
    })
}

fn image_download_url(url: String, resize: Option<(u16, u16)>) -> String {
    let Some((width, height)) = resize else {
        return url;
    };
    let Ok(mut parsed) = reqwest::Url::parse(&url) else {
        return url;
    };
    if !is_ncm_image_host(&parsed) {
        return url;
    };
    let retained_pairs = parsed
        .query_pairs()
        .filter(|(key, _)| key != "param")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    {
        let mut pairs = parsed.query_pairs_mut();
        pairs.extend_pairs(retained_pairs);
        pairs.append_pair("param", &format!("{width}y{height}"));
    }
    parsed.into()
}

fn original_image_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    if !is_ncm_image_host(&parsed) {
        return url.to_string();
    }

    let retained_pairs = parsed
        .query_pairs()
        .filter(|(key, _)| key != "param")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    if !retained_pairs.is_empty() {
        parsed.query_pairs_mut().extend_pairs(retained_pairs);
    }
    parsed.into()
}

fn image_download_candidates(url: String, resize: Option<(u16, u16)>) -> Vec<String> {
    let resized = image_download_url(url.clone(), resize);
    let original = original_image_url(&url);
    let mut candidates = vec![resized];
    if !candidates.contains(&original) {
        candidates.push(original);
    }
    candidates
}

fn has_supported_image_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
        || bytes.starts_with(b"BM")
}

#[derive(Clone)]
pub struct NcmClient {
    client: ApiClient,
    session: Arc<parking_lot::RwLock<SessionState>>,
    session_path: Arc<PathBuf>,
    anonymous_bootstrap: Arc<tokio::sync::Mutex<()>>,
    login_refresh: Arc<tokio::sync::Mutex<Option<Instant>>>,
    proxy: Option<String>,
    quality: Arc<AtomicU32>,
    overseas_compatibility: Arc<AtomicBool>,
}

impl std::fmt::Debug for NcmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NcmClient")
            .field("client", &"<ncm_api_rs::ApiClient>")
            .field("session", &*self.session.read())
            .field("has_proxy", &self.proxy.is_some())
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
        Self::from_session_and_proxy(SessionState::new(cookie), proxy, Self::cookie_file_path())
    }

    fn from_session_and_proxy(
        session: SessionState,
        proxy: Option<String>,
        session_path: PathBuf,
    ) -> Self {
        let request_cookie = session.request_cookie();
        let mut client = create_client(Some(request_cookie));
        client.set_device_id(session.device_id().to_string());
        if let Some(token) = session.anonymous_token() {
            client.set_anonymous_token(token.to_string());
        }
        Self {
            client,
            session: Arc::new(parking_lot::RwLock::new(session)),
            session_path: Arc::new(session_path),
            anonymous_bootstrap: Arc::new(tokio::sync::Mutex::new(())),
            login_refresh: Arc::new(tokio::sync::Mutex::new(None)),
            proxy,
            quality: Arc::new(AtomicU32::new(DEFAULT_QUALITY)),
            overseas_compatibility: Arc::new(AtomicBool::new(false)),
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

    pub fn from_session_file_with_proxy(proxy: Option<String>) -> Result<Option<Self>> {
        let path = Self::cookie_file_path();
        Self::from_session_path_with_proxy(path, proxy)
    }

    fn from_session_path_with_proxy(path: PathBuf, proxy: Option<String>) -> Result<Option<Self>> {
        let Some(session) = SessionState::load(&path)? else {
            return Ok(None);
        };
        session.save(&path)?;
        Ok(Some(Self::from_session_and_proxy(session, proxy, path)))
    }

    pub fn has_authenticated_session(&self) -> bool {
        self.session.read().has_login()
    }

    pub fn save_cookie_to_file(&self) {
        if let Err(error) = self.save_session() {
            tracing::error!(
                error_code = %error.code().as_str(),
                "ncm_session_save_failed"
            );
        }
    }

    pub fn set_proxy(&mut self, proxy: String) -> Result<()> {
        self.proxy = Some(proxy);
        Ok(())
    }

    pub fn clear_proxy(&mut self) {
        self.proxy = None;
    }

    /// Apply to subsequent API requests, including requests from existing clones.
    pub fn set_overseas_compatibility(&self, enabled: bool) {
        self.overseas_compatibility
            .store(enabled, Ordering::Relaxed);
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
        self.query_with_cookie(self.session.read().request_cookie())
    }

    fn query_for_os(&self, os: &str) -> Query {
        self.query_with_cookie(self.session.read().request_cookie_for_os(os))
    }

    fn query_with_cookie(&self, cookie: String) -> Query {
        let mut query = Query::new();
        if !cookie.trim().is_empty() {
            query = query.cookie(&cookie);
        }
        if let Some(proxy) = &self.proxy {
            query.proxy = Some(proxy.clone());
        }
        if self.overseas_compatibility.load(Ordering::Relaxed) {
            query.real_ip = Some(
                SESSION_REAL_IP
                    .get_or_init(ncm_api_rs::util::ip::generate_random_chinese_ip)
                    .clone(),
            );
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

    async fn request_with_timeout<T, F>(operation: &'static str, future: F) -> Result<T>
    where
        F: Future<Output = std::result::Result<T, ncm_api_rs::NcmError>>,
    {
        Self::request_with_deadline(NCM_REQUEST_TIMEOUT, operation, future).await
    }

    async fn request_with_deadline<T, F>(
        deadline: Duration,
        operation: &'static str,
        future: F,
    ) -> Result<T>
    where
        F: Future<Output = std::result::Result<T, ncm_api_rs::NcmError>>,
    {
        match tokio::time::timeout(deadline, future).await {
            Ok(result) => result.map_err(NcmError::from),
            Err(_) => Err(NcmError::Timeout { operation }),
        }
    }

    fn save_session(&self) -> Result<()> {
        // Keep the snapshot protected until publication so an older save cannot
        // overwrite a newer login/logout written by another clone.
        self.session.read().save(&self.session_path)
    }

    pub async fn ensure_anonymous_session(&self) -> Result<()> {
        self.ensure_anonymous_session_with(|username| async move {
            let query = self.query();
            let response = Self::request_with_timeout(
                "register_anonymous",
                self.client.request(
                    "/api/register/anonimous",
                    json!({ "username": username }),
                    RequestOption {
                        crypto: CryptoType::Weapi,
                        ..Self::request_options(&query)
                    },
                ),
            )
            .await?;
            let token = response
                .body
                .get("token")
                .and_then(serde_json::Value::as_str)
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| NcmError::protocol("anonymous session response omitted MUSIC_A"))?
                .to_string();
            Ok((token, response.cookie))
        })
        .await
    }

    pub fn invalidate_authenticated_session(&self) -> Result<()> {
        self.session.write().clear_authenticated_session();
        self.save_session()
    }

    async fn ensure_anonymous_session_with<F, Fut>(&self, register: F) -> Result<()>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<(String, Vec<String>)>>,
    {
        if self.session.read().has_session_identity() {
            return Ok(());
        }

        let _guard = self.anonymous_bootstrap.lock().await;
        if self.session.read().has_session_identity() {
            return Ok(());
        }

        let device_id = self.session.read().device_id().to_string();
        self.save_session()?;
        let username = anonymous_username(&device_id);
        let (token, cookies) = register(username).await?;

        let mut session = self.session.write();
        session.merge_response_cookies(cookies);
        session.set_anonymous_token(token);
        drop(session);
        self.save_session()
    }

    fn remember_cookies(&self, cookies: Vec<String>) {
        self.session.write().merge_response_cookies(cookies);
    }

    pub async fn create_qrcode(&self) -> Result<(PathBuf, String)> {
        let response =
            Self::request_with_timeout("login_qr_key", self.client.login_qr_key(&self.query()))
                .await?;
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
        let response =
            Self::request_with_timeout("login_qr_check", self.client.login_qr_check(&query))
                .await?;
        self.remember_cookies(response.cookie);
        Ok(mapper::msg(&response.body))
    }

    pub async fn login_status(&self) -> Result<LoginInfo> {
        let response =
            Self::request_with_timeout("login_status", self.client.login_status(&self.query()))
                .await?;
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

    pub async fn authenticated_login_status(&self) -> Result<LoginInfo> {
        let login = self.login_status().await?;
        if login.code == 200 && login.user_id > 0 {
            Ok(login)
        } else {
            Err(NcmError::Authentication(
                "saved NCM login is no longer valid".to_string(),
            ))
        }
    }

    pub async fn refresh_login(&self) -> Result<()> {
        self.refresh_login_with(|| async {
            let response = Self::request_with_timeout(
                "login_refresh",
                self.client.login_refresh(&self.query()),
            )
            .await?;
            if !mapper::confirmed_code_ok(&response.body) {
                return Err(NcmError::business("login refresh was not confirmed"));
            }
            Ok(response.cookie)
        })
        .await
    }

    async fn refresh_login_with<F, Fut>(&self, refresh: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<String>>>,
    {
        if !self.has_authenticated_session() {
            return Err(NcmError::Authentication(
                "login refresh requires an authenticated session".to_string(),
            ));
        }
        let mut last_success = self.login_refresh.lock().await;
        if last_success
            .as_ref()
            .is_some_and(|last| last.elapsed() < LOGIN_REFRESH_MIN_INTERVAL)
        {
            return Ok(());
        }
        self.remember_cookies(refresh().await?);
        self.save_session()?;
        *last_success = Some(Instant::now());
        Ok(())
    }

    async fn login_session_is_valid(&self) -> Result<bool> {
        let response = Self::request_with_timeout(
            "login_status_recovery",
            self.client.login_status(&self.query()),
        )
        .await?;
        self.remember_cookies(response.cookie);
        Ok(mapper::login_status_has_authenticated_account(
            &response.body,
        ))
    }

    async fn recover_playback_authentication(&self) -> Result<()> {
        match self.login_session_is_valid().await {
            Ok(true) => self.refresh_login().await,
            Ok(false) => {
                self.invalidate_authenticated_session()?;
                Err(NcmError::Authentication(
                    "NCM login expired while resolving playback".to_string(),
                ))
            }
            Err(error) if error.is_authentication() => {
                self.invalidate_authenticated_session()?;
                Err(NcmError::Authentication(
                    "NCM login expired while resolving playback".to_string(),
                ))
            }
            Err(error) => Err(error),
        }
    }

    pub async fn account_info(&self) -> Result<LoginInfo> {
        let response =
            Self::request_with_timeout("user_account", self.client.user_account(&self.query()))
                .await?;
        self.remember_cookies(response.cookie);
        mapper::account_info(&response.body)
    }

    /// Fetch the official membership image projection. Account/status
    /// endpoints do not reliably include badge URLs on current NCM accounts.
    pub async fn membership_info(&self, user_id: u64, base: &VipInfo) -> Result<VipInfo> {
        let query = self.query().param("uid", &user_id.to_string());
        let response = Self::request_with_timeout("vip_info", self.client.vip_info(&query)).await?;
        self.remember_cookies(response.cookie);
        mapper::merge_membership_vip(base, &response.body)
    }

    pub async fn logout(&self) -> Result<()> {
        let _ = Self::request_with_timeout("logout", self.client.logout(&self.query())).await;
        self.session.write().clear_authenticated_session();
        self.save_session()?;
        self.ensure_anonymous_session().await
    }

    pub async fn user_song_id_list(&self, user_id: u64) -> Result<Vec<u64>> {
        let query = self.query().param("uid", &user_id.to_string());
        let response = Self::request_with_timeout("likelist", self.client.likelist(&query)).await?;
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
        let response =
            Self::request_with_timeout("user_playlist", self.client.user_playlist(&query)).await?;
        mapper::playlist_summaries(&response.body, PlaylistSource::User)
    }

    pub async fn playlist_detail(&self, playlist_id: u64) -> Result<PlaylistDetail> {
        let query = self.query().param("id", &playlist_id.to_string());
        let response =
            Self::request_with_timeout("playlist_detail", self.client.playlist_detail(&query))
                .await?;
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
        let response = Self::request_with_timeout(
            "playlist_detail_preview",
            self.client.request(
                "/api/v6/playlist/detail",
                json!({
                    "id": playlist_id.to_string(),
                    "n": 0,
                    "s": 8,
                }),
                Self::request_options(&query),
            ),
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
        let response = Self::request_with_timeout("artists", self.client.artists(&query)).await?;
        mapper::artist_detail(&response.body)
    }

    pub async fn album_detail(&self, album_id: u64) -> Result<AlbumDetail> {
        let query = self.query().param("id", &album_id.to_string());
        let response = Self::request_with_timeout("album", self.client.album(&query)).await?;
        mapper::album_detail(&response.body)
    }

    pub async fn user_detail(&self, user_id: u64) -> Result<UserDetail> {
        let query = self.query().param("uid", &user_id.to_string());
        let response =
            Self::request_with_timeout("user_detail", self.client.user_detail(&query)).await?;
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
        let response =
            Self::request_with_timeout("song_url_v1", self.client.song_url_v1(&query)).await?;
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
        let response = Self::request_with_timeout(
            "legacy_dolby_url",
            self.client.request(
                "/api/song/enhance/player/url",
                json!({
                    "ids": id_json,
                    "br": 999000,
                    "immerseType": "c51",
                }),
                Self::request_options(&query),
            ),
        )
        .await?;
        self.remember_cookies(response.cookie);
        mapper::legacy_track_urls(&response.body, NcmQualityLevel::Dolby)
    }

    pub async fn song_quality(&self, song_id: u64) -> Result<SongQualityDetail> {
        let query = self.query().param("id", &song_id.to_string());
        let response =
            Self::request_with_timeout("song_music_detail", self.client.song_music_detail(&query))
                .await?;
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
        let authenticated = self.has_authenticated_session();
        retry_playback_once(
            authenticated,
            || self.resolve_track_urls_once(ids, requested),
            || self.recover_playback_authentication(),
        )
        .await
    }

    async fn resolve_track_urls_once(
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
                async move { client.resolve_track_url_once(song_id, requested).await }
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
                    .ok_or(NcmError::PlaybackUnavailable { song_id: *song_id })
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
        let authenticated = self.has_authenticated_session();
        retry_playback_once(
            authenticated,
            || self.resolve_track_url_once(song_id, requested),
            || self.recover_playback_authentication(),
        )
        .await
    }

    async fn resolve_track_url_once(
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
                .ok_or(NcmError::PlaybackUnavailable { song_id })?;
            url.requested_level = requested;
            return Ok(url);
        }

        self.track_urls_for_level(&[song_id], requested)
            .await?
            .into_iter()
            .find(|track| track.id == song_id)
            .ok_or(NcmError::PlaybackUnavailable { song_id })
    }

    pub async fn track_detail(&self, ids: &[u64]) -> Result<Vec<Track>> {
        let id_list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let query = self.query().param("ids", &id_list);
        let response =
            Self::request_with_timeout("song_detail", self.client.song_detail(&query)).await?;
        mapper::track_detail(&response.body)
    }

    pub async fn song_lyric(&self, music_id: u64) -> Result<Lyrics> {
        let query = self.query().param("id", &music_id.to_string());
        // Use the v1 endpoint, which is the only standard NCM endpoint that
        // returns the word-level YRC payload. The legacy endpoint only
        // reliably returns line-level LRC data.
        let response =
            Self::request_with_timeout("lyric_new", self.client.lyric_new(&query)).await?;
        mapper::lyrics(&response.body)
    }

    pub async fn scrobble_song(
        &self,
        song_id: u64,
        source_id: Option<u64>,
        time_secs: u64,
    ) -> Result<ScrobbleResult> {
        if song_id == 0 || time_secs == 0 {
            return Err(NcmError::protocol(
                "scrobble requires a song identity and positive play time",
            ));
        }
        let source_id = source_id.unwrap_or(song_id);
        let (startplay_data, play_data) = scrobble_payloads(song_id, source_id, time_secs)?;
        let query = self.query_for_os("osx");
        let options = RequestOption {
            crypto: CryptoType::Eapi,
            domain: Some(CLIENT_LOG_DOMAIN.to_string()),
            ..Self::request_options(&query)
        };
        let startplay = Self::request_with_timeout(
            "scrobble_startplay",
            self.client
                .request("/api/feedback/weblog", startplay_data, options.clone()),
        )
        .await?;
        self.remember_cookies(startplay.cookie);
        let play = Self::request_with_timeout(
            "scrobble_play",
            self.client
                .request("/api/feedback/weblog", play_data, options),
        )
        .await?;
        self.remember_cookies(play.cookie);
        let result = ScrobbleResult {
            startplay_confirmed: mapper::scrobble_stage_confirmed(&startplay.body),
            play_confirmed: mapper::scrobble_stage_confirmed(&play.body),
        };
        if !result.startplay_confirmed || !result.play_confirmed {
            return Err(NcmError::business(
                "NCM did not confirm both scrobble stages",
            ));
        }
        Ok(result)
    }

    pub async fn recommend_playlists(&self) -> Result<Vec<PlaylistSummary>> {
        let response = Self::request_with_timeout(
            "recommend_resource",
            self.client.recommend_resource(&self.query()),
        )
        .await?;
        mapper::playlist_summaries(&response.body, PlaylistSource::Recommend)
    }

    pub async fn recommend_tracks(&self) -> Result<Vec<Track>> {
        let response = Self::request_with_timeout(
            "recommend_songs",
            self.client.recommend_songs(&self.query()),
        )
        .await?;
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
        let response = Self::request_with_timeout(
            "top_playlist_highquality",
            self.client.top_playlist_highquality(&query),
        )
        .await?;
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
        let response =
            Self::request_with_timeout("top_playlist", self.client.top_playlist(&query)).await?;
        mapper::playlist_summaries(&response.body, PlaylistSource::Top)
    }

    pub async fn like_song(&self, track_id: u64, like: bool) -> Result<LikeResult> {
        if track_id == 0 {
            return Err(NcmError::protocol("like mutation requires a track ID"));
        }
        let query = self.query();
        let response = Self::request_with_timeout(
            "like_legacy_weapi",
            self.client.request(
                "/api/song/like",
                like_payload(track_id, like),
                RequestOption {
                    crypto: CryptoType::Weapi,
                    ..Self::request_options(&query)
                },
            ),
        )
        .await?;
        self.remember_cookies(response.cookie);
        mapper::like_result(&response.body, like)
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
        if path.exists() {
            return Ok(());
        }

        let bytes = self.image_bytes(url, resize).await?;
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    /// Fetch disposable artwork without creating a persistent image file.
    pub async fn image_bytes<I: Into<String>>(
        &self,
        url: I,
        resize: Option<(u16, u16)>,
    ) -> Result<Vec<u8>> {
        let source_url = url.into();
        let candidates = image_download_candidates(source_url, resize);
        let mut builder = reqwest::Client::builder().timeout(IMAGE_REQUEST_TIMEOUT);
        if let Some(proxy) = &self.proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        let client = builder.build()?;
        let mut last_http_error = None;
        let mut last_protocol_error = None;

        for image_url in candidates {
            for attempt in 0..IMAGE_DOWNLOAD_ATTEMPTS {
                let response = match client.get(&image_url).send().await {
                    Ok(response) => response,
                    Err(error) => {
                        last_http_error = Some(error);
                        continue;
                    }
                };
                let status = response.status();
                if !status.is_success() {
                    last_http_error = response.error_for_status().err();
                    if status.is_server_error() && attempt + 1 < IMAGE_DOWNLOAD_ATTEMPTS {
                        continue;
                    }
                    break;
                }

                let bytes = match response.bytes().await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        last_http_error = Some(error);
                        continue;
                    }
                };
                if !has_supported_image_signature(&bytes) {
                    last_protocol_error = Some(format!(
                        "image endpoint returned non-image bytes for `{image_url}`"
                    ));
                    break;
                }

                return Ok(bytes.to_vec());
            }
        }

        if let Some(error) = last_http_error {
            Err(error.into())
        } else {
            Err(NcmError::protocol(last_protocol_error.unwrap_or_else(
                || "image endpoint returned no usable response".to_string(),
            )))
        }
    }

    pub async fn playlist_subscribe(&self, subscribe: bool, playlist_id: u64) -> Result<()> {
        let query = self
            .query()
            .param("id", &playlist_id.to_string())
            .param("t", if subscribe { "1" } else { "0" });
        Self::request_with_timeout("playlist_subscribe", self.client.playlist_subscribe(&query))
            .await?;
        Ok(())
    }

    pub async fn personal_fm_tracks(&self) -> Result<Vec<Track>> {
        let response =
            Self::request_with_timeout("personal_fm", self.client.personal_fm(&self.query()))
                .await?;
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
        let response =
            Self::request_with_timeout("cloudsearch", self.client.cloudsearch(&query)).await?;
        mapper::search(&response.body, search_type)
    }

    pub async fn artist_albums(&self, artist_id: u64, limit: u32) -> Result<Vec<AlbumSummary>> {
        let query = self
            .query()
            .param("id", &artist_id.to_string())
            .param("limit", &limit.to_string())
            .param("offset", "0");
        let response =
            Self::request_with_timeout("artist_album", self.client.artist_album(&query)).await?;
        mapper::album_summaries(&response.body, AlbumSource::Artist)
    }

    pub async fn mutate_playlist_tracks(
        &self,
        playlist_id: u64,
        track_ids: &[u64],
        operation: PlaylistTrackOperation,
    ) -> Result<PlaylistTrackMutation> {
        if playlist_id == 0 {
            return Err(NcmError::protocol(
                "playlist mutation requires a playlist ID",
            ));
        }
        let requested_track_ids = normalized_track_ids(track_ids);
        if requested_track_ids.is_empty() {
            return Err(NcmError::protocol(
                "playlist mutation requires at least one track ID",
            ));
        }
        let query = playlist_track_query(self, playlist_id, &requested_track_ids, operation, false);
        let (response, retried_after_code_512) = match Self::request_with_timeout(
            "playlist_tracks",
            self.client.playlist_tracks(&query),
        )
        .await
        {
            Ok(response) => (response, false),
            Err(error) if is_playlist_code_512(&error) => {
                let retry_query =
                    playlist_track_query(self, playlist_id, &requested_track_ids, operation, true);
                (
                    Self::request_with_timeout(
                        "playlist_tracks_code_512_retry",
                        self.client.playlist_tracks(&retry_query),
                    )
                    .await?,
                    true,
                )
            }
            Err(error) => return Err(error),
        };
        self.remember_cookies(response.cookie);
        mapper::playlist_track_mutation(
            &response.body,
            operation,
            playlist_id,
            &requested_track_ids,
            retried_after_code_512,
        )
    }
}

async fn retry_playback_once<T, Attempt, AttemptFuture, Recover, RecoverFuture>(
    authenticated: bool,
    mut attempt: Attempt,
    recover: Recover,
) -> Result<T>
where
    Attempt: FnMut() -> AttemptFuture,
    AttemptFuture: Future<Output = Result<T>>,
    Recover: FnOnce() -> RecoverFuture,
    RecoverFuture: Future<Output = Result<()>>,
{
    match attempt().await {
        Err(error) if authenticated && error.is_playback_auth_candidate() => {
            recover().await?;
            attempt().await
        }
        result => result,
    }
}

fn like_payload(track_id: u64, like: bool) -> serde_json::Value {
    json!({
        "trackId": track_id,
        "like": like,
        "time": 3,
    })
}

fn scrobble_payloads(
    song_id: u64,
    source_id: u64,
    time_secs: u64,
) -> Result<(serde_json::Value, serde_json::Value)> {
    let shared = json!({
        "id": song_id.to_string(),
        "type": "song",
        "mainsite": "1",
        "mainsiteWeb": "1",
        "content": format!("id={source_id}"),
        "sourceId": source_id.to_string(),
        "source": "track",
        "sourcetype": "track",
    });
    let startplay = serde_json::to_string(&json!([{
        "action": "startplay",
        "json": shared.clone(),
    }]))?;
    let mut play_fields = shared;
    let play_object = play_fields
        .as_object_mut()
        .ok_or_else(|| NcmError::protocol("scrobble payload must be an object"))?;
    play_object.insert("download".to_string(), json!(0));
    play_object.insert("end".to_string(), json!("playend"));
    play_object.insert("time".to_string(), json!(time_secs));
    play_object.insert("wifi".to_string(), json!(0));
    let play = serde_json::to_string(&json!([{
        "action": "play",
        "json": play_fields,
    }]))?;
    Ok((json!({ "logs": startplay }), json!({ "logs": play })))
}

fn normalized_track_ids(track_ids: &[u64]) -> Vec<u64> {
    let mut normalized = Vec::with_capacity(track_ids.len());
    for track_id in track_ids.iter().copied().filter(|track_id| *track_id > 0) {
        if !normalized.contains(&track_id) {
            normalized.push(track_id);
        }
    }
    normalized
}

fn playlist_track_query(
    client: &NcmClient,
    playlist_id: u64,
    track_ids: &[u64],
    operation: PlaylistTrackOperation,
    duplicate: bool,
) -> Query {
    let mut serialized_ids = track_ids.iter().map(u64::to_string).collect::<Vec<_>>();
    if duplicate {
        serialized_ids.extend(track_ids.iter().map(u64::to_string));
    }
    client
        .query()
        .param("pid", &playlist_id.to_string())
        .param("tracks", &serialized_ids.join(","))
        .param("op", operation.as_api_str())
}

fn is_playlist_code_512(error: &NcmError) -> bool {
    matches!(
        error,
        NcmError::Upstream(ncm_api_rs::NcmError::Api { code: 512, .. })
    )
}

fn anonymous_username(device_id: &str) -> String {
    const ID_XOR_KEY: &[u8] = b"3go8&$8*3*3h0k(2)2";
    let xored = device_id
        .bytes()
        .enumerate()
        .map(|(index, byte)| byte ^ ID_XOR_KEY[index % ID_XOR_KEY.len()])
        .collect::<Vec<_>>();
    let digest = base64::engine::general_purpose::STANDARD.encode(Md5::digest(xored));
    base64::engine::general_purpose::STANDARD.encode(format!("{device_id} {digest}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustle_application::error::ErrorCode;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn overseas_compatibility_updates_clones_and_preserves_request_context() {
        let client = NcmClient::from_cookie_with_proxy(
            "MUSIC_U=test-cookie".to_string(),
            Some("http://127.0.0.1:7890".to_string()),
        );
        let cloned = client.clone();
        let original = cloned.query();
        assert!(original.real_ip.is_none());
        assert!(!original.random_cn_ip);

        client.set_overseas_compatibility(true);
        let enabled = cloned.query();
        let ip = enabled.real_ip.as_ref().unwrap();
        assert!(ip.parse::<std::net::Ipv4Addr>().is_ok());
        assert_eq!(enabled.cookie, original.cookie);
        assert_eq!(enabled.proxy, original.proxy);
        assert_eq!(cloned.query_for_os("pc").real_ip.as_ref(), Some(ip));
        assert_eq!(
            NcmClient::request_options(&enabled).real_ip.as_ref(),
            Some(ip)
        );

        cloned.set_overseas_compatibility(false);
        assert!(client.query().real_ip.is_none());
        client.set_overseas_compatibility(true);
        assert_eq!(client.query().real_ip.as_ref(), Some(ip));

        let rebuilt = NcmClient::new();
        assert!(rebuilt.query().real_ip.is_none());
        rebuilt.set_overseas_compatibility(true);
        assert_eq!(rebuilt.query().real_ip.as_ref(), Some(ip));
    }

    fn test_session_path(name: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rustle-ncm-client-test-{}-{counter}-{name}.json",
            std::process::id()
        ))
    }

    #[test]
    fn image_download_url_preserves_original_when_resize_is_absent() {
        let original = "https://p1.music.126.net/vip.png?auth=1";
        assert_eq!(image_download_url(original.to_string(), None), original);
        assert_eq!(
            image_download_url(original.to_string(), Some((200, 200))),
            "https://p1.music.126.net/vip.png?auth=1&param=200y200"
        );
    }

    #[test]
    fn image_download_url_replaces_existing_size_and_preserves_other_pairs() {
        assert_eq!(
            image_download_url(
                "https://p1.music.126.net/cover.jpg?param=200y200&auth=a%20b&param=300y300"
                    .to_string(),
                Some((1024, 1024)),
            ),
            "https://p1.music.126.net/cover.jpg?auth=a+b&param=1024y1024"
        );
        assert_eq!(
            image_download_url(
                "https://p1.music.126.net/cover.jpg?auth=1&param=300y300".to_string(),
                Some((600, 600)),
            ),
            "https://p1.music.126.net/cover.jpg?auth=1&param=600y600"
        );
    }

    #[test]
    fn image_download_url_leaves_non_ncm_and_invalid_sources_unchanged() {
        let external = "https://cdn.example.com/cover.jpg?auth=1";
        assert_eq!(
            image_download_url(external.to_string(), Some((600, 600))),
            external
        );

        let invalid = "not a valid image URL";
        assert_eq!(
            image_download_url(invalid.to_string(), Some((600, 600))),
            invalid
        );
    }

    #[test]
    fn image_download_candidates_try_requested_derivative_then_original() {
        assert_eq!(
            image_download_candidates(
                "https://p1.music.126.net/cover.jpg?auth=1&param=300y300".to_string(),
                Some((1024, 1024)),
            ),
            vec![
                "https://p1.music.126.net/cover.jpg?auth=1&param=1024y1024",
                "https://p1.music.126.net/cover.jpg?auth=1",
            ]
        );
        assert_eq!(
            image_download_candidates(
                "https://cdn.example.com/cover.jpg?size=small".to_string(),
                Some((1024, 1024)),
            ),
            vec!["https://cdn.example.com/cover.jpg?size=small"]
        );
    }

    #[test]
    fn image_payload_signature_rejects_successful_html_error_pages() {
        assert!(has_supported_image_signature(&[
            0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0
        ]));
        assert!(has_supported_image_signature(b"RIFFxxxxWEBPpayload"));
        assert!(!has_supported_image_signature(b"<html>not an image</html>"));
        assert!(!has_supported_image_signature(&[]));
    }

    #[test]
    fn anonymous_username_matches_the_stable_device_contract() {
        assert_eq!(
            anonymous_username(&"A".repeat(52)),
            "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQSAzazVJblM0VFJaTGIrLzE3azhIQ0JBPT0="
        );
    }

    #[test]
    fn legacy_session_is_upgraded_when_the_client_loads_it() {
        let path = test_session_path("legacy-upgrade");
        fs::write(&path, br#"{"cookie":"MUSIC_U=login; __csrf=token"}"#).unwrap();

        let client = NcmClient::from_session_path_with_proxy(path.clone(), None)
            .unwrap()
            .unwrap();

        assert!(client.has_authenticated_session());
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["cookie"], "MUSIC_U=login; __csrf=token");
        assert_eq!(saved["device_id"].as_str().unwrap().len(), 52);
        let _ = fs::remove_file(path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn adapter_deadline_maps_to_the_stable_timeout_code() {
        let result =
            NcmClient::request_with_deadline(Duration::from_millis(1), "deadline_test", async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<(), ncm_api_rs::NcmError>(())
            })
            .await;

        let error = result.unwrap_err();
        assert!(matches!(
            error,
            NcmError::Timeout {
                operation: "deadline_test"
            }
        ));
        assert_eq!(error.code(), ErrorCode::NetworkTimeout);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_anonymous_bootstrap_registers_once_and_persists_identity() {
        let path = test_session_path("coalesced-bootstrap");
        let client = NcmClient::from_session_and_proxy(SessionState::new(None), None, path.clone());
        let calls = Arc::new(AtomicUsize::new(0));

        let first_client = client.clone();
        let first_calls = calls.clone();
        let first = first_client.ensure_anonymous_session_with(move |_| async move {
            first_calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(("anonymous".to_string(), Vec::new()))
        });
        let second_client = client.clone();
        let second_calls = calls.clone();
        let second = second_client.ensure_anonymous_session_with(move |_| async move {
            second_calls.fetch_add(1, Ordering::Relaxed);
            Ok(("unexpected-second-token".to_string(), Vec::new()))
        });

        let (first_result, second_result) = futures_util::future::join(first, second).await;

        first_result.unwrap();
        second_result.unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let reloaded = SessionState::load(&path).unwrap().unwrap();
        assert_eq!(reloaded.anonymous_token(), Some("anonymous"));
        assert_eq!(reloaded.device_id(), client.session.read().device_id());
        let _ = fs::remove_file(path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_anonymous_bootstrap_still_persists_the_stable_device() {
        let path = test_session_path("failed-bootstrap");
        let client = NcmClient::from_session_and_proxy(SessionState::new(None), None, path.clone());
        let expected_device = client.session.read().device_id().to_string();

        let result = client
            .ensure_anonymous_session_with(|_| async {
                Err(NcmError::business("anonymous registration rejected"))
            })
            .await;

        assert!(result.is_err());
        let reloaded = SessionState::load(&path).unwrap().unwrap();
        assert_eq!(reloaded.device_id(), expected_device);
        assert!(!reloaded.has_session_identity());
        let _ = fs::remove_file(path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn login_refresh_is_coalesced_and_persists_returned_cookie() {
        let path = test_session_path("coalesced-refresh");
        let client = NcmClient::from_session_and_proxy(
            SessionState::new(Some("MUSIC_U=old; MUSIC_A=anonymous".to_string())),
            None,
            path.clone(),
        );
        let calls = Arc::new(AtomicUsize::new(0));

        let first_client = client.clone();
        let first_calls = calls.clone();
        let first = first_client.refresh_login_with(move || async move {
            first_calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(vec!["MUSIC_U=new; Path=/".to_string()])
        });
        let second_client = client.clone();
        let second_calls = calls.clone();
        let second = second_client.refresh_login_with(move || async move {
            second_calls.fetch_add(1, Ordering::Relaxed);
            Ok(vec!["MUSIC_U=unexpected; Path=/".to_string()])
        });

        let (first_result, second_result) = futures_util::future::join(first, second).await;

        first_result.unwrap();
        second_result.unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let reloaded = SessionState::load(&path).unwrap().unwrap();
        let cookie = reloaded.request_cookie();
        assert!(cookie.contains("MUSIC_U=new"));
        assert!(!cookie.contains("MUSIC_U=unexpected"));
        assert!(!cookie.contains("MUSIC_A=anonymous"));
        let _ = fs::remove_file(path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn playback_authentication_recovery_retries_exactly_once() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let recoveries = Arc::new(AtomicUsize::new(0));
        let attempt_counter = attempts.clone();
        let recovery_counter = recoveries.clone();

        let result = retry_playback_once(
            true,
            move || {
                let attempt = attempt_counter.fetch_add(1, Ordering::Relaxed);
                async move {
                    if attempt == 0 {
                        Err(NcmError::PlaybackUnavailable { song_id: 7 })
                    } else {
                        Ok(7)
                    }
                }
            },
            move || async move {
                recovery_counter.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(result, 7);
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        assert_eq!(recoveries.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn playback_recovery_does_not_retry_unauthenticated_or_network_failures() {
        let unauthenticated_attempts = Arc::new(AtomicUsize::new(0));
        let attempt_counter = unauthenticated_attempts.clone();
        let result = retry_playback_once(
            false,
            move || {
                attempt_counter.fetch_add(1, Ordering::Relaxed);
                async { Err::<(), _>(NcmError::PlaybackUnavailable { song_id: 7 }) }
            },
            || async { Ok(()) },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(unauthenticated_attempts.load(Ordering::Relaxed), 1);

        let network_attempts = Arc::new(AtomicUsize::new(0));
        let attempt_counter = network_attempts.clone();
        let result = retry_playback_once(
            true,
            move || {
                attempt_counter.fetch_add(1, Ordering::Relaxed);
                async {
                    Err::<(), _>(NcmError::Timeout {
                        operation: "playback",
                    })
                }
            },
            || async { Ok(()) },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(network_attempts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn like_payload_matches_splayers_verified_weapi_fallback() {
        assert_eq!(
            like_payload(42, true),
            json!({ "trackId": 42, "like": true, "time": 3 })
        );
    }

    #[test]
    fn scrobble_payloads_are_ordered_startplay_then_play() {
        let (startplay, play) = scrobble_payloads(42, 9, 180).unwrap();
        let start_logs: serde_json::Value =
            serde_json::from_str(startplay["logs"].as_str().unwrap()).unwrap();
        let play_logs: serde_json::Value =
            serde_json::from_str(play["logs"].as_str().unwrap()).unwrap();
        assert_eq!(start_logs[0]["action"], "startplay");
        assert_eq!(start_logs[0]["json"]["sourceId"], "9");
        assert_eq!(play_logs[0]["action"], "play");
        assert_eq!(play_logs[0]["json"]["time"], 180);
        assert_eq!(play_logs[0]["json"]["end"], "playend");
    }

    #[test]
    fn playlist_retry_is_exactly_code_512_and_duplicates_the_payload_once() {
        let error = NcmError::Upstream(ncm_api_rs::NcmError::Api {
            code: 512,
            msg: "restricted".to_string(),
        });
        assert!(is_playlist_code_512(&error));
        assert!(!is_playlist_code_512(&NcmError::Upstream(
            ncm_api_rs::NcmError::Api {
                code: 502,
                msg: "other".to_string(),
            }
        )));

        let client = NcmClient::new();
        let normal = playlist_track_query(&client, 5, &[8, 9], PlaylistTrackOperation::Add, false);
        let retry = playlist_track_query(&client, 5, &[8, 9], PlaylistTrackOperation::Add, true);
        assert_eq!(normal.get_or("tracks", ""), "8,9");
        assert_eq!(retry.get_or("tracks", ""), "8,9,8,9");
    }
}
