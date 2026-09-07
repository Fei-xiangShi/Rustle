//! NCM service boundary.
//!
//! Rustle keeps stable domain models here while delegating all NetEase request
//! implementation to `ncm-api-rs`.

mod client;
mod error;
mod mapper;
mod models;

pub use client::NcmClient;
pub use error::{NcmError, NcmResult};
pub use models::{
    AlbumDetail, AlbumSummary, ArtistDetail, ArtistSummary, LoginInfo, NcmQualityLevel,
    PlaylistDetail, PlaylistSummary, RadioSummary, SearchType, SongQualityDetail,
    SongQualityOption, Track, TrackAvailability, TrackUrl, UserDetail, UserSummary, VideoSummary,
    VipInfo, VipTier,
};

/// SPlayer's canonical Private Radar playlist.
pub const PRIVATE_RADAR_PLAYLIST_ID: u64 = 3_136_952_023;
