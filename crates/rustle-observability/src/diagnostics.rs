//! User-initiated, privacy-safe diagnostic archive export.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOTAL_INPUT_BYTES: usize = 64 * 1024 * 1024;
static EXPORT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticError {
    #[error("diagnostic I/O failed")]
    Io(#[from] io::Error),
    #[error("diagnostic archive failed")]
    Archive(#[from] zip::result::ZipError),
    #[error("diagnostic manifest serialization failed")]
    Manifest(#[from] serde_json::Error),
}

pub type DiagnosticResult<T> = Result<T, DiagnosticError>;

#[derive(Debug)]
pub struct ExportSummary {
    pub destination: PathBuf,
    pub included_files: usize,
    pub included_bytes: usize,
    pub truncated_files: usize,
    pub omitted_files: usize,
}

#[derive(Debug, Clone, Copy)]
struct ExportLimits {
    max_entry_bytes: usize,
    max_total_bytes: usize,
}

impl Default for ExportLimits {
    fn default() -> Self {
        Self {
            max_entry_bytes: MAX_ENTRY_BYTES,
            max_total_bytes: MAX_TOTAL_INPUT_BYTES,
        }
    }
}

#[derive(Debug)]
struct ArchiveEntry {
    name: String,
    contents: Vec<u8>,
}

#[derive(Debug, Default)]
struct CollectionResult {
    entries: Vec<ArchiveEntry>,
    discovered_files: usize,
    truncated_files: usize,
    omitted_files: usize,
    directory_failures: usize,
    included_bytes: usize,
}

#[derive(Serialize)]
struct DiagnosticManifest<'a> {
    format: &'static str,
    app_version: &'static str,
    git_commit: &'static str,
    rust_toolchain: &'static str,
    os: &'static str,
    arch: &'static str,
    included_files: usize,
    included_bytes: usize,
    discovered_files: usize,
    truncated_files: usize,
    omitted_files: usize,
    directory_failures: usize,
    max_entry_bytes: usize,
    max_total_input_bytes: usize,
    privacy: &'a [&'a str],
}

/// Export a local diagnostic ZIP containing only redacted Rustle log/crash files.
///
/// The destination is never overwritten. Callers should ask the user to choose
/// or remove an existing file rather than silently replacing prior diagnostics.
pub fn export(destination: impl AsRef<Path>) -> DiagnosticResult<ExportSummary> {
    let directories = crate::diagnostic_directories();
    let redactor = crate::diagnostic_redactor();
    export_from_directories(
        destination.as_ref(),
        &directories,
        &redactor,
        ExportLimits::default(),
    )
}

fn export_from_directories(
    destination: &Path,
    directories: &[PathBuf],
    redactor: &crate::Redactor,
    limits: ExportLimits,
) -> DiagnosticResult<ExportSummary> {
    if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "diagnostic destination already exists",
        )
        .into());
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let collection = collect_entries(directories, redactor, limits);
    let manifest = DiagnosticManifest {
        format: "rustle-diagnostics-v1",
        app_version: env!("CARGO_PKG_VERSION"),
        git_commit: crate::build_commit(),
        rust_toolchain: crate::pinned_toolchain(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        included_files: collection.entries.len(),
        included_bytes: collection.included_bytes,
        discovered_files: collection.discovered_files,
        truncated_files: collection.truncated_files,
        omitted_files: collection.omitted_files,
        directory_failures: collection.directory_failures,
        max_entry_bytes: limits.max_entry_bytes,
        max_total_input_bytes: limits.max_total_bytes,
        privacy: &[
            "owned log/crash files only",
            "all text re-redacted during export",
            "no settings, cookies, database, cache, media, or environment dump",
        ],
    };
    let manifest = serde_json::to_vec_pretty(&manifest)?;

    let (temporary_path, file) = create_temporary_output(destination)?;
    let mut temporary = TemporaryArtifact::new(temporary_path);
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o600);
    archive.start_file("manifest.json", options)?;
    archive.write_all(&manifest)?;
    for entry in &collection.entries {
        archive.start_file(&entry.name, options)?;
        archive.write_all(&entry.contents)?;
    }
    let file = archive.finish()?;
    file.sync_all()?;
    drop(file);
    publish_temporary(&mut temporary, destination)?;

    Ok(ExportSummary {
        destination: destination.to_path_buf(),
        included_files: collection.entries.len(),
        included_bytes: collection.included_bytes,
        truncated_files: collection.truncated_files,
        omitted_files: collection.omitted_files,
    })
}

fn collect_entries(
    directories: &[PathBuf],
    redactor: &crate::Redactor,
    limits: ExportLimits,
) -> CollectionResult {
    let mut result = CollectionResult::default();
    let mut candidates = Vec::new();
    let mut names = HashSet::new();

    for directory in directories {
        let read_dir = match fs::read_dir(directory) {
            Ok(read_dir) => read_dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                result.directory_failures += 1;
                continue;
            }
        };
        for entry in read_dir.filter_map(Result::ok) {
            let Ok(file_type) = entry.file_type() else {
                result.omitted_files += 1;
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(file_name_text) = file_name.to_str() else {
                continue;
            };
            let archive_directory = if crate::is_owned_log_name(&file_name) {
                "logs"
            } else if crate::is_owned_crash_name(&file_name) {
                "crashes"
            } else {
                continue;
            };
            result.discovered_files += 1;
            let archive_name = format!("{archive_directory}/{file_name_text}");
            if names.insert(archive_name.clone()) {
                candidates.push((archive_name, entry.path()));
            }
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));

    for (name, path) in candidates {
        let remaining = limits.max_total_bytes.saturating_sub(result.included_bytes);
        if remaining == 0 {
            result.omitted_files += 1;
            continue;
        }
        let read_limit = limits.max_entry_bytes.saturating_add(1);
        let mut contents = Vec::new();
        let read_result = File::open(path).and_then(|file| {
            file.take(read_limit as u64)
                .read_to_end(&mut contents)
                .map(|_| ())
        });
        if read_result.is_err() {
            result.omitted_files += 1;
            continue;
        }

        let was_entry_truncated = contents.len() > limits.max_entry_bytes;
        contents.truncate(limits.max_entry_bytes);
        let redacted = redactor.redact(&String::from_utf8_lossy(&contents));
        let allowed = remaining.min(limits.max_entry_bytes);
        let was_total_truncated = redacted.len() > allowed;
        let redacted = truncate_utf8(&redacted, allowed).as_bytes().to_vec();
        if redacted.is_empty() && !contents.is_empty() {
            result.omitted_files += 1;
            continue;
        }
        if was_entry_truncated || was_total_truncated {
            result.truncated_files += 1;
        }
        result.included_bytes = result.included_bytes.saturating_add(redacted.len());
        result.entries.push(ArchiveEntry {
            name,
            contents: redacted,
        });
    }
    result
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn create_temporary_output(destination: &Path) -> io::Result<(PathBuf, File)> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rustle-diagnostics.zip");
    for _ in 0..32 {
        let sequence = EXPORT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{file_name}.tmp-{}-{sequence}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique diagnostic temporary file",
    ))
}

fn publish_temporary(temporary: &mut TemporaryArtifact, destination: &Path) -> io::Result<()> {
    // The temporary file is a sibling of the destination, so a hard-link
    // creation is an atomic, same-filesystem no-clobber publish on every
    // supported desktop platform. Unlike rename on Unix, it cannot replace a
    // file created after the initial existence check.
    fs::hard_link(temporary.path(), destination)?;
    if let Err(error) = fs::remove_file(temporary.path()) {
        let _ = fs::remove_file(destination);
        return Err(error);
    }
    temporary.mark_published();
    Ok(())
}

struct TemporaryArtifact {
    path: PathBuf,
    published: bool,
}

impl TemporaryArtifact {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            published: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn mark_published(&mut self) {
        self.published = true;
    }
}

impl Drop for TemporaryArtifact {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use zip::ZipArchive;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustle-diagnostics-test-{}-{}",
                std::process::id(),
                EXPORT_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn export_uses_owned_files_re_redacts_and_publishes_atomically() {
        let directory = TestDirectory::new();
        let source = directory.0.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("rustle-session-a1-b2-000.log"),
            "token=hidden url=https://example.test/?signed=yes path=/home/alice/song.flac",
        )
        .unwrap();
        fs::write(
            source.join("rustle-crash-a1-b2-001.jsonl"),
            r#"{"payload":"cookie=secret"}"#,
        )
        .unwrap();
        fs::write(source.join("settings.json"), "password=keep-out").unwrap();
        let destination = directory.0.join("bundle.zip");

        let summary = export_from_directories(
            &destination,
            &[source],
            &crate::Redactor::new(None),
            ExportLimits::default(),
        )
        .unwrap();
        assert_eq!(summary.included_files, 2);
        assert!(destination.is_file());

        let mut archive = ZipArchive::new(File::open(&destination).unwrap()).unwrap();
        let names = archive.file_names().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "manifest.json",
                "crashes/rustle-crash-a1-b2-001.jsonl",
                "logs/rustle-session-a1-b2-000.log"
            ]
        );
        let mut combined = String::new();
        for index in 0..archive.len() {
            archive
                .by_index(index)
                .unwrap()
                .read_to_string(&mut combined)
                .unwrap();
        }
        for secret in [
            "hidden",
            "signed=yes",
            "alice",
            "song.flac",
            "secret",
            "keep-out",
        ] {
            assert!(!combined.contains(secret), "archive leaked {secret}");
        }
    }

    #[test]
    fn export_limits_each_entry_and_total_input() {
        let directory = TestDirectory::new();
        let source = directory.0.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("rustle-session-a1-000.log"), "x".repeat(80)).unwrap();
        fs::write(source.join("rustle-session-a2-000.log"), "y".repeat(80)).unwrap();
        let destination = directory.0.join("limited.zip");

        let summary = export_from_directories(
            &destination,
            &[source],
            &crate::Redactor::new(None),
            ExportLimits {
                max_entry_bytes: 48,
                max_total_bytes: 64,
            },
        )
        .unwrap();

        assert_eq!(summary.included_bytes, 64);
        assert_eq!(summary.truncated_files, 2);
        assert_eq!(summary.omitted_files, 0);
    }

    #[test]
    fn export_refuses_to_overwrite_an_existing_bundle() {
        let directory = TestDirectory::new();
        let destination = directory.0.join("existing.zip");
        fs::write(&destination, b"keep").unwrap();
        let error = export_from_directories(
            &destination,
            &[],
            &crate::Redactor::new(None),
            ExportLimits::default(),
        )
        .unwrap_err();

        assert!(
            matches!(error, DiagnosticError::Io(ref io) if io.kind() == io::ErrorKind::AlreadyExists)
        );
        assert_eq!(fs::read(destination).unwrap(), b"keep");
    }

    #[test]
    fn atomic_publish_refuses_a_destination_created_after_collection() {
        let directory = TestDirectory::new();
        let destination = directory.0.join("raced.zip");
        let (temporary_path, mut file) = create_temporary_output(&destination).unwrap();
        file.write_all(b"new diagnostic").unwrap();
        file.sync_all().unwrap();
        drop(file);
        let mut temporary = TemporaryArtifact::new(temporary_path);
        fs::write(&destination, b"existing diagnostic").unwrap();

        let error = publish_temporary(&mut temporary, &destination).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(destination).unwrap(), b"existing diagnostic");
    }
}
