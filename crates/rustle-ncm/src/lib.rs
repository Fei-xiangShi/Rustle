//! Netease Cloud Music API module
//!
//! Provides NCM client with cookie management, QR login, and API wrappers.

mod ncm;

pub use ncm::quality_api_level;
#[allow(unused_imports)]
pub use ncm::{
    AlbumDetail, AlbumSummary, ArtistDetail, ArtistSummary, LikeProtocol, LikeResult, LoginInfo,
    NcmClient, NcmError, NcmQualityLevel, NcmResult, PRIVATE_RADAR_PLAYLIST_ID, PlaylistDetail,
    PlaylistSummary, PlaylistTrackMutation, PlaylistTrackOperation, RadioSummary, ScrobbleResult,
    SearchType, SongQualityDetail, SongQualityOption, Track, TrackAvailability, TrackUrl,
    UserDetail, UserSummary, VideoSummary, VipInfo, VipTier,
};
