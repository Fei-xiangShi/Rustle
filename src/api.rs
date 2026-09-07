//! Netease Cloud Music API module
//!
//! Provides NCM client with cookie management, QR login, and API wrappers.

mod ncm;

#[allow(unused_imports)]
pub use ncm::{
    AlbumDetail, AlbumSummary, ArtistDetail, ArtistSummary, LoginInfo, NcmClient, NcmError,
    NcmQualityLevel, NcmResult, PRIVATE_RADAR_PLAYLIST_ID, PlaylistDetail, PlaylistSummary,
    RadioSummary, SearchType, SongQualityDetail, SongQualityOption, Track, TrackAvailability,
    TrackUrl, UserDetail, UserSummary, VideoSummary, VipInfo, VipTier,
};
