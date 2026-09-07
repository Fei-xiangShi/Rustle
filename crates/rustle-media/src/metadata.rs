//! Audio metadata extraction with encoding fallback
//!
//! Uses lofty for metadata reading, with custom encoding handling
//! for legacy files that use GBK/Shift-JIS/etc.

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag};
use rodio::{Decoder, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use super::encoding::{decode_string, normalize_string};
use crate::error::{MediaError, MediaResult};

/// Extracted metadata from an audio file
#[derive(Debug, Clone)]
pub struct AudioMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: i64,
    pub track_number: Option<i64>,
    pub year: Option<i64>,
    pub genre: Option<String>,
    pub format: String,
    /// Raw cover art data (if present)
    pub cover_data: Option<Vec<u8>>,
    /// Cover art MIME type
    pub cover_mime: Option<String>,
}

impl Default for AudioMetadata {
    fn default() -> Self {
        Self {
            title: "Unknown Title".to_string(),
            artist: "Unknown Artist".to_string(),
            album: "Unknown Album".to_string(),
            duration_secs: 0,
            track_number: None,
            year: None,
            genre: None,
            format: "unknown".to_string(),
            cover_data: None,
            cover_mime: None,
        }
    }
}

/// Extract metadata from an audio file
pub fn extract_metadata(path: &Path) -> MediaResult<AudioMetadata> {
    let tagged_file = Probe::open(path)
        .map_err(|error| MediaError::metadata("open audio file", error))?
        .read()
        .map_err(|error| MediaError::metadata("read audio file", error))?;

    let properties = tagged_file.properties();
    let duration = properties.duration();

    // Determine format from file extension
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "unknown".to_string());

    // Try to get the primary tag, or any available tag
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let mut metadata = AudioMetadata {
        duration_secs: duration.as_secs() as i64,
        format,
        ..Default::default()
    };

    if let Some(tag) = tag {
        // Extract title with encoding fallback
        if let Some(title) = tag.title() {
            metadata.title = normalize_string(&decode_string(title.as_bytes()));
        }

        // Extract artist with encoding fallback
        if let Some(artist) = tag.artist() {
            metadata.artist = normalize_string(&decode_string(artist.as_bytes()));
        }

        // Extract album with encoding fallback
        if let Some(album) = tag.album() {
            metadata.album = normalize_string(&decode_string(album.as_bytes()));
        }

        // Track number
        metadata.track_number = tag.track().map(|t| t as i64);

        // Year
        metadata.year = tag.date().map(|date| i64::from(date.year));

        // Genre with encoding fallback
        if let Some(genre) = tag.genre() {
            metadata.genre = Some(normalize_string(&decode_string(genre.as_bytes())));
        }

        // Extract cover art
        if let Some(picture) = tag.pictures().first() {
            metadata.cover_data = Some(picture.data().to_vec());
            metadata.cover_mime = Some(
                picture
                    .mime_type()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "image/jpeg".to_string()),
            );
        }
    }

    // If title is still unknown, use filename
    if metadata.title == "Unknown Title" {
        metadata.title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Unknown Title".to_string());
    }

    Ok(metadata)
}

/// Extract track normalization gain from audio tags.
///
/// Returns linear gain to multiply the player volume by.
/// Priority:
/// 1. `REPLAYGAIN_TRACK_GAIN`
/// 2. `REPLAYGAIN_ALBUM_GAIN`
/// 3. `R128_TRACK_GAIN`
/// 4. `R128_ALBUM_GAIN`
pub fn extract_track_gain(path: &Path) -> Option<f32> {
    let tagged_file = Probe::open(path).ok()?.read().ok()?;

    tagged_file
        .tags()
        .iter()
        .find_map(extract_track_gain_from_tag)
}

/// Resolve normalization gain from tags or by analyzing the decoded waveform.
pub fn resolve_track_gain(path: &Path) -> Option<f32> {
    extract_track_gain(path).or_else(|| analyze_track_gain(path))
}

fn extract_track_gain_from_tag(tag: &Tag) -> Option<f32> {
    tag.get_string(ItemKey::ReplayGainTrackGain)
        .and_then(parse_replaygain_db)
        .map(db_to_linear)
        .or_else(|| {
            tag.get_string(ItemKey::ReplayGainAlbumGain)
                .and_then(parse_replaygain_db)
                .map(db_to_linear)
        })
        .or_else(|| extract_r128_gain(tag, ItemKey::R128TrackGain))
        .or_else(|| extract_r128_gain(tag, ItemKey::R128AlbumGain))
}

fn extract_r128_gain(tag: &Tag, key: ItemKey) -> Option<f32> {
    tag.get_string(key)
        .and_then(parse_r128_db)
        .map(db_to_linear)
}

fn parse_replaygain_db(value: &str) -> Option<f32> {
    let cleaned = value
        .trim()
        .trim_end_matches(" dB")
        .trim_end_matches("dB")
        .trim();
    cleaned.parse::<f32>().ok()
}

fn parse_r128_db(value: &str) -> Option<f32> {
    let raw = value.trim().parse::<f32>().ok()?;
    Some(raw / 256.0)
}

fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

fn analyze_track_gain(path: &Path) -> Option<f32> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let decoder = Decoder::new(reader).ok()?;

    let channels = decoder.channels().get() as usize;
    let sample_rate = decoder.sample_rate().get() as usize;
    let stride = ((sample_rate * channels) / 4_000).max(1);

    let mut sum_sq = 0.0_f64;
    let mut sample_count = 0_u64;
    let mut peak = 0.0_f32;

    for (idx, sample) in decoder.enumerate() {
        if idx % stride != 0 {
            continue;
        }

        let sample = sample.clamp(-1.0, 1.0);
        peak = peak.max(sample.abs());
        sum_sq += f64::from(sample) * f64::from(sample);
        sample_count += 1;
    }

    if sample_count == 0 {
        return None;
    }

    let rms = (sum_sq / sample_count as f64).sqrt() as f32;
    if rms <= 1e-6 {
        return Some(1.0);
    }

    // Approximate integrated loudness target near -18 dBFS.
    let target_rms = 10.0_f32.powf(-18.0 / 20.0);
    let min_gain = 10.0_f32.powf(-18.0 / 20.0);
    let max_gain = 10.0_f32.powf(12.0 / 20.0);

    let mut gain = (target_rms / rms).clamp(min_gain, max_gain);

    // Keep some headroom to avoid clipping after normalization.
    if peak > 1e-6 {
        gain = gain.min(0.98 / peak);
    }

    Some(gain.clamp(min_gain, max_gain))
}

/// Try to parse artist and title from filename
///
/// Common patterns:
/// - "Artist - Title.mp3"
/// - "Artist_-_Title.mp3"
/// - "01 - Artist - Title.mp3"
/// - "01. Title.mp3"
/// - "Title.mp3"
pub fn parse_filename(filename: &str) -> (Option<String>, Option<String>) {
    // Remove extension
    let name = filename
        .rsplit_once('.')
        .map(|(name, _)| name)
        .unwrap_or(filename);

    // Try "Artist - Title" pattern (most common)
    if let Some((artist, title)) = name.split_once(" - ") {
        // Check if artist part is just a track number
        let artist_trimmed = artist.trim();
        if artist_trimmed.parse::<u32>().is_ok()
            || artist_trimmed
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.')
        {
            // It's a track number, so the rest is the title
            return (None, Some(normalize_string(title)));
        }
        return (
            Some(normalize_string(artist_trimmed)),
            Some(normalize_string(title)),
        );
    }

    // Try "Artist_-_Title" pattern
    if let Some((artist, title)) = name.split_once("_-_") {
        return (
            Some(normalize_string(artist)),
            Some(normalize_string(title)),
        );
    }

    // Try "01. Title" or "01 Title" pattern
    let name_trimmed = name.trim();
    if name_trimmed.len() > 3 {
        let first_chars: String = name_trimmed.chars().take(3).collect();
        if first_chars.chars().take(2).all(|c| c.is_ascii_digit()) {
            let rest = &name_trimmed[2..].trim_start_matches(['.', ' ', '_']);
            if !rest.is_empty() {
                return (None, Some(normalize_string(rest)));
            }
        }
    }

    // Just return the filename as title
    (None, Some(normalize_string(name)))
}

/// Apply smart filename parsing to fill in missing metadata
pub fn apply_smart_parsing(metadata: &mut AudioMetadata, filename: &str) {
    let (parsed_artist, parsed_title) = parse_filename(filename);

    // Only apply if metadata is missing
    if metadata.artist == "Unknown Artist"
        && let Some(artist) = parsed_artist
    {
        metadata.artist = artist;
    }

    if metadata.title == "Unknown Title"
        && let Some(title) = parsed_title
    {
        metadata.title = title;
    }
}

/// Editable metadata fields for saving back to file
#[derive(Debug, Clone, Default)]
pub struct MetadataEdits {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub cover_data: Option<Vec<u8>>,
    pub cover_mime: Option<String>,
}

/// Save metadata edits back to an audio file using lofty
pub fn save_metadata(path: &Path, edits: &MetadataEdits) -> MediaResult<()> {
    use lofty::config::WriteOptions;
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::Accessor;

    let mut tf = Probe::open(path)
        .map_err(|error| MediaError::metadata("open editable audio file", error))?
        .read()
        .map_err(|error| MediaError::metadata("read editable audio file", error))?;

    // Scope the tag borrow so save_to_path can take &mut tf
    {
        let tag = match tf.primary_tag_mut() {
            Some(tag) => tag,
            None => tf.first_tag_mut().ok_or(MediaError::MissingEditableTag)?,
        };

        if let Some(ref t) = edits.title {
            tag.set_title(t.clone());
        }
        if let Some(ref a) = edits.artist {
            tag.set_artist(a.clone());
        }
        if let Some(ref a) = edits.album {
            tag.set_album(a.clone());
        }
        if let Some(n) = edits.track_number {
            tag.set_track(n);
        }
        if let Some(y) = edits.year {
            let year = u16::try_from(y)
                .ok()
                .filter(|year| *year <= 9999)
                .ok_or(MediaError::InvalidYear(y))?;
            tag.set_date(Timestamp {
                year,
                ..Timestamp::default()
            });
        }
        if let Some(ref g) = edits.genre {
            tag.set_genre(g.clone());
        }

        // Write cover art if provided
        if let (Some(data), Some(mime)) = (&edits.cover_data, &edits.cover_mime) {
            use lofty::picture::{MimeType, Picture, PictureType};
            let mime_type = if mime.contains("png") {
                MimeType::Png
            } else {
                MimeType::Jpeg
            };
            let picture = Picture::unchecked(data.clone())
                .pic_type(PictureType::CoverFront)
                .mime_type(mime_type)
                .build();
            // Replace first picture if exists, otherwise push
            if !tag.pictures().is_empty() {
                tag.set_picture(0, picture);
            } else {
                tag.push_picture(picture);
            }
        }
    } // tag borrow dropped here

    // FLAC backup before writing (known lofty#549 issue)
    let is_flac = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("flac"))
        .unwrap_or(false);
    if is_flac {
        let bak = path.with_extension("flac.bak");
        std::fs::copy(path, &bak).map_err(|error| MediaError::io("backup FLAC", error))?;
    }

    tf.save_to_path(path, WriteOptions::default())
        .map_err(|error| MediaError::metadata("save audio tags", error))?;

    // Remove backup on success
    if is_flac {
        let _ = std::fs::remove_file(path.with_extension("flac.bak"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_filename_artist_title() {
        let (artist, title) = parse_filename("周杰伦 - 七里香.mp3");
        assert_eq!(artist, Some("周杰伦".to_string()));
        assert_eq!(title, Some("七里香".to_string()));
    }

    #[test]
    fn test_parse_filename_track_number() {
        let (artist, title) = parse_filename("01 - 七里香.mp3");
        assert_eq!(artist, None);
        assert_eq!(title, Some("七里香".to_string()));
    }

    #[test]
    fn test_parse_filename_simple() {
        let (artist, title) = parse_filename("七里香.mp3");
        assert_eq!(artist, None);
        assert_eq!(title, Some("七里香".to_string()));
    }

    #[test]
    fn test_parse_filename_numbered() {
        let (artist, title) = parse_filename("01. 七里香.mp3");
        assert_eq!(artist, None);
        assert_eq!(title, Some("七里香".to_string()));
    }

    #[test]
    fn test_parse_replaygain_db() {
        assert_eq!(parse_replaygain_db("-7.43 dB"), Some(-7.43));
        assert_eq!(parse_replaygain_db("+3.00 dB"), Some(3.0));
        assert_eq!(parse_replaygain_db("1.25"), Some(1.25));
    }

    #[test]
    fn test_parse_r128_db() {
        assert_eq!(parse_r128_db("-256"), Some(-1.0));
        assert_eq!(parse_r128_db("512"), Some(2.0));
    }
}
