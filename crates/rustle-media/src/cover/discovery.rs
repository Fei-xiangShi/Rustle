//! External cover art discovery
//!
//! Finds cover art from same-name image files or common album art filenames.

use std::path::{Path, PathBuf};

/// Common cover art filenames to search for (in priority order)
const COVER_FILENAMES: &[&str] = &[
    "cover",
    "folder",
    "front",
    "albumart",
    "album",
    "artwork",
    "art",
    "thumb",
    "thumbnail",
];

/// Supported image extensions
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];

/// Find cover art for an audio file
///
/// Priority:
/// 1. Same-name image file
/// 2. Common cover filenames in same directory
pub fn find_cover_art(audio_path: &Path) -> Option<PathBuf> {
    if let Some(external) = find_same_name_image(audio_path) {
        return Some(external);
    }

    if let Some(external) = find_common_cover_file(audio_path) {
        return Some(external);
    }

    None
}

/// Find image file with same name as audio file
fn find_same_name_image(audio_path: &Path) -> Option<PathBuf> {
    let parent = audio_path.parent()?;
    let stem = audio_path.file_stem()?.to_str()?;

    for ext in IMAGE_EXTENSIONS {
        let image_path = parent.join(format!("{}.{}", stem, ext));
        if image_path.exists() {
            return Some(image_path);
        }

        // Also check uppercase extension
        let image_path_upper = parent.join(format!("{}.{}", stem, ext.to_uppercase()));
        if image_path_upper.exists() {
            return Some(image_path_upper);
        }
    }

    None
}

/// Find common cover art file in same directory
fn find_common_cover_file(audio_path: &Path) -> Option<PathBuf> {
    let parent = audio_path.parent()?;

    for filename in COVER_FILENAMES {
        for ext in IMAGE_EXTENSIONS {
            // Try lowercase
            let cover_path = parent.join(format!("{}.{}", filename, ext));
            if cover_path.exists() {
                return Some(cover_path);
            }

            // Try capitalized
            let capitalized = format!(
                "{}{}",
                filename.chars().next()?.to_uppercase(),
                &filename[1..]
            );
            let cover_path = parent.join(format!("{}.{}", capitalized, ext));
            if cover_path.exists() {
                return Some(cover_path);
            }

            // Try uppercase
            let cover_path = parent.join(format!(
                "{}.{}",
                filename.to_uppercase(),
                ext.to_uppercase()
            ));
            if cover_path.exists() {
                return Some(cover_path);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cover_filenames_priority() {
        // cover.jpg should be found before folder.jpg
        assert_eq!(COVER_FILENAMES[0], "cover");
        assert_eq!(COVER_FILENAMES[1], "folder");
    }
}
