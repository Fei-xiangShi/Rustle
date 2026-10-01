use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Lyrics {
    pub lyric: Vec<String>,
    pub tlyric: Vec<String>,
    /// NetEase word-level lyrics (YRC) returned by `/api/song/lyric/v1`.
    pub yrc: Vec<String>,
    /// Word-level translated lyrics, when available.
    pub ytlrc: Vec<String>,
    /// Romanized line lyrics.
    pub romalrc: Vec<String>,
    /// Word-level romanized lyrics.
    pub yromalrc: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ArtistSummary {
    pub id: u64,
    pub name: String,
    pub image_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserSummary {
    pub id: u64,
    pub nickname: String,
    pub avatar_url: String,
    #[serde(default)]
    pub vip: VipInfo,
}

/// Normalized NCM membership rights shared by account, profile and UI layers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct VipInfo {
    pub vip_type: i32,
    pub red_vip_level: u32,
    pub annual_count: u32,
    /// The Black Vinyl VIP badge returned by the membership endpoint.
    #[serde(default)]
    pub black_vinyl_icon_url: String,
    /// The SVIP/redplus badge returned by the membership endpoint.
    #[serde(default)]
    pub svip_icon_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum VipTier {
    None,
    BlackVinylVip,
    Svip,
}

impl VipInfo {
    pub fn tier(&self) -> VipTier {
        if self.vip_type >= 11 {
            VipTier::Svip
        } else if self.vip_type > 0 || self.red_vip_level > 0 {
            VipTier::BlackVinylVip
        } else {
            VipTier::None
        }
    }

    pub fn badge_url(&self) -> Option<&str> {
        let url: &str = match self.tier() {
            VipTier::None => "",
            VipTier::BlackVinylVip => self.black_vinyl_icon_url.as_str(),
            VipTier::Svip => self.svip_icon_url.as_str(),
        };
        (!url.is_empty()).then_some(url)
    }

    /// Remove all badge URLs when the authoritative membership projection is
    /// unavailable. Tier metadata remains useful, but no legacy image may be
    /// rendered as a fallback.
    pub fn without_badges(mut self) -> Self {
        self.black_vinyl_icon_url.clear();
        self.svip_icon_url.clear();
        self
    }
}

/// Compatibility name retained at the NCM adapter boundary.
pub type NcmQualityLevel = rustle_domain::audio::QualityLevel;

pub fn quality_api_level(level: NcmQualityLevel) -> &'static str {
    match level {
        NcmQualityLevel::Standard => "standard",
        NcmQualityLevel::Higher => "higher",
        NcmQualityLevel::ExHigh => "exhigh",
        NcmQualityLevel::Lossless => "lossless",
        NcmQualityLevel::HiRes => "hires",
        NcmQualityLevel::JvEffect => "jyeffect",
        NcmQualityLevel::Sky => "sky",
        NcmQualityLevel::Dolby => "dolby",
        NcmQualityLevel::JyMaster => "jymaster",
    }
}

/// Parse only the documented compact fields from the quality-detail API.
pub fn quality_from_api_field(value: &str) -> Option<NcmQualityLevel> {
    match value {
        "l" => Some(NcmQualityLevel::Standard),
        "m" => Some(NcmQualityLevel::Higher),
        "h" => Some(NcmQualityLevel::ExHigh),
        "sq" => Some(NcmQualityLevel::Lossless),
        "hr" => Some(NcmQualityLevel::HiRes),
        "je" => Some(NcmQualityLevel::JvEffect),
        "sk" => Some(NcmQualityLevel::Sky),
        "db" => Some(NcmQualityLevel::Dolby),
        "jm" => Some(NcmQualityLevel::JyMaster),
        _ => None,
    }
}

pub fn quality_from_api_level(value: &str) -> Option<NcmQualityLevel> {
    match value {
        "standard" => Some(NcmQualityLevel::Standard),
        "higher" => Some(NcmQualityLevel::Higher),
        "exhigh" => Some(NcmQualityLevel::ExHigh),
        "lossless" => Some(NcmQualityLevel::Lossless),
        "hires" => Some(NcmQualityLevel::HiRes),
        "jyeffect" => Some(NcmQualityLevel::JvEffect),
        "sky" => Some(NcmQualityLevel::Sky),
        "dolby" => Some(NcmQualityLevel::Dolby),
        "jymaster" => Some(NcmQualityLevel::JyMaster),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct SongQualityOption {
    pub level: NcmQualityLevel,
    pub bitrate: Option<u32>,
    pub size: Option<u64>,
    pub format: Option<String>,
    /// Source file identifier (`fid`) from `/song/music/detail/get`.
    pub file_id: Option<u64>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub channels: Option<u32>,
    /// Loudness/volume delta (`vd`) when the API provides it.
    pub volume_delta: Option<f64>,
    /// Spatial codec/effect marker (`it`) for enhanced levels.
    pub effect_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct SongQualityDetail {
    pub song_id: u64,
    pub options: Vec<SongQualityOption>,
    pub highest_available: Option<NcmQualityLevel>,
}

impl SongQualityDetail {
    pub fn best_for(&self, requested: NcmQualityLevel) -> Option<&SongQualityOption> {
        self.options.iter().find(|option| option.level == requested)
    }
}

impl VipTier {
    pub fn cache_discriminant(self) -> u8 {
        match self {
            Self::None => 0,
            Self::BlackVinylVip => 1,
            Self::Svip => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NcmQualityLevel, SongQualityDetail, SongQualityOption, VipInfo, VipTier,
        quality_from_api_level,
    };

    #[test]
    fn quality_level_parser_accepts_only_canonical_api_values() {
        assert_eq!(
            quality_from_api_level("exhigh"),
            Some(NcmQualityLevel::ExHigh)
        );
        assert_eq!(quality_from_api_level("hi-res"), None);
        assert_eq!(quality_from_api_level(" HIGH "), None);
        assert_eq!(quality_from_api_level("master"), None);
    }

    #[test]
    fn quality_detail_selection_is_exact_for_dolby_negotiation() {
        let detail = SongQualityDetail {
            song_id: 1,
            options: vec![SongQualityOption {
                level: NcmQualityLevel::ExHigh,
                ..Default::default()
            }],
            highest_available: Some(NcmQualityLevel::ExHigh),
        };
        assert!(detail.best_for(NcmQualityLevel::Lossless).is_none());
        assert_eq!(
            detail.best_for(NcmQualityLevel::ExHigh).unwrap().level,
            NcmQualityLevel::ExHigh
        );
    }

    #[test]
    fn vip_tiers_distinguish_black_vinyl_and_svip() {
        assert_eq!(
            VipInfo {
                vip_type: 10,
                ..Default::default()
            }
            .tier(),
            VipTier::BlackVinylVip
        );
        assert_eq!(
            VipInfo {
                vip_type: 11,
                ..Default::default()
            }
            .tier(),
            VipTier::Svip
        );
        assert_eq!(VipInfo::default().tier(), VipTier::None);
        assert_ne!(
            VipTier::BlackVinylVip.cache_discriminant(),
            VipTier::Svip.cache_discriminant()
        );
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct AlbumSummary {
    pub id: u64,
    pub name: String,
    pub image_url: String,
    pub artists: Vec<ArtistSummary>,
    pub publish_time: u64,
    pub tags: String,
}

impl AlbumSummary {
    pub fn primary_artist(&self) -> Option<&ArtistSummary> {
        self.artists.first()
    }

    pub fn artist_names(&self) -> String {
        joined_artist_names(&self.artists)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PlaylistSummary {
    pub id: u64,
    pub name: String,
    pub cover_url: String,
    pub creator: UserSummary,
    pub subscribed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct VideoSummary {
    pub id: u64,
    pub name: String,
    pub cover_url: String,
    pub artist_name: String,
    pub duration_ms: u64,
    pub play_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RadioSummary {
    pub id: u64,
    pub name: String,
    pub cover_url: String,
    pub creator: UserSummary,
    pub category: String,
    pub program_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum TrackAvailability {
    Free,
    VipOnly,
    Payment,
    VipOnlyHighRate,
    Unavailable,
    Unknown,
}

impl TrackAvailability {
    pub fn from_fee(fee: i32) -> Self {
        match fee {
            0 => Self::Free,
            1 => Self::VipOnly,
            4 => Self::Payment,
            8 => Self::VipOnlyHighRate,
            _ => Self::Unknown,
        }
    }

    pub fn from_privilege(st: i32, fee: i32) -> Self {
        if st < 0 {
            Self::Unavailable
        } else {
            Self::from_fee(fee)
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Free => "免费",
            Self::VipOnly => "VIP",
            Self::Payment => "EP/购买",
            Self::VipOnlyHighRate => "高音质 VIP",
            Self::Unavailable => "不可用",
            Self::Unknown => "",
        }
    }

    pub fn is_restricted(&self) -> bool {
        matches!(
            self,
            Self::VipOnly | Self::Payment | Self::VipOnlyHighRate | Self::Unavailable
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Track {
    pub id: u64,
    pub title: String,
    pub artists: Vec<ArtistSummary>,
    pub album: AlbumSummary,
    pub duration_ms: u64,
    pub availability: TrackAvailability,
    pub track_number: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    #[serde(default)]
    pub quality_options: Vec<SongQualityOption>,
}

impl Track {
    pub fn primary_artist(&self) -> Option<&ArtistSummary> {
        self.artists.first()
    }

    pub fn artist_names(&self) -> String {
        joined_artist_names(&self.artists)
    }

    pub fn cover_url(&self) -> &str {
        self.album.image_url.as_str()
    }
}

impl PartialEq for Track {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: 0,
            title: String::new(),
            artists: Vec::new(),
            album: AlbumSummary::default(),
            duration_ms: 0,
            availability: TrackAvailability::Unknown,
            track_number: None,
            year: None,
            genre: None,
            quality_options: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistDetail {
    pub id: u64,
    pub name: String,
    pub cover_url: String,
    pub description: String,
    pub create_time: u64,
    pub track_update_time: u64,
    pub creator: UserSummary,
    pub track_count: u64,
    pub subscribed: bool,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtistDetail {
    pub id: u64,
    pub name: String,
    pub image_url: String,
    pub description: String,
    pub track_count: u32,
    pub album_count: u32,
    pub mv_count: u32,
    pub followed: bool,
    pub top_tracks: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlbumDetail {
    pub id: u64,
    pub name: String,
    pub image_url: String,
    pub description: String,
    pub artists: Vec<ArtistSummary>,
    pub track_count: u32,
    pub publish_time: u64,
    pub tracks: Vec<Track>,
    pub tags: String,
}

impl AlbumDetail {
    pub fn primary_artist(&self) -> Option<&ArtistSummary> {
        self.artists.first()
    }

    pub fn artist_names(&self) -> String {
        joined_artist_names(&self.artists)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserDetail {
    pub user_id: u64,
    pub artist_id: u64,
    pub nickname: String,
    pub artist_name: String,
    pub signature: String,
    pub follows: u64,
    pub followeds: u64,
    pub avatar_url: String,
    pub background_url: String,
    #[serde(default)]
    pub vip: VipInfo,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackUrl {
    pub id: u64,
    pub url: String,
    pub requested_level: NcmQualityLevel,
    pub level: NcmQualityLevel,
    pub rate: u32,
    pub size: Option<u64>,
    pub format: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub immerse_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LikeProtocol {
    LegacyWeapi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikeResult {
    pub liked: bool,
    pub playlist_id: Option<u64>,
    pub protocol: LikeProtocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrobbleResult {
    pub startplay_confirmed: bool,
    pub play_confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistTrackOperation {
    Add,
    Delete,
}

impl PlaylistTrackOperation {
    pub(super) const fn as_api_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Delete => "del",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistTrackMutation {
    pub operation: PlaylistTrackOperation,
    pub playlist_id: u64,
    pub requested_track_ids: Vec<u64>,
    pub returned_track_ids: Vec<u64>,
    pub changed_count: u64,
    pub cloud_count: Option<u64>,
    pub retried_after_code_512: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Msg {
    pub code: i32,
    pub msg: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LoginInfo {
    pub code: i32,
    pub user_id: u64,
    pub nickname: String,
    pub avatar_url: String,
    pub vip_type: i32,
    #[serde(default)]
    pub vip: VipInfo,
    pub msg: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    Songs,
    Albums,
    Artists,
    Playlists,
    Videos,
    Radios,
}

impl SearchType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchType::Songs => "1",
            SearchType::Albums => "10",
            SearchType::Artists => "100",
            SearchType::Playlists => "1000",
            SearchType::Videos => "1004",
            SearchType::Radios => "1009",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchResponse {
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
    pub playlists: Vec<PlaylistSummary>,
    pub videos: Vec<VideoSummary>,
    pub radios: Vec<RadioSummary>,
    pub track_count: u32,
    pub album_count: u32,
    pub artist_count: u32,
    pub playlist_count: u32,
    pub video_count: u32,
    pub radio_count: u32,
}

fn joined_artist_names(artists: &[ArtistSummary]) -> String {
    let names = artists
        .iter()
        .filter_map(|artist| {
            let name = artist.name.trim();
            (!name.is_empty()).then_some(name)
        })
        .collect::<Vec<_>>();

    if names.is_empty() {
        "unknown".to_string()
    } else {
        names.join(" / ")
    }
}
