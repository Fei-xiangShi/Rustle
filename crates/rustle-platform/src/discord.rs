//! Discord Rich Presence integration
//!
//! Broadcasts "now playing" status to the Discord desktop client via local IPC.
//!
//! - Lazy connection: only connects on first playback event
//! - Silent failure: no error shown to user if Discord is not running
//! - 15-second debounce: Discord enforces a minimum update interval
//! - Full timestamp control: start/end timestamps for progress bar

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::DiscordIpc;
use discord_rich_presence::DiscordIpcClient;
use discord_rich_presence::activity::{Activity, ActivityType, Assets, Button, Timestamps};

const DISCORD_CLIENT_ID: &str = "1505971130908020966";
const MIN_UPDATE_INTERVAL: Duration = Duration::from_secs(15);

pub struct DiscordPresence {
    enabled: bool,
}

impl DiscordPresence {
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

pub fn truncate_field(s: &str) -> String {
    if s.len() <= 125 {
        return s.to_string();
    }
    let mut end = 125;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

pub fn compute_timestamps(position: Duration, duration: Duration) -> Timestamps {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let start = now_ms - position.as_millis() as i64;
    let end = start + duration.as_millis() as i64;
    Timestamps::new().start(start).end(end)
}

pub fn build_activity(
    title: &str,
    artist: &str,
    album: &str,
    art_url: Option<&str>,
    position: Duration,
    duration: Duration,
) -> Activity<'static> {
    let details = truncate_field(title);
    let state = truncate_field(&format!("{} — {}", artist, album));

    let builder = Activity::new()
        .name("Rustle")
        .details(details)
        .state(state)
        .activity_type(ActivityType::Listening)
        .timestamps(compute_timestamps(position, duration))
        .buttons(vec![Button::new(
            "Play on Rustle",
            "https://github.com/Fei-xiangShi/Rustle",
        )]);

    if let Some(url) = art_url {
        builder.assets(Assets::new().large_image(url.to_string()))
    } else {
        builder
    }
}

pub fn build_activity_minimal(
    filename: &str,
    position: Duration,
    duration: Duration,
) -> Activity<'static> {
    Activity::new()
        .name("Rustle")
        .details(truncate_field(filename))
        .state("Unknown Artist — Unknown Album".to_string())
        .activity_type(ActivityType::Listening)
        .timestamps(compute_timestamps(position, duration))
        .buttons(vec![Button::new(
            "Play on Rustle",
            "https://github.com/Fei-xiangShi/Rustle",
        )])
}

/// Build Activity from potentially incomplete metadata.
/// Auto-selects the right variant based on available fields.
pub fn build_activity_safe(
    title: Option<&str>,
    artist: Option<&str>,
    album: Option<&str>,
    art_url: Option<&str>,
    filename: Option<&str>,
    position: Duration,
    duration: Duration,
) -> Activity<'static> {
    match (
        title.filter(|t| !t.is_empty()),
        artist.filter(|a| !a.is_empty()),
    ) {
        (Some(t), Some(a)) => {
            let album_str = album.unwrap_or("Unknown Album");
            build_activity(t, a, album_str, art_url, position, duration)
        }
        (Some(t), _) => build_activity_minimal(t, position, duration),
        _ => {
            let name = filename.unwrap_or("Unknown Track");
            build_activity_minimal(name, position, duration)
        }
    }
}

use std::sync::OnceLock;

static DISCORD_CLIENT: OnceLock<Arc<Mutex<DiscordIpcClient>>> = OnceLock::new();
static DISCORD_CONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAST_SEND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn get_or_init_client() -> Option<&'static Arc<Mutex<DiscordIpcClient>>> {
    DISCORD_CLIENT.get_or_init(|| Arc::new(Mutex::new(DiscordIpcClient::new(DISCORD_CLIENT_ID))));
    DISCORD_CLIENT.get()
}

fn check_debounce() -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let last = LAST_SEND.load(std::sync::atomic::Ordering::Relaxed);
    now.saturating_sub(last) >= MIN_UPDATE_INTERVAL.as_secs()
}

fn update_last_send() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    LAST_SEND.store(now, std::sync::atomic::Ordering::Relaxed);
}

/// Send an activity to Discord with 15-second debounce.
/// Handles connection, reconnection, and error logging internally.
pub async fn send_activity_oneshot(activity: discord_rich_presence::activity::Activity<'static>) {
    if !check_debounce() {
        tracing::debug!("Discord RPC: skipping update (cooldown)");
        return;
    }
    let client = match get_or_init_client() {
        Some(c) => c,
        None => return,
    };
    let client = Arc::clone(client);
    let needs_connect = !DISCORD_CONNECTED.load(std::sync::atomic::Ordering::Relaxed);
    let result = tokio::task::spawn_blocking(move || {
        let mut c = client.lock().unwrap();
        if needs_connect {
            match c.connect() {
                Ok(()) => {
                    DISCORD_CONNECTED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::warn!("Discord RPC connection failed: {}", e);
                    return false;
                }
            }
        }
        match c.set_activity(activity) {
            Ok(()) => {
                tracing::debug!("Discord RPC activity sent");
                true
            }
            Err(e) => {
                tracing::error!("Discord RPC set_activity failed: {}", e);
                DISCORD_CONNECTED.store(false, std::sync::atomic::Ordering::Relaxed);
                false
            }
        }
    })
    .await
    .unwrap_or(false);

    if result {
        update_last_send();
    }
}

/// Clear Discord presence (fire-and-forget).
pub async fn clear_activity_oneshot() {
    if !DISCORD_CONNECTED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some(client) = DISCORD_CLIENT.get() else {
        return;
    };
    let client = Arc::clone(client);
    let _ = tokio::task::spawn_blocking(move || {
        let mut c = client.lock().unwrap();
        let _ = c.clear_activity();
    })
    .await;
}
