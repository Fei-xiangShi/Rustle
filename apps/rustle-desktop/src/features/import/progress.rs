//! Scan progress tracking and reporting

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::mpsc;

/// Progress update sent during scanning
#[derive(Debug, Clone)]
pub enum ScanProgress {
    /// Scanning started
    Started { total_files: u64 },
    /// Currently processing a file
    Processing {
        current: u64,
        total: u64,
        file_name: String,
    },
    /// A file was successfully imported
    Imported {
        current: u64,
        total: u64,
        title: String,
        artist: String,
        cover_path: Option<String>,
    },
    /// A file was skipped because it could not be imported
    Skipped {
        current: u64,
        total: u64,
        file_name: String,
        reason: SkipReason,
    },
    /// Scanning completed
    Completed {
        imported: u64,
        skipped: u64,
        errors: u64,
        duration_secs: f64,
    },
    /// Scanning was cancelled
    Cancelled,
    /// An error occurred
    Error(String),
}

/// Reason why a file was skipped
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// File is corrupted or unreadable
    Corrupted,
    /// Not a valid audio file
    NotAudioFile,
    /// File is empty (0 bytes)
    EmptyFile,
    /// Failed to read metadata
    MetadataError(String),
    /// Failed to write the imported song to the local database
    DatabaseError(String),
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::Corrupted => write!(f, "文件无法读取或已损坏"),
            SkipReason::NotAudioFile => write!(f, "不是支持的音频文件"),
            SkipReason::EmptyFile => write!(f, "空文件"),
            SkipReason::MetadataError(message) => write!(f, "读取元数据失败：{}", message),
            SkipReason::DatabaseError(message) => write!(f, "写入数据库失败：{}", message),
        }
    }
}

impl std::error::Error for SkipReason {}

/// Shared state for tracking scan progress
#[derive(Debug)]
pub struct ScanState {
    /// Total files to scan
    pub total: AtomicU64,
    /// Current file being processed
    pub current: AtomicU64,
    /// Successfully imported count
    pub imported: AtomicU64,
    /// Skipped count
    pub skipped: AtomicU64,
    /// Error count
    pub errors: AtomicU64,
    /// Whether scan was cancelled
    pub cancelled: AtomicBool,
    /// All scanned file paths (for playlist creation)
    pub scanned_paths: std::sync::Mutex<Option<Vec<std::path::PathBuf>>>,
}

impl ScanState {
    pub fn new() -> Self {
        Self {
            total: AtomicU64::new(0),
            current: AtomicU64::new(0),
            imported: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
            scanned_paths: std::sync::Mutex::new(None),
        }
    }

    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::SeqCst);
    }

    pub fn increment_current(&self) -> u64 {
        self.current.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn increment_imported(&self) {
        self.imported.fetch_add(1, Ordering::SeqCst);
    }

    pub fn increment_skipped(&self) {
        self.skipped.fetch_add(1, Ordering::SeqCst);
    }

    pub fn increment_errors(&self) {
        self.errors.fetch_add(1, Ordering::SeqCst);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn get_stats(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.total.load(Ordering::SeqCst),
            self.current.load(Ordering::SeqCst),
            self.imported.load(Ordering::SeqCst),
            self.skipped.load(Ordering::SeqCst),
            self.errors.load(Ordering::SeqCst),
        )
    }

    /// Set the list of all scanned file paths
    pub fn set_scanned_paths(&self, paths: Vec<std::path::PathBuf>) {
        if let Ok(mut guard) = self.scanned_paths.lock() {
            *guard = Some(paths);
        }
    }

    /// Add a successfully imported file path for playlist creation.
    pub fn push_scanned_path(&self, path: std::path::PathBuf) {
        if let Ok(mut guard) = self.scanned_paths.lock() {
            guard.get_or_insert_with(Vec::new).push(path);
        }
    }

    /// Get the list of all scanned file paths
    pub fn get_scanned_paths(&self) -> Option<Vec<std::path::PathBuf>> {
        self.scanned_paths
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }
}

impl Default for ScanState {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle for controlling and monitoring a scan operation
#[derive(Debug, Clone)]
pub struct ScanHandle {
    state: Arc<ScanState>,
}

impl ScanHandle {
    pub fn new(state: Arc<ScanState>) -> Self {
        Self { state }
    }

    /// Cancel the ongoing scan
    pub fn cancel(&self) {
        self.state.cancel();
    }
}

/// Progress sender for reporting scan updates
pub type ProgressSender = mpsc::UnboundedSender<ScanProgress>;
/// Progress receiver for receiving scan updates
pub type ProgressReceiver = mpsc::UnboundedReceiver<ScanProgress>;

/// Create a new progress channel
pub fn progress_channel() -> (ProgressSender, ProgressReceiver) {
    mpsc::unbounded_channel()
}
