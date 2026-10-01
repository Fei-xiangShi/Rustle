//! Blocking artwork decoding. Audio tags are authoritative; no song JPEG cache.
use super::{ImageKind, ImageVariant};
use crate::app::ImageEntry;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub fn load(path: &Path) -> Option<image::DynamicImage> {
    if path.extension().is_some_and(|ext| {
        crate::utils::AUDIO_EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    }) {
        return rustle_media::cover::read_audio_cover(path).ok().flatten();
    }
    image::open(path).ok()
}

pub fn prepare(
    kind: ImageKind,
    variant: ImageVariant,
    mut path: PathBuf,
    source: String,
    image: image::DynamicImage,
) -> ImageEntry {
    let backing_file = path.is_absolute().then(|| path.clone());
    let edge = u32::from(variant.edge());
    let image = if image.width() > edge || image.height() > edge {
        image.thumbnail(edge, edge)
    } else {
        image
    };
    let dimensions = Some((image.width(), image.height()));
    let pixels = image.to_rgba8();
    if matches!(kind, ImageKind::SongCover | ImageKind::LocalSongCover) {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        dimensions.hash(&mut hash);
        pixels.as_raw().hash(&mut hash);
        // A request's cancellation generation is never artwork identity.
        path = PathBuf::from(format!("{}/content-{:016x}", path.display(), hash.finish()));
    }
    let handle =
        iced::widget::image::Handle::from_rgba(pixels.width(), pixels.height(), pixels.into_raw());
    let playlist_footer_handle =
        (kind == ImageKind::PlaylistCover && variant == ImageVariant::Thumbnail).then(|| {
            let processed =
                crate::ui::effects::image_processing::process_image_for_playlist_footer(&image);
            iced::widget::image::Handle::from_rgba(
                processed.width,
                processed.height,
                processed.data,
            )
        });
    ImageEntry {
        backing_file,
        palette: Some(crate::utils::ColorPalette {
            primary: crate::utils::DominantColors::from_image(&image).primary,
        }),
        path,
        handle,
        dimensions,
        playlist_footer_handle,
        artwork: Some(Arc::new(image)),
        source_url_digest: Some(super::source_url_digest(&source)),
        processing_version: Some(super::IMAGE_PROCESSING_VERSION),
        source_url: Some(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::{
        config::WriteOptions,
        picture::{MimeType, Picture, PictureType},
        tag::{Tag, TagExt, TagType},
    };

    fn entry(color: u8, source: &str) -> ImageEntry {
        prepare(
            ImageKind::SongCover,
            ImageVariant::Thumbnail,
            "artwork://SongCover/42/Thumbnail".into(),
            source.into(),
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                2,
                2,
                image::Rgb([color, 0, 0]),
            )),
        )
    }

    #[test]
    fn same_pixels_retain_gpu_handle_across_source_refresh_and_other_songs_survive() {
        let mut state = crate::app::ImageState::default();
        let kind = ImageKind::SongCover;
        let variant = ImageVariant::Thumbnail;
        let original = entry(100, "https://original/cover");
        let path = original.path.clone();
        let handle_id = original.handle.id();
        state
            .known_sources
            .insert((kind, 42), "https://original/cover".into());
        state.insert_prepared(kind, 42, variant, original);
        state.insert_prepared(kind, 43, variant, entry(120, "https://other/cover"));
        let other_id = state.get(kind, 43).unwrap().id();
        state.refresh_retaining_image(kind, 42);
        assert_eq!(state.get(kind, 42).unwrap().id(), handle_id);
        assert_eq!(state.get(kind, 43).unwrap().id(), other_id);
        assert!(!state.has_current_remote_variant(kind, 42, variant, "https://original/cover"));
        let refreshed = entry(100, "https://replacement/cover");
        assert_eq!(refreshed.path, path);
        state.insert_prepared(kind, 42, variant, refreshed);
        assert_eq!(state.get(kind, 42).unwrap().id(), handle_id);
        assert!(state.has_current_remote_variant(kind, 42, variant, "https://replacement/cover"));
        let changed = entry(200, "https://replacement/cover");
        assert_ne!(changed.path, path);
        state.insert_prepared(kind, 42, variant, changed);
        assert_ne!(state.get(kind, 42).unwrap().id(), handle_id);
        assert_eq!(state.get(kind, 43).unwrap().id(), other_id);
    }

    #[test]
    fn embedded_front_cover_wins_without_creating_a_disk_thumbnail() {
        let root =
            crate::cache::unique_temp_path(&std::env::temp_dir().join("rustle-embedded-cover"));
        std::fs::create_dir_all(&root).unwrap();
        let audio = root.join("曲目, with spaces.mp3");
        let mut tag = Tag::new(TagType::Id3v2);
        for (kind, color) in [
            (PictureType::CoverBack, [255, 0, 0]),
            (PictureType::CoverFront, [0, 0, 255]),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::RgbImage::from_pixel(8, 8, image::Rgb(color))
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            tag.push_picture(
                Picture::unchecked(bytes.into_inner())
                    .pic_type(kind)
                    .mime_type(MimeType::Png)
                    .build(),
            );
        }
        let mut bytes = Vec::new();
        tag.dump_to(&mut bytes, WriteOptions::default()).unwrap();
        for _ in 0..3 {
            let mut frame = vec![0; 417];
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
            bytes.extend(frame);
        }
        std::fs::write(&audio, bytes).unwrap();
        let decoded = load(&audio).unwrap();
        assert_eq!(decoded.to_rgb8().get_pixel(0, 0).0, [0, 0, 255]);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        let prepared = prepare(
            ImageKind::LocalSongCover,
            ImageVariant::Thumbnail,
            audio.clone(),
            audio.to_string_lossy().into_owned(),
            decoded,
        );
        assert_eq!(prepared.dimensions, Some((8, 8)));
        assert!(prepared.artwork.is_some());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        let cache_dir = root.join("covers");
        let cache = rustle_media::cover::CoverCache::new(
            cache_dir.clone(),
            crate::cache::cache_publisher(),
        )
        .unwrap();
        let scanned = rustle_media::scan::scan_audio_file(
            &audio,
            &rustle_media::scan::ScanConfig::default(),
            Some(&cache),
        )
        .unwrap();
        assert!(scanned.cover_path.is_none());
        assert_eq!(std::fs::read_dir(&cache_dir).unwrap().count(), 0);
        std::fs::remove_dir(cache_dir).unwrap();
        std::fs::remove_file(&audio).unwrap();
        assert!(load(&audio).is_none());
        std::fs::remove_dir(root).unwrap();
    }
}
