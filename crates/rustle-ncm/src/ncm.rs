//! NCM service boundary.
//!
//! Rustle keeps stable domain models here while delegating all NetEase request
//! implementation to `ncm-api-rs`.

mod client;
mod error;
mod mapper;
mod models;
mod session;

pub use models::quality_api_level;

pub use client::NcmClient;
pub use error::{NcmError, NcmResult};
pub use models::{
    AlbumDetail, AlbumSummary, ArtistDetail, ArtistSummary, LikeProtocol, LikeResult, LoginInfo,
    NcmQualityLevel, PlaylistDetail, PlaylistSummary, PlaylistTrackMutation,
    PlaylistTrackOperation, RadioSummary, ScrobbleResult, SearchType, SongQualityDetail,
    SongQualityOption, Track, TrackAvailability, TrackUrl, UserDetail, UserSummary, VideoSummary,
    VipInfo, VipTier,
};

/// SPlayer's canonical Private Radar playlist.
pub const PRIVATE_RADAR_PLAYLIST_ID: u64 = 3_136_952_023;
