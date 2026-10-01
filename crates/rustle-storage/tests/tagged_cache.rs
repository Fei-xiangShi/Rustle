use rustle_domain::audio::QualityLevel;
use rustle_storage::cache::{
    is_audio_cache_complete, unique_temp_path, write_tagged_audio_manifest,
};
use std::fs;

#[test]
fn tagged_cache_validates_remote_and_local_sizes_independently() {
    let root = unique_temp_path(&std::env::temp_dir().join("tagged-manifest"));
    fs::create_dir(&root).unwrap();
    let path = root.join("7_lossless.flac");
    fs::write(&path, b"audio plus embedded cover").unwrap();
    write_tagged_audio_manifest(&path, 7, QualityLevel::Lossless, 5, "flac").unwrap();
    assert!(is_audio_cache_complete(
        &path,
        7,
        QualityLevel::Lossless,
        Some(5)
    ));
    assert!(is_audio_cache_complete(
        &path,
        7,
        QualityLevel::Lossless,
        None
    ));
    assert!(!is_audio_cache_complete(
        &path,
        7,
        QualityLevel::Lossless,
        Some(6)
    ));
    assert!(!is_audio_cache_complete(
        &path,
        8,
        QualityLevel::Lossless,
        Some(5)
    ));
    fs::write(&path, b"truncated").unwrap();
    assert!(!is_audio_cache_complete(
        &path,
        7,
        QualityLevel::Lossless,
        Some(5)
    ));
    fs::remove_dir_all(root).unwrap();
}
