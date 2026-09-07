//! Online lyrics fetching module
//!
//! Fetches lyrics from NetEase Cloud Music API and caches them locally.

use anyhow::Result;
use std::path::PathBuf;

use super::{LyricLineOwned, LyricsFormat, merge_translation, parse_lyrics_with_format};
use crate::api::NcmClient;

/// Get cached lyrics file path for a song
fn get_cache_path(ncm_id: u64, suffix: &str) -> PathBuf {
    let cache_dir = crate::utils::lyrics_cache_dir();
    cache_dir.join(format!("{}{}", ncm_id, suffix))
}

/// Load cached word-level lyrics, if a YRC cache exists.
fn load_cached_yrc_lyrics(ncm_id: u64) -> Option<Vec<LyricLineOwned>> {
    let yrc_path = get_cache_path(ncm_id, ".yrc");
    if yrc_path.exists()
        && let Ok(content) = std::fs::read_to_string(&yrc_path)
    {
        let mut lines = super::parse_lyrics(&content);
        if !lines.is_empty() {
            // Try to load translation
            let tlrc_path = get_cache_path(ncm_id, ".tlrc");
            if tlrc_path.exists()
                && let Ok(trans_content) = std::fs::read_to_string(&tlrc_path)
            {
                let trans_lines = super::parse_lyrics(&trans_content);
                merge_translation(&mut lines, &trans_lines);
            }
            return Some(lines);
        }
    }

    None
}

/// Return whether a valid word-level cache exists for a song.
///
/// This is intentionally narrower than `load_cached_lyrics`: an old `.lrc`
/// cache must not prevent the first upgrade attempt to the v1 YRC endpoint.
pub fn has_cached_word_level_lyrics(ncm_id: u64) -> bool {
    load_cached_yrc_lyrics(ncm_id).is_some()
}

/// Load cached lyrics, preferring word-level YRC data.
pub fn load_cached_lyrics(ncm_id: u64) -> Option<Vec<LyricLineOwned>> {
    if let Some(lines) = load_cached_yrc_lyrics(ncm_id) {
        return Some(lines);
    }

    // Fall back to LRC
    let lrc_path = get_cache_path(ncm_id, ".lrc");
    if lrc_path.exists()
        && let Ok(content) = std::fs::read_to_string(&lrc_path)
    {
        let mut lines = super::parse_lyrics(&content);
        if !lines.is_empty() {
            // Try to load translation
            let tlrc_path = get_cache_path(ncm_id, ".tlrc");
            if tlrc_path.exists()
                && let Ok(trans_content) = std::fs::read_to_string(&tlrc_path)
            {
                let trans_lines = super::parse_lyrics(&trans_content);
                merge_translation(&mut lines, &trans_lines);
            }
            return Some(lines);
        }
    }

    None
}

/// Fetch YRC (word-level) lyrics from NCM API
pub async fn fetch_yrc_lyrics(
    client: &NcmClient,
    ncm_id: u64,
) -> Result<(Option<String>, Option<String>)> {
    let lyrics = client.song_lyric(ncm_id).await?;

    // The v1 endpoint exposes YRC separately from the line-level LRC. Keep
    // YRC intact so the parser can preserve each word's timing; only fall
    // back to LRC when the song has no word-level payload.
    let main_lyric = if !lyrics.yrc.is_empty() {
        Some(lyrics.yrc.join("\n"))
    } else if !lyrics.lyric.is_empty() {
        Some(lyrics.lyric.join("\n"))
    } else {
        None
    };

    // Prefer the word-level translation returned alongside YRC, while
    // retaining compatibility with the regular tlyric field.
    let trans_lyric = if !lyrics.ytlrc.is_empty() {
        Some(lyrics.ytlrc.join("\n"))
    } else if !lyrics.tlyric.is_empty() {
        Some(lyrics.tlyric.join("\n"))
    } else {
        None
    };

    Ok((main_lyric, trans_lyric))
}

/// Save lyrics to cache
pub fn save_lyrics_cache(
    ncm_id: u64,
    main_lyric: &str,
    trans_lyric: Option<&str>,
    is_yrc: bool,
) -> Result<()> {
    let cache_dir = crate::utils::lyrics_cache_dir();
    std::fs::create_dir_all(&cache_dir)?;

    let suffix = if is_yrc { ".yrc" } else { ".lrc" };
    let main_path = get_cache_path(ncm_id, suffix);
    write_cache_file(&main_path, main_lyric.as_bytes())?;

    if let Some(trans) = trans_lyric
        && !trans.is_empty()
    {
        let trans_path = get_cache_path(ncm_id, ".tlrc");
        write_cache_file(&trans_path, trans.as_bytes())?;
    }

    Ok(())
}

fn write_cache_file(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let temp = crate::cache::unique_temp_path(path);
    if let Err(error) = std::fs::write(&temp, bytes) {
        crate::cache::cleanup_temp_file(&temp);
        return Err(error.into());
    }
    crate::cache::publish_replace(&temp, path).map_err(Into::into)
}

/// Fetch and parse lyrics with automatic format detection
pub async fn fetch_lyrics(client: &NcmClient, ncm_id: u64) -> Result<Vec<LyricLineOwned>> {
    // A YRC cache is authoritative. An old LRC-only cache is deliberately not
    // returned yet: try the v1 endpoint once so existing users can upgrade to
    // word-level lyrics without manually deleting their cache.
    if let Some(cached) = load_cached_yrc_lyrics(ncm_id) {
        tracing::debug!("Loaded cached word-level lyrics for {}", ncm_id);
        return Ok(cached);
    }

    // Fetch from API
    let (main_lyric, trans_lyric) = match fetch_yrc_lyrics(client, ncm_id).await {
        Ok(lyrics) => lyrics,
        Err(error) => {
            // Preserve offline/legacy behavior when only an old LRC cache is
            // available and the online upgrade request cannot complete.
            if let Some(cached) = load_cached_lyrics(ncm_id) {
                tracing::debug!("Falling back to cached line-level lyrics for {}", ncm_id);
                return Ok(cached);
            }
            return Err(error);
        }
    };

    let main_lyric = match main_lyric {
        Some(lyric) => lyric,
        None => {
            if let Some(cached) = load_cached_lyrics(ncm_id) {
                tracing::debug!(
                    "Falling back to cached lyrics because the API returned none for {}",
                    ncm_id
                );
                return Ok(cached);
            }
            return Err(anyhow::anyhow!("No lyrics found"));
        }
    };

    // Detect format and parse
    let format = super::detect_format(&main_lyric);
    let is_yrc = format == LyricsFormat::Yrc;

    let mut lines = parse_lyrics_with_format(&main_lyric, format);

    // Merge translation if available
    if let Some(trans) = &trans_lyric {
        // YTLRC may itself contain word timing markers, so use automatic
        // format detection instead of assuming a plain LRC sidecar.
        let trans_lines = super::parse_lyrics(trans);
        merge_translation(&mut lines, &trans_lines);
    }

    // Save to cache
    if let Err(e) = save_lyrics_cache(ncm_id, &main_lyric, trans_lyric.as_deref(), is_yrc) {
        tracing::warn!("Failed to cache lyrics: {}", e);
    }

    Ok(lines)
}
