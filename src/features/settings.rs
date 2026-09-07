//! Application settings persistence
//!
//! Handles saving and loading user preferences.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use crate::domain::playback::PlayMode;

use super::KeyBindings;

/// Close behavior when clicking the X button
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    /// Always ask what to do
    #[default]
    Ask,
    /// Close the application
    Exit,
    /// Minimize to system tray
    MinimizeToTray,
}

impl std::fmt::Display for CloseBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloseBehavior::Ask => write!(f, "询问"),
            CloseBehavior::Exit => write!(f, "退出"),
            CloseBehavior::MinimizeToTray => write!(f, "最小化到托盘"),
        }
    }
}

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// What to do when closing the window
    pub close_behavior: CloseBehavior,
    /// Volume level (0.0 to 1.0)
    pub volume: f32,
    /// Play mode (sequential, shuffle, repeat one, repeat all)
    pub play_mode: PlayMode,
    /// Custom keybindings
    pub keybindings: KeyBindings,
    /// Playback settings
    pub playback: PlaybackSettings,
    /// Display and interface settings
    pub display: DisplaySettings,
    /// Lyrics panel settings
    pub lyrics: LyricsSettings,
    /// Storage settings
    pub storage: StorageSettings,
    /// System settings
    pub system: SystemSettings,
    /// Network settings
    #[serde(default)]
    pub network: NetworkSettings,
}

/// Playback-related settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackSettings {
    /// Enable fade in/out when playing/pausing
    pub fade_in_out: bool,
    /// Enable volume normalization
    pub volume_normalization: bool,
    /// Enable equalizer
    pub equalizer_enabled: bool,
    /// Equalizer preset
    #[serde(default)]
    pub equalizer_preset: EqualizerPreset,
    /// Equalizer preset or custom values
    pub equalizer_values: [f32; 10], // 10-band equalizer
    /// Preamp gain in dB (-12 to +12)
    #[serde(default)]
    pub equalizer_preamp: f32,
    /// Spectrum analyzer decay (0.0 = instant, 0.95 = slow)
    #[serde(default = "default_spectrum_decay")]
    pub spectrum_decay: f32,
    /// Spectrum display mode (true = bars, false = line)
    #[serde(default = "default_true")]
    pub spectrum_bars_mode: bool,
    /// Music quality setting (0=128k, 1=192k, 2=320k, 3=SQ, 4=Hi-Res)
    #[serde(default = "default_music_quality")]
    pub music_quality: MusicQuality,
    /// Enable native Automix planning and scheduling. Disabled by default.
    #[serde(default)]
    pub automix_enabled: bool,
    /// Maximum seconds of audio inspected by the offline Automix analyzer.
    #[serde(default = "default_automix_analysis_max_seconds")]
    pub automix_analysis_max_seconds: u32,
}

fn default_music_quality() -> MusicQuality {
    MusicQuality::High // 320k as default
}

fn default_spectrum_decay() -> f32 {
    0.85
}

fn default_true() -> bool {
    true
}

fn default_automix_analysis_max_seconds() -> u32 {
    crate::audio::automix::DEFAULT_ANALYSIS_MAX_SECONDS
}

/// Music quality options for streaming
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MusicQuality {
    /// Standard quality (128kbps)
    Standard,
    /// Higher quality (192kbps)
    Higher,
    /// High quality (320kbps) - default
    #[default]
    High,
    /// Lossless quality (SQ/FLAC ~999kbps)
    Lossless,
    /// Hi-Res quality (~1900kbps)
    HiRes,
    /// NCM HD enhanced sound.
    JvEffect,
    /// NCM immersive surround sound.
    Sky,
    /// Dolby Atmos.
    Dolby,
    /// NCM master quality.
    JyMaster,
}

impl MusicQuality {
    /// Get all quality options
    pub fn all() -> Vec<Self> {
        vec![
            Self::Standard,
            Self::Higher,
            Self::High,
            Self::Lossless,
            Self::HiRes,
            Self::JvEffect,
            Self::Sky,
            Self::Dolby,
            Self::JyMaster,
        ]
    }

    /// Get the API rate value for this quality
    pub fn to_api_rate(self) -> u32 {
        match self {
            Self::Standard => 0, // 128000
            Self::Higher => 1,   // 192000
            Self::High => 2,     // 320000
            Self::Lossless => 3, // 999000 (SQ)
            Self::HiRes => 4,    // 1900000 (Hi-Res)
            Self::JvEffect => 5,
            Self::Sky => 6,
            Self::Dolby => 7,
            Self::JyMaster => 8,
        }
    }

    /// Get display name for this quality
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Standard => "128kbps",
            Self::Higher => "192kbps",
            Self::High => "320kbps",
            Self::Lossless => "SQ (无损)",
            Self::HiRes => "Hi-Res",
            Self::JvEffect => "高清臻音",
            Self::Sky => "沉浸环绕声",
            Self::Dolby => "杜比全景声",
            Self::JyMaster => "超清母带",
        }
    }

    pub fn from_display_name(value: &str) -> Option<Self> {
        match value {
            "128kbps" | "Standard" => Some(Self::Standard),
            "192kbps" | "Higher" => Some(Self::Higher),
            "320kbps" | "High" | "ExHigh" => Some(Self::High),
            "SQ (无损)" | "Lossless" => Some(Self::Lossless),
            "Hi-Res" | "HiRes" => Some(Self::HiRes),
            "高清臻音" | "JvEffect" | "JyEffect" => Some(Self::JvEffect),
            "沉浸环绕声" | "Sky" => Some(Self::Sky),
            "杜比全景声" | "Dolby" => Some(Self::Dolby),
            "超清母带" | "JyMaster" => Some(Self::JyMaster),
            _ => None,
        }
    }
}

/// Equalizer presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EqualizerPreset {
    /// Flat (no adjustment)
    #[default]
    Flat,
    /// Pop music
    Pop,
    /// Rock music
    Rock,
    /// Jazz music
    Jazz,
    /// Classical music
    Classical,
    /// Electronic/Dance music
    Electronic,
    /// Hip-Hop/R&B
    HipHop,
    /// Acoustic/Folk
    Acoustic,
    /// Bass boost
    BassBoost,
    /// Treble boost
    TrebleBoost,
    /// Vocal enhancement
    Vocal,
    /// Custom (user-defined)
    Custom,
}

impl EqualizerPreset {
    /// Get all presets
    pub fn all() -> &'static [EqualizerPreset] {
        &[
            EqualizerPreset::Flat,
            EqualizerPreset::Pop,
            EqualizerPreset::Rock,
            EqualizerPreset::Jazz,
            EqualizerPreset::Classical,
            EqualizerPreset::Electronic,
            EqualizerPreset::HipHop,
            EqualizerPreset::Acoustic,
            EqualizerPreset::BassBoost,
            EqualizerPreset::TrebleBoost,
            EqualizerPreset::Vocal,
            EqualizerPreset::Custom,
        ]
    }

    /// Get display name for the preset
    pub fn display_name(&self) -> &'static str {
        match self {
            EqualizerPreset::Flat => "平坦",
            EqualizerPreset::Pop => "流行",
            EqualizerPreset::Rock => "摇滚",
            EqualizerPreset::Jazz => "爵士",
            EqualizerPreset::Classical => "古典",
            EqualizerPreset::Electronic => "电子",
            EqualizerPreset::HipHop => "嘻哈",
            EqualizerPreset::Acoustic => "原声",
            EqualizerPreset::BassBoost => "低音增强",
            EqualizerPreset::TrebleBoost => "高音增强",
            EqualizerPreset::Vocal => "人声",
            EqualizerPreset::Custom => "自定义",
        }
    }

    /// Get equalizer values for this preset
    /// Returns [31Hz, 62Hz, 125Hz, 250Hz, 500Hz, 1kHz, 2kHz, 4kHz, 8kHz, 16kHz]
    pub fn values(&self) -> [f32; 10] {
        match self {
            EqualizerPreset::Flat => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            EqualizerPreset::Pop => [-1.0, -1.0, 0.0, 2.0, 4.0, 4.0, 2.0, 0.0, -1.0, -1.0],
            EqualizerPreset::Rock => [4.0, 3.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0],
            EqualizerPreset::Jazz => [3.0, 2.0, 1.0, 2.0, -2.0, -2.0, 0.0, 1.0, 2.0, 3.0],
            EqualizerPreset::Classical => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0, -3.0, -3.0, -4.0],
            EqualizerPreset::Electronic => [4.0, 3.0, 1.0, 0.0, -2.0, 2.0, 1.0, 2.0, 4.0, 4.0],
            EqualizerPreset::HipHop => [5.0, 4.0, 2.0, 3.0, -1.0, -1.0, 2.0, -1.0, 2.0, 3.0],
            EqualizerPreset::Acoustic => [4.0, 3.0, 2.0, 1.0, 2.0, 2.0, 3.0, 3.0, 3.0, 2.0],
            EqualizerPreset::BassBoost => [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            EqualizerPreset::TrebleBoost => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 5.0, 6.0],
            EqualizerPreset::Vocal => [-2.0, -2.0, 0.0, 3.0, 5.0, 5.0, 4.0, 2.0, 0.0, -2.0],
            EqualizerPreset::Custom => [0.0; 10], // Custom uses stored values
        }
    }
}

impl std::fmt::Display for EqualizerPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Display and interface settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplaySettings {
    /// Dark mode enabled
    pub dark_mode: bool,
    /// Application language
    pub language: String,
    /// Power saving mode - disables animations and uses simple rendering
    #[serde(default)]
    pub power_saving_mode: bool,
}

// Lyrics panel settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LyricsSettings {
    pub lyrics_font_family: Option<String>,
}

/// Storage settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSettings {
    /// Maximum cache size in MB
    pub max_cache_mb: u64,
    /// Cache directory path
    pub cache_dir: Option<String>,
    /// Download directory path (None = use system Music folder)
    #[serde(default)]
    pub download_dir: Option<String>,
    /// Download audio quality
    #[serde(default = "default_download_quality")]
    pub download_quality: MusicQuality,
}

fn default_download_quality() -> MusicQuality {
    MusicQuality::High
}

/// System settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSettings {
    /// Selected audio output device
    pub audio_output_device: Option<String>,
    /// Audio buffer size in samples
    pub audio_buffer_size: u32,
    /// Enable Discord Rich Presence integration
    #[serde(default = "default_true")]
    pub discord_enabled: bool,
}

/// Proxy type for network settings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyType {
    /// No proxy
    #[default]
    None,
    /// HTTP proxy
    Http,
    /// HTTPS proxy
    Https,
    /// SOCKS5 proxy
    Socks5,
    /// Use system proxy settings
    System,
}

impl std::fmt::Display for ProxyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyType::None => write!(f, "无代理"),
            ProxyType::Http => write!(f, "HTTP"),
            ProxyType::Https => write!(f, "HTTPS"),
            ProxyType::Socks5 => write!(f, "SOCKS5"),
            ProxyType::System => write!(f, "系统代理"),
        }
    }
}

/// Network settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSettings {
    /// Proxy type
    pub proxy_type: ProxyType,
    /// Proxy host address
    pub proxy_host: String,
    /// Proxy port
    pub proxy_port: u16,
    /// Proxy username (optional)
    pub proxy_username: Option<String>,
    /// Proxy password (optional)
    pub proxy_password: Option<String>,
}

impl NetworkSettings {
    /// Build proxy URL string from settings
    /// Returns None if proxy is disabled or invalid
    pub fn proxy_url(&self) -> Option<String> {
        match self.proxy_type {
            ProxyType::None => None,
            ProxyType::System => {
                // Try to get system proxy from environment
                std::env::var("HTTP_PROXY")
                    .or_else(|_| std::env::var("http_proxy"))
                    .or_else(|_| std::env::var("HTTPS_PROXY"))
                    .or_else(|_| std::env::var("https_proxy"))
                    .ok()
            }
            ProxyType::Http | ProxyType::Https | ProxyType::Socks5 => {
                if self.proxy_host.is_empty() || self.proxy_port == 0 {
                    return None;
                }

                let scheme = match self.proxy_type {
                    ProxyType::Http => "http",
                    ProxyType::Https => "https",
                    ProxyType::Socks5 => "socks5",
                    _ => unreachable!(),
                };

                // Build URL with optional auth
                let auth = match (&self.proxy_username, &self.proxy_password) {
                    (Some(user), Some(pass)) if !user.is_empty() => {
                        format!("{}:{}@", user, pass)
                    }
                    (Some(user), None) if !user.is_empty() => {
                        format!("{}@", user)
                    }
                    _ => String::new(),
                };

                Some(format!(
                    "{}://{}{}:{}",
                    scheme, auth, self.proxy_host, self.proxy_port
                ))
            }
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            close_behavior: CloseBehavior::Ask,
            volume: 1.0,
            play_mode: PlayMode::Sequential,
            keybindings: KeyBindings::default(),
            playback: PlaybackSettings::default(),
            display: DisplaySettings::default(),
            lyrics: LyricsSettings::default(),
            storage: StorageSettings::default(),
            system: SystemSettings::default(),
            network: NetworkSettings::default(),
        }
    }
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            fade_in_out: false,
            volume_normalization: false,
            equalizer_enabled: false,
            equalizer_preset: EqualizerPreset::Flat,
            equalizer_values: [0.0; 10], // Flat EQ
            equalizer_preamp: 0.0,
            spectrum_decay: 0.85,
            spectrum_bars_mode: true,
            music_quality: MusicQuality::High, // 320k default
            automix_enabled: false,
            automix_analysis_max_seconds: default_automix_analysis_max_seconds(),
        }
    }
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            language: "zh".to_string(),
            power_saving_mode: false,
        }
    }
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            max_cache_mb: 1024, // 1GB default
            cache_dir: None,
            download_dir: None,
            download_quality: MusicQuality::High,
        }
    }
}

impl StorageSettings {
    /// Get the effective download directory, resolving defaults
    pub fn effective_download_dir(&self) -> PathBuf {
        if let Some(ref dir) = self.download_dir {
            let path = PathBuf::from(dir);
            if path.exists() {
                return path;
            }
            // Create if parent exists (path is valid)
            if path.parent().map(|p| p.exists()).unwrap_or(false) {
                let _ = std::fs::create_dir_all(&path);
                return path;
            }
        }
        // Default: system Music folder / Rustle
        let default_dir = directories::UserDirs::new()
            .and_then(|dirs| dirs.audio_dir().map(|p| p.join("Rustle")))
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join("Music").join("Rustle"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&default_dir);
        default_dir
    }
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self {
            audio_output_device: None,
            audio_buffer_size: 512,
            discord_enabled: true,
        }
    }
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            proxy_type: ProxyType::None,
            proxy_host: String::new(),
            proxy_port: 0,
            proxy_username: None,
            proxy_password: None,
        }
    }
}

impl Settings {
    /// Get the settings file path
    pub fn file_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "rustle", "Rustle")
            .map(|dirs| dirs.config_dir().join("settings.json"))
    }

    /// Load settings from file, or return defaults if not found
    pub fn load() -> Self {
        Self::file_path()
            .and_then(|path| Self::load_from_file(&path).ok())
            .unwrap_or_default()
    }

    /// Load settings from a specific file
    pub fn load_from_file(path: &Path) -> Result<Self, SettingsError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| SettingsError::Io(e.to_string()))?;
        serde_json::from_str(&content).map_err(|e| SettingsError::Parse(e.to_string()))
    }

    /// Save settings to the default file
    pub fn save(&self) -> Result<(), SettingsError> {
        if let Some(path) = Self::file_path() {
            self.save_to_file(&path)
        } else {
            Err(SettingsError::Io(
                "Could not determine config directory".to_string(),
            ))
        }
    }

    /// Save settings to a specific file
    pub fn save_to_file(&self, path: &Path) -> Result<(), SettingsError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SettingsError::Io(e.to_string()))?;
        }

        let content =
            serde_json::to_string_pretty(self).map_err(|e| SettingsError::Parse(e.to_string()))?;
        std::fs::write(path, content).map_err(|e| SettingsError::Io(e.to_string()))?;
        Ok(())
    }
}

/// Errors that can occur with settings
#[derive(Debug, Clone)]
pub enum SettingsError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Io(e) => write!(f, "IO error: {}", e),
            SettingsError::Parse(e) => write!(f, "Parse error: {}", e),
        }
    }
}

impl std::error::Error for SettingsError {}
