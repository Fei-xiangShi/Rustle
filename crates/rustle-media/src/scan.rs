//! Local audio-file discovery and inspection primitives.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use walkdir::WalkDir;
use xxhash_rust::xxh3::xxh3_64;

use crate::cover::{CoverCache, find_cover_art};
use crate::error::{MediaError, MediaResult};
use crate::metadata::{AudioMetadata, apply_smart_parsing, extract_metadata, resolve_track_gain};

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "ogg", "wav", "opus", "wma", "aiff",
];

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
}

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub compute_hash: bool,
    pub extract_covers: bool,
    pub smart_parsing: bool,
    pub max_depth: Option<usize>,
    pub extensions: Vec<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            compute_hash: true,
            extract_covers: true,
            smart_parsing: true,
            max_depth: None,
            extensions: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct ScanResult {
    pub metadata: AudioMetadata,
    pub file_size: u64,
    pub file_hash: Option<String>,
    pub normalization_gain: Option<f64>,
    pub cover_path: Option<PathBuf>,
}

pub fn discover_audio_files(root: &Path, config: &ScanConfig) -> Vec<PathBuf> {
    let walker = match config.max_depth {
        Some(max_depth) => WalkDir::new(root)
            .max_depth(max_depth)
            .follow_links(true)
            .into_iter(),
        None => WalkDir::new(root).follow_links(true).into_iter(),
    };

    walker
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            if config.extensions.is_empty() {
                is_audio_file(path)
            } else {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        config
                            .extensions
                            .iter()
                            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
                    })
            }
        })
        .collect()
}

pub fn inspect_audio_files(
    paths: &[PathBuf],
    config: &ScanConfig,
    cover_cache: Option<&CoverCache>,
) -> Vec<(PathBuf, MediaResult<ScanResult>)> {
    paths
        .par_iter()
        .map(|path| (path.clone(), scan_audio_file(path, config, cover_cache)))
        .collect()
}

pub fn compute_partial_hash(path: &Path) -> MediaResult<String> {
    const CHUNK_SIZE: usize = 64 * 1024;

    let mut file = std::fs::File::open(path).map_err(|error| MediaError::io("open", error))?;
    let file_size = file
        .metadata()
        .map_err(|error| MediaError::io("metadata", error))?
        .len();
    let mut hasher_data = Vec::with_capacity(CHUNK_SIZE * 2 + 8);
    hasher_data.extend_from_slice(&file_size.to_le_bytes());

    let mut first_chunk = vec![0_u8; CHUNK_SIZE.min(file_size as usize)];
    file.read_exact(&mut first_chunk)
        .map_err(|error| MediaError::io("read hash prefix", error))?;
    hasher_data.extend_from_slice(&first_chunk);

    if file_size > CHUNK_SIZE as u64 * 2 {
        file.seek(SeekFrom::End(-(CHUNK_SIZE as i64)))
            .map_err(|error| MediaError::io("seek hash suffix", error))?;
        let mut last_chunk = vec![0_u8; CHUNK_SIZE];
        file.read_exact(&mut last_chunk)
            .map_err(|error| MediaError::io("read hash suffix", error))?;
        hasher_data.extend_from_slice(&last_chunk);
    }

    Ok(format!("{:016x}", xxh3_64(&hasher_data)))
}

pub fn scan_audio_file(
    path: &Path,
    config: &ScanConfig,
    cover_cache: Option<&CoverCache>,
) -> MediaResult<ScanResult> {
    if !is_audio_file(path) {
        return Err(MediaError::UnsupportedFormat);
    }

    let file_meta = std::fs::metadata(path).map_err(|error| MediaError::io("metadata", error))?;
    if file_meta.len() == 0 {
        return Err(MediaError::EmptyMedia);
    }

    let mut metadata = match extract_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::debug!(error = %error, "media metadata inspection failed");
            return if has_known_audio_header(path).unwrap_or(false) {
                Err(MediaError::InvalidMedia)
            } else {
                Err(MediaError::UnsupportedFormat)
            };
        }
    };
    if config.smart_parsing
        && let Some(filename) = path.file_name().and_then(|name| name.to_str())
    {
        apply_smart_parsing(&mut metadata, filename);
    }

    let file_hash = config
        .compute_hash
        .then(|| compute_partial_hash(path))
        .transpose()?;
    let normalization_gain = resolve_track_gain(path).map(f64::from);
    let cover_path = if config.extract_covers {
        if let (Some(data), Some(cache)) = (&metadata.cover_data, cover_cache) {
            match cache.save_cover_with_mime(data, metadata.cover_mime.as_deref()) {
                Ok((_, path)) => Some(path),
                Err(error) => {
                    tracing::warn!(error = %error, "embedded cover cache failed");
                    find_cover_art(path)
                }
            }
        } else {
            find_cover_art(path)
        }
    } else {
        None
    };

    Ok(ScanResult {
        metadata,
        file_size: file_meta.len(),
        file_hash,
        normalization_gain,
        cover_path,
    })
}

fn has_known_audio_header(path: &Path) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0_u8; 64];
    let len = file.read(&mut header)?;
    let bytes = &header[..len];
    Ok(bytes.starts_with(b"fLaC")
        || bytes.starts_with(b"ID3")
        || matches!(
            bytes.get(0..2),
            Some([0xFF, 0xFB] | [0xFF, 0xFA] | [0xFF, 0xF3] | [0xFF, 0xF2])
        )
        || bytes.starts_with(b"OggS")
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE")
        || (bytes.len() >= 8 && &bytes[4..8] == b"ftyp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_matching_is_ascii_case_insensitive() {
        assert!(is_audio_file(Path::new("track.FLAC")));
        assert!(!is_audio_file(Path::new("cover.png")));
    }
}
