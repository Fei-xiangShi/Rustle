//! Portable synthetic media fixtures. No user library or network dependency.
use lofty::{config::WriteOptions, tag::TagExt};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct TestDir(pub PathBuf);
impl TestDir {
    pub fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustle-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn write_mp3(path: &Path, tag: &impl TagExt) {
    let mut bytes = Vec::new();
    tag.dump_to(&mut bytes, WriteOptions::default()).unwrap();
    // Three MPEG1 Layer III 128 kbit/s, 44.1 kHz frames for format probing.
    for _ in 0..3 {
        let mut frame = vec![0; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
        bytes.extend(frame);
    }
    fs::write(path, bytes).unwrap();
}
