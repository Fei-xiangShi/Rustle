//! Database import orchestration over media-owned scanning primitives.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;

use super::cover::CoverCache;
use super::progress::{ProgressSender, ScanProgress, ScanState, SkipReason};
use crate::database::{Database, NewSong};

#[allow(unused_imports)]
pub use rustle_media::scan::{
    ScanConfig, ScanResult, compute_partial_hash, discover_audio_files, scan_audio_file,
};

struct PendingImport {
    path: PathBuf,
    file_name: String,
    title: String,
    artist: String,
    cover_path: Option<String>,
}

fn classify_media_error(error: rustle_media::MediaError) -> SkipReason {
    match error {
        rustle_media::MediaError::UnsupportedFormat => SkipReason::NotAudioFile,
        rustle_media::MediaError::EmptyMedia => SkipReason::EmptyFile,
        rustle_media::MediaError::InvalidMedia => SkipReason::Corrupted,
        other => SkipReason::MetadataError(other.to_string()),
    }
}

/// Scan a directory and import inspected media into the local database.
pub async fn scan_and_import(
    db: Arc<Database>,
    root: PathBuf,
    config: ScanConfig,
    cover_cache: Arc<CoverCache>,
    state: Arc<ScanState>,
    progress_tx: ProgressSender,
) -> Result<()> {
    let start_time = Instant::now();
    let files = tokio::task::spawn_blocking({
        let root = root.clone();
        let config = config.clone();
        move || discover_audio_files(&root, &config)
    })
    .await?;

    let total_files = files.len() as u64;
    state.set_total(total_files);
    state.set_scanned_paths(Vec::new());
    let _ = progress_tx.send(ScanProgress::Started { total_files });

    if total_files == 0 {
        let _ = progress_tx.send(ScanProgress::Completed {
            imported: 0,
            skipped: 0,
            errors: 0,
            duration_secs: start_time.elapsed().as_secs_f64(),
        });
        return Ok(());
    }

    for batch in files.chunks(100) {
        if state.is_cancelled() {
            let _ = progress_tx.send(ScanProgress::Cancelled);
            return Ok(());
        }

        let batch = batch.to_vec();
        let config = config.clone();
        let cover_cache = Arc::clone(&cover_cache);
        let results = tokio::task::spawn_blocking(move || {
            rustle_media::scan::inspect_audio_files(&batch, &config, Some(&cover_cache))
        })
        .await?;

        let mut pending_imports = Vec::new();
        let mut pending_songs = Vec::new();

        for (path, result) in results {
            if state.is_cancelled() {
                let _ = progress_tx.send(ScanProgress::Cancelled);
                return Ok(());
            }

            let current = state.increment_current();
            let path_str = path.to_string_lossy().to_string();
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_string();

            let _ = progress_tx.send(ScanProgress::Processing {
                current,
                total: total_files,
                file_name: file_name.clone(),
            });

            match result {
                Ok(scan_result) => {
                    let cover_path = scan_result
                        .cover_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string());
                    pending_songs.push(NewSong {
                        file_path: path_str,
                        title: scan_result.metadata.title.clone(),
                        artist: scan_result.metadata.artist.clone(),
                        album: scan_result.metadata.album.clone(),
                        duration_secs: scan_result.metadata.duration_secs,
                        track_number: scan_result.metadata.track_number,
                        year: scan_result.metadata.year,
                        genre: scan_result.metadata.genre.clone(),
                        cover_path: cover_path.clone(),
                        file_hash: scan_result.file_hash,
                        file_size: scan_result.file_size as i64,
                        format: Some(scan_result.metadata.format),
                        normalization_gain: scan_result.normalization_gain,
                    });
                    pending_imports.push(PendingImport {
                        path,
                        file_name,
                        title: scan_result.metadata.title,
                        artist: scan_result.metadata.artist,
                        cover_path,
                    });
                }
                Err(error) => {
                    state.increment_skipped();
                    let _ = progress_tx.send(ScanProgress::Skipped {
                        current,
                        total: total_files,
                        file_name,
                        reason: classify_media_error(error),
                    });
                }
            }
        }

        if pending_songs.is_empty() {
            continue;
        }

        if state.is_cancelled() {
            let _ = progress_tx.send(ScanProgress::Cancelled);
            return Ok(());
        }

        match db.upsert_local_songs(pending_songs).await {
            Ok(ids) if ids.len() == pending_imports.len() => {
                for pending in pending_imports {
                    if state.is_cancelled() {
                        let _ = progress_tx.send(ScanProgress::Cancelled);
                        return Ok(());
                    }
                    state.increment_imported();
                    state.push_scanned_path(pending.path);
                    let (_, current, _, _, _) = state.get_stats();
                    let _ = progress_tx.send(ScanProgress::Imported {
                        current,
                        total: total_files,
                        title: pending.title,
                        artist: pending.artist,
                        cover_path: pending.cover_path,
                    });
                }
            }
            Ok(ids) => {
                let message = format!(
                    "batch upsert returned {} ids for {} songs",
                    ids.len(),
                    pending_imports.len()
                );
                report_database_failure(
                    &state,
                    &progress_tx,
                    total_files,
                    pending_imports,
                    message,
                );
            }
            Err(error) => report_database_failure(
                &state,
                &progress_tx,
                total_files,
                pending_imports,
                error.to_string(),
            ),
        }
    }

    let (_, _, imported, skipped, errors) = state.get_stats();
    let _ = progress_tx.send(ScanProgress::Completed {
        imported,
        skipped,
        errors,
        duration_secs: start_time.elapsed().as_secs_f64(),
    });
    Ok(())
}

fn report_database_failure(
    state: &ScanState,
    progress_tx: &ProgressSender,
    total_files: u64,
    pending_imports: Vec<PendingImport>,
    message: String,
) {
    for pending in pending_imports {
        state.increment_errors();
        let (_, current, _, _, _) = state.get_stats();
        let _ = progress_tx.send(ScanProgress::Skipped {
            current,
            total: total_files,
            file_name: pending.file_name,
            reason: SkipReason::DatabaseError(message.clone()),
        });
    }
}
