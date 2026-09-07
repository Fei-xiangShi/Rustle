//! Root compatibility facade for the physical NCM adapter crate.

pub(crate) use rustle_ncm::quality_api_level;
#[allow(unused_imports)]
pub use rustle_ncm::{
    AlbumDetail, AlbumSummary, ArtistDetail, ArtistSummary, LoginInfo, NcmClient, NcmError,
    NcmQualityLevel, NcmResult, PRIVATE_RADAR_PLAYLIST_ID, PlaylistDetail, PlaylistSummary,
    RadioSummary, SearchType, SongQualityDetail, SongQualityOption, Track, TrackAvailability,
    TrackUrl, UserDetail, UserSummary, VideoSummary, VipInfo, VipTier,
};
