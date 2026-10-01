//! Read disposable display artwork directly from audio, without writing files.
use crate::error::{MediaError, MediaResult};
use lofty::{config::ParseOptions, file::TaggedFileExt, probe::Probe};
use std::path::Path;

pub fn read_audio_cover(path: &Path) -> MediaResult<Option<image::DynamicImage>> {
    let mut failure = None;
    match Probe::open(path).and_then(|probe| {
        probe
            .options(ParseOptions::new().read_properties(false))
            .read()
    }) {
        Ok(tagged) => {
            let pictures = || tagged.tags().iter().flat_map(|tag| tag.pictures());
            if let Some(picture) = pictures()
                .find(|picture| picture.pic_type() == lofty::picture::PictureType::CoverFront)
                .or_else(|| pictures().next())
            {
                match image::load_from_memory(picture.data()) {
                    Ok(image) => return Ok(Some(image)),
                    Err(source) => {
                        failure = Some(MediaError::Image {
                            operation: "decode embedded cover",
                            source,
                        })
                    }
                }
            }
        }
        Err(error) => failure = Some(MediaError::metadata("read artwork tags", error)),
    }
    if let Some(path) = super::find_cover_art(path) {
        return image::open(path)
            .map(Some)
            .map_err(|source| MediaError::Image {
                operation: "decode external cover",
                source,
            });
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(None),
    }
}
