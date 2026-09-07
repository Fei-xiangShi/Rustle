//! URL scheme parser for rustle:// protocol
//!
//! Parses rustle:// URIs into typed ProtocolAction values
//! for routing to the appropriate application behavior.

use std::fmt;

/// Playback control commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackCmd {
    Play,
    Pause,
    Next,
    Previous,
    Toggle,
}

/// Target for playlist navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistTarget {
    Local(i64),
    Ncm(u64),
}

/// Actions that can be triggered via rustle:// URIs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolAction {
    /// Navigate to song detail page
    NavigateToSong(u64),
    /// Navigate to a playlist (local or NCM)
    NavigateToPlaylist(PlaylistTarget),
    /// Navigate to artist page
    NavigateToArtist(u64),
    /// Navigate to album page
    NavigateToAlbum(u64),
    /// Search for a query
    Search(String),
    /// Resolve and play a specific song
    PlaySong(u64),
    /// Control playback
    PlaybackControl(PlaybackCmd),
}

/// Errors that can occur when parsing a rustle:// URI
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// URI is empty
    EmptyUri,
    /// URI exceeds maximum length (2048 bytes)
    TooLong(usize),
    /// URI does not start with rustle://
    MissingScheme,
    /// Unknown action in the URI path
    UnknownAction(String),
    /// ID parameter is not a valid number
    InvalidId(String),
    /// Required parameter is missing
    MissingParameter(String),
    /// Parameter has an invalid value
    InvalidParameter(String, String),
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUri => write!(f, "empty URI"),
            Self::TooLong(len) => write!(f, "URI too long ({} bytes, max 2048)", len),
            Self::MissingScheme => write!(f, "URI must start with rustle://"),
            Self::UnknownAction(action) => write!(f, "unknown action: {}", action),
            Self::InvalidId(ctx) => write!(f, "invalid ID in {}", ctx),
            Self::MissingParameter(name) => write!(f, "missing parameter: {}", name),
            Self::InvalidParameter(name, value) => {
                write!(f, "invalid value for {}: {}", name, value)
            }
        }
    }
}

impl std::error::Error for UriError {}

/// Maximum URI length in bytes
pub const MAX_URI_LENGTH: usize = 2048;

/// URI scheme name
pub const SCHEME: &str = "rustle";

/// URI scheme name for development builds
pub const SCHEME_DEV: &str = "rustle-dev";

fn strip_scheme<'a>(uri: &'a str, scheme: &str) -> Option<&'a str> {
    uri.strip_prefix(scheme)?.strip_prefix("://")
}

/// Parse a rustle:// URI into a typed ProtocolAction.
///
/// Supported URI formats:
/// - `rustle://song/{ncm_id}[?action=play]`
/// - `rustle://playlist/local/{db_id}`
/// - `rustle://playlist/ncm/{ncm_id}`
/// - `rustle://artist/{ncm_id}`
/// - `rustle://album/{ncm_id}`
/// - `rustle://search?q={keyword}`
/// - `rustle://play`
/// - `rustle://pause`
/// - `rustle://next`
/// - `rustle://previous`
/// - `rustle://toggle`
pub fn parse_rustle_uri(uri: &str) -> Result<ProtocolAction, UriError> {
    if uri.is_empty() {
        return Err(UriError::EmptyUri);
    }

    if uri.len() > MAX_URI_LENGTH {
        return Err(UriError::TooLong(uri.len()));
    }

    // Strip scheme
    let content = if cfg!(debug_assertions) {
        strip_scheme(uri, SCHEME_DEV).or_else(|| strip_scheme(uri, SCHEME))
    } else {
        strip_scheme(uri, SCHEME)
    }
    .ok_or(UriError::MissingScheme)?;

    // Split path and query
    let (path, query) = match content.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (content, None),
    };

    // Parse path segments
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if segments.is_empty() {
        return Err(UriError::UnknownAction("(empty path)".to_string()));
    }

    match segments[0] {
        "song" => parse_song_action(&segments[1..], query),
        "playlist" => parse_playlist_action(&segments[1..]),
        "artist" => parse_artist_action(&segments[1..]),
        "album" => parse_album_action(&segments[1..]),
        "search" => parse_search_action(query),
        "play" => Ok(ProtocolAction::PlaybackControl(PlaybackCmd::Play)),
        "pause" => Ok(ProtocolAction::PlaybackControl(PlaybackCmd::Pause)),
        "next" => Ok(ProtocolAction::PlaybackControl(PlaybackCmd::Next)),
        "previous" => Ok(ProtocolAction::PlaybackControl(PlaybackCmd::Previous)),
        "toggle" => Ok(ProtocolAction::PlaybackControl(PlaybackCmd::Toggle)),
        other => Err(UriError::UnknownAction(other.to_string())),
    }
}

fn parse_song_action(segments: &[&str], query: Option<&str>) -> Result<ProtocolAction, UriError> {
    let id = parse_u64_id(segments.first().copied(), "song")?;
    let action = query.and_then(|q| {
        q.split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == "action")
            .map(|(_, v)| v)
    });

    match action {
        Some("play") => Ok(ProtocolAction::PlaySong(id)),
        Some(value) => Err(UriError::InvalidParameter(
            "action".to_string(),
            value.to_string(),
        )),
        None => Ok(ProtocolAction::NavigateToSong(id)),
    }
}

fn parse_playlist_action(segments: &[&str]) -> Result<ProtocolAction, UriError> {
    match segments.first().copied() {
        Some("local") => {
            let id: i64 = segments
                .get(1)
                .copied()
                .ok_or(UriError::MissingParameter("playlist local ID".to_string()))
                .and_then(|s| {
                    s.parse()
                        .map_err(|_| UriError::InvalidId("local playlist".to_string()))
                })?;
            if id <= 0 {
                return Err(UriError::InvalidId(
                    "local playlist ID must be positive".to_string(),
                ));
            }
            Ok(ProtocolAction::NavigateToPlaylist(PlaylistTarget::Local(
                id,
            )))
        }
        Some("ncm") => {
            let id = parse_u64_id(segments.get(1).copied(), "NCM playlist")?;
            Ok(ProtocolAction::NavigateToPlaylist(PlaylistTarget::Ncm(id)))
        }
        Some(other) => Err(UriError::UnknownAction(format!("playlist/{}", other))),
        None => Err(UriError::MissingParameter(
            "playlist type (local/ncm)".to_string(),
        )),
    }
}

fn parse_artist_action(segments: &[&str]) -> Result<ProtocolAction, UriError> {
    let id = parse_u64_id(segments.first().copied(), "artist")?;
    Ok(ProtocolAction::NavigateToArtist(id))
}

fn parse_album_action(segments: &[&str]) -> Result<ProtocolAction, UriError> {
    let id = parse_u64_id(segments.first().copied(), "album")?;
    Ok(ProtocolAction::NavigateToAlbum(id))
}

fn parse_search_action(query: Option<&str>) -> Result<ProtocolAction, UriError> {
    let q = query
        .and_then(|q| {
            q.split('&')
                .filter_map(|p| p.split_once('='))
                .find(|(k, _)| *k == "q")
                .map(|(_, v)| v.to_string())
        })
        .filter(|v| !v.is_empty())
        .ok_or(UriError::MissingParameter("q".to_string()))?;
    Ok(ProtocolAction::Search(q))
}

/// Parse a u64 ID from a string segment, with context for error messages.
fn parse_u64_id(segment: Option<&str>, context: &str) -> Result<u64, UriError> {
    let s = segment.ok_or_else(|| UriError::MissingParameter(context.to_string()))?;
    let id: u64 = s
        .parse()
        .map_err(|_| UriError::InvalidId(context.to_string()))?;
    if id == 0 {
        return Err(UriError::InvalidId(context.to_string()));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ Valid URIs ============

    #[test]
    fn test_song_navigate() {
        let result = parse_rustle_uri("rustle://song/123456").unwrap();
        assert_eq!(result, ProtocolAction::NavigateToSong(123456));
    }

    #[test]
    fn test_song_play() {
        let result = parse_rustle_uri("rustle://song/123456?action=play").unwrap();
        assert_eq!(result, ProtocolAction::PlaySong(123456));
    }

    #[test]
    fn test_playlist_local() {
        let result = parse_rustle_uri("rustle://playlist/local/42").unwrap();
        assert_eq!(
            result,
            ProtocolAction::NavigateToPlaylist(PlaylistTarget::Local(42))
        );
    }

    #[test]
    fn test_playlist_ncm() {
        let result = parse_rustle_uri("rustle://playlist/ncm/789012").unwrap();
        assert_eq!(
            result,
            ProtocolAction::NavigateToPlaylist(PlaylistTarget::Ncm(789012))
        );
    }

    #[test]
    fn test_artist() {
        let result = parse_rustle_uri("rustle://artist/111").unwrap();
        assert_eq!(result, ProtocolAction::NavigateToArtist(111));
    }

    #[test]
    fn test_album() {
        let result = parse_rustle_uri("rustle://album/222").unwrap();
        assert_eq!(result, ProtocolAction::NavigateToAlbum(222));
    }

    #[test]
    fn test_search() {
        let result = parse_rustle_uri("rustle://search?q=周杰伦").unwrap();
        assert_eq!(result, ProtocolAction::Search("周杰伦".to_string()));
    }

    #[test]
    fn test_playback_play() {
        let result = parse_rustle_uri("rustle://play").unwrap();
        assert_eq!(result, ProtocolAction::PlaybackControl(PlaybackCmd::Play));
    }

    #[test]
    fn test_playback_pause() {
        let result = parse_rustle_uri("rustle://pause").unwrap();
        assert_eq!(result, ProtocolAction::PlaybackControl(PlaybackCmd::Pause));
    }

    #[test]
    fn test_playback_next() {
        let result = parse_rustle_uri("rustle://next").unwrap();
        assert_eq!(result, ProtocolAction::PlaybackControl(PlaybackCmd::Next));
    }

    #[test]
    fn test_playback_previous() {
        let result = parse_rustle_uri("rustle://previous").unwrap();
        assert_eq!(
            result,
            ProtocolAction::PlaybackControl(PlaybackCmd::Previous)
        );
    }

    #[test]
    fn test_playback_toggle() {
        let result = parse_rustle_uri("rustle://toggle").unwrap();
        assert_eq!(result, ProtocolAction::PlaybackControl(PlaybackCmd::Toggle));
    }

    #[test]
    fn test_dev_scheme() {
        let result = parse_rustle_uri("rustle-dev://play").unwrap();
        assert_eq!(result, ProtocolAction::PlaybackControl(PlaybackCmd::Play));
    }

    // ============ Invalid URIs ============

    #[test]
    fn test_empty_uri() {
        assert_eq!(parse_rustle_uri(""), Err(UriError::EmptyUri));
    }

    #[test]
    fn test_missing_scheme() {
        assert_eq!(
            parse_rustle_uri("https://example.com"),
            Err(UriError::MissingScheme)
        );
    }

    #[test]
    fn test_unknown_action() {
        assert_eq!(
            parse_rustle_uri("rustle://unknown/thing"),
            Err(UriError::UnknownAction("unknown".to_string()))
        );
    }

    #[test]
    fn test_invalid_song_id() {
        assert_eq!(
            parse_rustle_uri("rustle://song/abc"),
            Err(UriError::InvalidId("song".to_string()))
        );
    }

    #[test]
    fn test_zero_song_id() {
        assert_eq!(
            parse_rustle_uri("rustle://song/0"),
            Err(UriError::InvalidId("song".to_string()))
        );
    }

    #[test]
    fn test_negative_playlist_id() {
        assert_eq!(
            parse_rustle_uri("rustle://playlist/local/-1"),
            Err(UriError::InvalidId(
                "local playlist ID must be positive".to_string()
            ))
        );
    }

    #[test]
    fn test_missing_artist_id() {
        assert_eq!(
            parse_rustle_uri("rustle://artist"),
            Err(UriError::MissingParameter("artist".to_string()))
        );
    }

    #[test]
    fn test_missing_album_id() {
        assert_eq!(
            parse_rustle_uri("rustle://album"),
            Err(UriError::MissingParameter("album".to_string()))
        );
    }

    #[test]
    fn test_search_no_query() {
        assert_eq!(
            parse_rustle_uri("rustle://search"),
            Err(UriError::MissingParameter("q".to_string()))
        );
    }

    #[test]
    fn test_search_empty_query() {
        assert_eq!(
            parse_rustle_uri("rustle://search?q="),
            Err(UriError::MissingParameter("q".to_string()))
        );
    }

    #[test]
    fn test_missing_playlist_type() {
        assert_eq!(
            parse_rustle_uri("rustle://playlist"),
            Err(UriError::MissingParameter(
                "playlist type (local/ncm)".to_string()
            ))
        );
    }

    #[test]
    fn test_too_long_uri() {
        let long_uri = format!("rustle://song/{}", "1".repeat(MAX_URI_LENGTH));
        assert_eq!(
            parse_rustle_uri(&long_uri),
            Err(UriError::TooLong(long_uri.len()))
        );
    }
}
