//! Process-wide tracing initialization, redaction, rotation, and retention.

use std::backtrace::Backtrace;
use std::cell::Cell;
use std::env;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const DEFAULT_FILTER: &str = "info";
const DEFAULT_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_MAX_FILES: usize = 8;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const NON_BLOCKING_BUFFERED_LINES: usize = 4_096;
const LOG_FILE_PREFIX: &str = "rustle-session-";
const LOG_FILE_SUFFIX: &str = ".log";
const CRASH_FILE_PREFIX: &str = "rustle-crash-";
const CRASH_FILE_SUFFIX: &str = ".jsonl";
const MAX_CRASH_FILES: usize = 4;
const MAX_CRASH_FILE_BYTES: u64 = 1024 * 1024;
const MAX_CRASH_TOTAL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PANIC_PAYLOAD_BYTES: usize = 8 * 1024;
const MAX_PANIC_LOCATION_BYTES: usize = 2 * 1024;
const MAX_PANIC_BACKTRACE_BYTES: usize = 512 * 1024;
const TOOLCHAIN_MANIFEST: &str = include_str!("../rust-toolchain.toml");

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
static CRASH_COUNTER: AtomicU64 = AtomicU64::new(1);
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
static RUNTIME_PHASE: AtomicU8 = AtomicU8::new(RuntimePhase::Observability as u8);

thread_local! {
    static IN_PANIC_HOOK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RuntimePhase {
    Observability = 0,
    SingleInstance = 1,
    Platform = 2,
    UiRuntime = 3,
    Application = 4,
    Shutdown = 5,
}

impl RuntimePhase {
    fn current() -> Self {
        match RUNTIME_PHASE.load(Ordering::Relaxed) {
            1 => Self::SingleInstance,
            2 => Self::Platform,
            3 => Self::UiRuntime,
            4 => Self::Application,
            5 => Self::Shutdown,
            _ => Self::Observability,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Observability => "observability",
            Self::SingleInstance => "single_instance",
            Self::Platform => "platform",
            Self::UiRuntime => "ui_runtime",
            Self::Application => "application",
            Self::Shutdown => "shutdown",
        }
    }
}

pub(crate) fn set_runtime_phase(phase: RuntimePhase) {
    RUNTIME_PHASE.store(phase as u8, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy)]
struct RotationPolicy {
    max_file_bytes: u64,
    max_files: usize,
    max_total_bytes: u64,
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_files: DEFAULT_MAX_FILES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

impl RotationPolicy {
    fn validate(self) -> io::Result<Self> {
        if self.max_file_bytes == 0
            || self.max_files == 0
            || self.max_total_bytes < self.max_file_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "log retention limits must be non-zero and total bytes must fit one file",
            ));
        }
        Ok(self)
    }
}

/// Keeps the non-blocking worker alive until after the shutdown event is sent.
pub struct ObservabilityGuard {
    session_id: String,
    sink: SinkKind,
    dropped_lines: Option<ErrorCounter>,
    installed: bool,
    _worker_guard: Option<WorkerGuard>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        if !self.installed {
            return;
        }

        set_runtime_phase(RuntimePhase::Shutdown);
        let dropped_lines = self
            .dropped_lines
            .as_ref()
            .map_or(0, ErrorCounter::dropped_lines);
        if dropped_lines > 0 {
            tracing::warn!(
                event = "observability_queue_overflow",
                session_id = %self.session_id,
                dropped_lines,
                "Observability queue dropped log lines"
            );
        }
        tracing::info!(
            event = "process_shutdown",
            session_id = %self.session_id,
            log_sink = %self.sink,
            dropped_lines,
            "Rustle process is shutting down"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkKind {
    ExplicitDirectory,
    LocalDataDirectory,
    TemporaryDirectory,
    Stderr,
    ExternalSubscriber,
}

impl std::fmt::Display for SinkKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ExplicitDirectory => "explicit_directory",
            Self::LocalDataDirectory => "local_data_directory",
            Self::TemporaryDirectory => "temporary_directory",
            Self::Stderr => "stderr",
            Self::ExternalSubscriber => "external_subscriber",
        })
    }
}

#[derive(Debug)]
struct DirectoryCandidate {
    kind: SinkKind,
    path: PathBuf,
}

#[derive(Debug)]
struct DirectoryFailure {
    kind: SinkKind,
    error_kind: io::ErrorKind,
}

#[derive(Debug)]
struct FileSink {
    kind: SinkKind,
    writer: RotatingFileWriter,
    failures: Vec<DirectoryFailure>,
}

#[derive(Debug)]
struct FilterSelection {
    directive: String,
    invalid_requested_filter: bool,
}

#[derive(Clone)]
enum CrashSink {
    Directory(PathBuf),
    Stderr,
}

#[derive(Clone)]
struct CrashRuntime {
    session_id: String,
    redactor: Arc<Redactor>,
    sink: CrashSink,
}

impl CrashRuntime {
    fn new(session_id: String, redactor: Arc<Redactor>, sink: CrashSink) -> Self {
        Self {
            session_id,
            redactor,
            sink,
        }
    }

    fn record(&self, panic_info: &PanicHookInfo<'_>) {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let payload = panic_payload(panic_info);
        let location = panic_info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown".to_string());
        let backtrace = backtrace_enabled().then(|| Backtrace::force_capture().to_string());
        let record = format_crash_record(
            &self.session_id,
            thread_name,
            &payload,
            &location,
            backtrace.as_deref(),
            &self.redactor,
        );
        self.write(&record);
    }

    fn write(&self, record: &[u8]) {
        match &self.sink {
            CrashSink::Directory(directory) => {
                if write_crash_file(directory, &self.session_id, record).is_err() {
                    write_crash_stderr(record);
                }
            }
            CrashSink::Stderr => write_crash_stderr(record),
        }
    }
}

#[derive(Serialize)]
struct CrashRecord<'a> {
    event: &'static str,
    timestamp_unix_ms: u128,
    session_id: &'a str,
    app_version: &'static str,
    git_commit: &'static str,
    rust_toolchain: &'static str,
    os: &'static str,
    arch: &'static str,
    phase: &'static str,
    thread: &'a str,
    location: &'a str,
    payload: &'a str,
    backtrace: Option<&'a str>,
}

fn install_panic_hook(runtime: CrashRuntime) {
    if PANIC_HOOK_INSTALLED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    std::panic::set_hook(Box::new(move |panic_info| {
        let already_running = IN_PANIC_HOOK.with(|active| active.replace(true));
        if already_running {
            let _ = io::stderr().write_all(b"Rustle panic hook recursion detected\n");
            return;
        }

        run_panic_hook(
            || runtime.record(panic_info),
            || {
                let _ = io::stderr()
                    .write_all(b"Rustle panic hook failed while writing a redacted record\n");
            },
        );
        IN_PANIC_HOOK.with(|active| active.set(false));
    }));
}

fn run_panic_hook<R, F>(record: R, fallback: F)
where
    R: FnOnce(),
    F: FnOnce(),
{
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(record)).is_err() {
        fallback();
    }
}

fn panic_payload(panic_info: &PanicHookInfo<'_>) -> String {
    if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn backtrace_enabled() -> bool {
    env::var("RUST_BACKTRACE")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("off"))
        .unwrap_or(false)
}

fn format_crash_record(
    session_id: &str,
    thread: &str,
    payload: &str,
    location: &str,
    backtrace: Option<&str>,
    redactor: &Redactor,
) -> Vec<u8> {
    let thread = bounded_redacted(redactor, thread, 256);
    let payload = bounded_redacted(redactor, payload, MAX_PANIC_PAYLOAD_BYTES);
    let location = bounded_redacted(redactor, location, MAX_PANIC_LOCATION_BYTES);
    let backtrace =
        backtrace.map(|value| bounded_redacted(redactor, value, MAX_PANIC_BACKTRACE_BYTES));
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let record = CrashRecord {
        event: "panic",
        timestamp_unix_ms,
        session_id,
        app_version: env!("CARGO_PKG_VERSION"),
        git_commit: build_commit(),
        rust_toolchain: pinned_toolchain(),
        os: env::consts::OS,
        arch: env::consts::ARCH,
        phase: RuntimePhase::current().as_str(),
        thread: &thread,
        location: &location,
        payload: &payload,
        backtrace: backtrace.as_deref(),
    };
    serde_json::to_vec(&record).unwrap_or_else(|_| {
        br#"{"event":"panic","payload":"crash record serialization failed"}"#.to_vec()
    })
}

fn bounded_redacted(redactor: &Redactor, input: &str, max_bytes: usize) -> String {
    let redacted = redactor.redact(input);
    truncate_utf8(&redacted, max_bytes).to_string()
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

fn write_crash_file(directory: &Path, session_id: &str, record: &[u8]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let index = CRASH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!(
        "{CRASH_FILE_PREFIX}{session_id}-{index:03}{CRASH_FILE_SUFFIX}"
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let record = truncate_bytes(record, MAX_CRASH_FILE_BYTES.saturating_sub(1) as usize);
    file.write_all(record)?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.sync_data()?;
    prune_owned_crashes(directory, &path)
}

fn truncate_bytes(value: &[u8], max_bytes: usize) -> &[u8] {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && std::str::from_utf8(&value[..end]).is_err() {
        end -= 1;
    }
    &value[..end]
}

fn write_crash_stderr(record: &[u8]) {
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(record);
    let _ = stderr.write_all(b"\n");
    let _ = stderr.flush();
}

fn prune_owned_crashes(directory: &Path, current_path: &Path) -> io::Result<()> {
    let mut crashes = retained_files(directory, is_owned_crash_name)?;
    crashes.sort_by(|left, right| {
        left.modified
            .cmp(&right.modified)
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut total_bytes = crashes.iter().map(|crash| crash.bytes).sum::<u64>();

    while crashes.len() > MAX_CRASH_FILES || total_bytes > MAX_CRASH_TOTAL_BYTES {
        let Some(index) = crashes.iter().position(|crash| crash.path != current_path) else {
            break;
        };
        let removed = crashes.remove(index);
        match fs::remove_file(&removed.path) {
            Ok(()) => total_bytes = total_bytes.saturating_sub(removed.bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                total_bytes = total_bytes.saturating_sub(removed.bytes);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Install Rustle's single process-wide tracing subscriber.
///
/// File-system failures degrade through isolated directories and finally to a
/// redacted stderr sink. A pre-existing external subscriber is treated as an
/// already usable sink instead of causing application startup to panic.
pub fn initialize() -> ObservabilityGuard {
    set_runtime_phase(RuntimePhase::Observability);
    let session_id = new_session_id();
    let redactor = Arc::new(Redactor::new(home_directory()));
    let filter = select_filter();
    let candidates = configured_log_directories();

    match open_first_file_sink(&candidates, &session_id, RotationPolicy::default()) {
        Ok(file_sink) => {
            let FileSink {
                kind,
                writer,
                failures,
            } = file_sink;
            install_panic_hook(CrashRuntime::new(
                session_id.clone(),
                redactor.clone(),
                CrashSink::Directory(writer.directory().to_path_buf()),
            ));
            let (non_blocking, worker_guard) = NonBlockingBuilder::default()
                .buffered_lines_limit(NON_BLOCKING_BUFFERED_LINES)
                .lossy(true)
                .thread_name("rustle-log-writer")
                .finish(writer);
            let dropped_lines = non_blocking.error_counter();
            let file_writer = RedactingMakeWriter::non_blocking(non_blocking, redactor.clone());

            if install_file_subscriber(file_writer, redactor, &filter.directive).is_err() {
                eprintln!(
                    "Rustle tracing subscriber was already installed; using the existing sink"
                );
                return ObservabilityGuard {
                    session_id,
                    sink: SinkKind::ExternalSubscriber,
                    dropped_lines: None,
                    installed: false,
                    _worker_guard: None,
                };
            }

            emit_startup(&session_id, kind, &filter, &failures);
            ObservabilityGuard {
                session_id,
                sink: kind,
                dropped_lines: Some(dropped_lines),
                installed: true,
                _worker_guard: Some(worker_guard),
            }
        }
        Err(failures) => {
            install_panic_hook(CrashRuntime::new(
                session_id.clone(),
                redactor.clone(),
                CrashSink::Stderr,
            ));
            let stderr_writer = RedactingMakeWriter::stderr(redactor);
            if install_stderr_subscriber(stderr_writer, &filter.directive).is_err() {
                eprintln!(
                    "Rustle tracing subscriber was already installed; using the existing sink"
                );
                return ObservabilityGuard {
                    session_id,
                    sink: SinkKind::ExternalSubscriber,
                    dropped_lines: None,
                    installed: false,
                    _worker_guard: None,
                };
            }

            emit_startup(&session_id, SinkKind::Stderr, &filter, &failures);
            ObservabilityGuard {
                session_id,
                sink: SinkKind::Stderr,
                dropped_lines: None,
                installed: true,
                _worker_guard: None,
            }
        }
    }
}

fn emit_startup(
    session_id: &str,
    sink: SinkKind,
    filter: &FilterSelection,
    failures: &[DirectoryFailure],
) {
    tracing::info!(
        event = "process_start",
        session_id,
        app_version = env!("CARGO_PKG_VERSION"),
        git_commit = build_commit(),
        rust_toolchain = pinned_toolchain(),
        os = env::consts::OS,
        arch = env::consts::ARCH,
        log_sink = %sink,
        log_filter = %filter.directive,
        invalid_requested_filter = filter.invalid_requested_filter,
        "Rustle observability initialized"
    );

    for failure in failures {
        tracing::warn!(
            event = "observability_sink_fallback",
            session_id,
            failed_sink = %failure.kind,
            error_kind = ?failure.error_kind,
            selected_sink = %sink,
            "Observability file sink was unavailable"
        );
    }
}

fn install_file_subscriber(
    file_writer: RedactingMakeWriter,
    redactor: Arc<Redactor>,
    filter: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let file_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_writer(file_writer)
        .with_filter(env_filter(filter));

    #[cfg(debug_assertions)]
    {
        let console_layer = tracing_subscriber::fmt::layer()
            .compact()
            .with_ansi(true)
            .with_target(true)
            .with_thread_ids(true)
            .with_thread_names(true)
            .with_writer(RedactingMakeWriter::stderr(redactor))
            .with_filter(env_filter(filter));
        tracing_subscriber::registry()
            .with(file_layer)
            .with(console_layer)
            .try_init()?;
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = redactor;
        tracing_subscriber::registry().with(file_layer).try_init()?;
    }

    Ok(())
}

fn install_stderr_subscriber(
    writer: RedactingMakeWriter,
    filter: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(cfg!(debug_assertions))
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_writer(writer)
        .with_filter(env_filter(filter));
    tracing_subscriber::registry().with(layer).try_init()?;
    Ok(())
}

fn env_filter(directive: &str) -> EnvFilter {
    EnvFilter::try_new(directive).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

fn select_filter() -> FilterSelection {
    match env::var("RUSTLE_LOG") {
        Ok(requested) if EnvFilter::try_new(&requested).is_ok() => FilterSelection {
            directive: requested,
            invalid_requested_filter: false,
        },
        Ok(_) => FilterSelection {
            directive: DEFAULT_FILTER.to_string(),
            invalid_requested_filter: true,
        },
        Err(_) => FilterSelection {
            directive: DEFAULT_FILTER.to_string(),
            invalid_requested_filter: false,
        },
    }
}

fn configured_log_directories() -> Vec<DirectoryCandidate> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("RUSTLE_LOG_DIR").filter(|path| !path.is_empty()) {
        push_unique_candidate(
            &mut candidates,
            DirectoryCandidate {
                kind: SinkKind::ExplicitDirectory,
                path: PathBuf::from(path),
            },
        );
    }
    if let Some(project_dirs) = directories::ProjectDirs::from("com", "rustle", "Rustle") {
        push_unique_candidate(
            &mut candidates,
            DirectoryCandidate {
                kind: SinkKind::LocalDataDirectory,
                path: project_dirs.data_local_dir().join("logs"),
            },
        );
    }
    push_unique_candidate(
        &mut candidates,
        DirectoryCandidate {
            kind: SinkKind::TemporaryDirectory,
            path: env::temp_dir().join("rustle").join("logs"),
        },
    );
    candidates
}

#[cfg(feature = "diagnostics")]
pub(crate) fn diagnostic_directories() -> Vec<PathBuf> {
    configured_log_directories()
        .into_iter()
        .map(|candidate| candidate.path)
        .collect()
}

fn push_unique_candidate(candidates: &mut Vec<DirectoryCandidate>, candidate: DirectoryCandidate) {
    if !candidates
        .iter()
        .any(|existing| existing.path == candidate.path)
    {
        candidates.push(candidate);
    }
}

fn open_first_file_sink(
    candidates: &[DirectoryCandidate],
    session_id: &str,
    policy: RotationPolicy,
) -> Result<FileSink, Vec<DirectoryFailure>> {
    let mut failures = Vec::new();
    for candidate in candidates {
        match RotatingFileWriter::new(&candidate.path, session_id, policy) {
            Ok(writer) => {
                return Ok(FileSink {
                    kind: candidate.kind,
                    writer,
                    failures,
                });
            }
            Err(error) => failures.push(DirectoryFailure {
                kind: candidate.kind,
                error_kind: error.kind(),
            }),
        }
    }
    Err(failures)
}

fn new_session_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:x}-{:x}-{sequence:x}", std::process::id())
}

pub(crate) fn build_commit() -> &'static str {
    option_env!("RUSTLE_BUILD_COMMIT")
        .or(option_env!("GITHUB_SHA"))
        .unwrap_or("unknown")
}

pub(crate) fn pinned_toolchain() -> &'static str {
    TOOLCHAIN_MANIFEST
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "channel").then(|| value.trim().trim_matches('"'))
        })
        .unwrap_or("unknown")
}

fn home_directory() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(feature = "diagnostics")]
pub(crate) fn diagnostic_redactor() -> Redactor {
    Redactor::new(home_directory())
}

#[derive(Debug)]
struct RotatingFileWriter {
    directory: PathBuf,
    session_id: String,
    policy: RotationPolicy,
    file: File,
    current_path: PathBuf,
    current_len: u64,
    next_index: u32,
}

impl RotatingFileWriter {
    fn new(directory: &Path, session_id: &str, policy: RotationPolicy) -> io::Result<Self> {
        let policy = policy.validate()?;
        fs::create_dir_all(directory)?;
        let (file, current_path, next_index) = open_next_log_file(directory, session_id, 0)?;
        let mut writer = Self {
            directory: directory.to_path_buf(),
            session_id: session_id.to_string(),
            policy,
            file,
            current_path,
            current_len: 0,
            next_index,
        };
        writer.prune()?;
        Ok(writer)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.sync_data()?;
        let (file, path, next_index) =
            open_next_log_file(&self.directory, &self.session_id, self.next_index)?;
        self.file = file;
        self.current_path = path;
        self.current_len = 0;
        self.next_index = next_index;
        self.prune()
    }

    fn directory(&self) -> &Path {
        &self.directory
    }

    fn prune(&mut self) -> io::Result<()> {
        prune_owned_logs(&self.directory, &self.current_path, self.policy)
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.current_len > 0
            && self.current_len.saturating_add(buffer.len() as u64) > self.policy.max_file_bytes
        {
            self.rotate()?;
        }
        let available = self.policy.max_file_bytes.saturating_sub(self.current_len);
        let write_len = usize::try_from(available)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let written = self.file.write(&buffer[..write_len])?;
        self.current_len = self.current_len.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn open_next_log_file(
    directory: &Path,
    session_id: &str,
    mut index: u32,
) -> io::Result<(File, PathBuf, u32)> {
    loop {
        let path = directory.join(format!(
            "{LOG_FILE_PREFIX}{session_id}-{index:03}{LOG_FILE_SUFFIX}"
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path, index.saturating_add(1))),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                index = index
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("exhausted observability log file sequence"))?;
            }
            Err(error) => return Err(error),
        }
    }
}

#[derive(Debug)]
struct RetainedLog {
    path: PathBuf,
    modified: SystemTime,
    bytes: u64,
}

fn retained_files(
    directory: &Path,
    is_owned: fn(&std::ffi::OsStr) -> bool,
) -> io::Result<Vec<RetainedLog>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() || !is_owned(&entry.file_name()) {
            continue;
        }
        let metadata = entry.metadata()?;
        files.push(RetainedLog {
            path: entry.path(),
            modified: metadata.modified().unwrap_or(UNIX_EPOCH),
            bytes: metadata.len(),
        });
    }
    Ok(files)
}

fn prune_owned_logs(
    directory: &Path,
    current_path: &Path,
    policy: RotationPolicy,
) -> io::Result<()> {
    let mut logs = retained_files(directory, is_owned_log_name)?;

    logs.sort_by(|left, right| {
        left.modified
            .cmp(&right.modified)
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut total_bytes = logs.iter().map(|log| log.bytes).sum::<u64>();

    while logs.len() > policy.max_files || total_bytes > policy.max_total_bytes {
        let Some(index) = logs.iter().position(|log| log.path != current_path) else {
            break;
        };
        let removed = logs.remove(index);
        match fs::remove_file(&removed.path) {
            Ok(()) => total_bytes = total_bytes.saturating_sub(removed.bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                total_bytes = total_bytes.saturating_sub(removed.bytes);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn is_owned_log_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(stem) = name
        .strip_prefix(LOG_FILE_PREFIX)
        .and_then(|name| name.strip_suffix(LOG_FILE_SUFFIX))
    else {
        return false;
    };
    let Some((session_id, index)) = stem.rsplit_once('-') else {
        return false;
    };
    !session_id.is_empty()
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        && index.len() >= 3
        && index.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn is_owned_crash_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(stem) = name
        .strip_prefix(CRASH_FILE_PREFIX)
        .and_then(|name| name.strip_suffix(CRASH_FILE_SUFFIX))
    else {
        return false;
    };
    let Some((session_id, index)) = stem.rsplit_once('-') else {
        return false;
    };
    !session_id.is_empty()
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        && index.len() >= 3
        && index.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug)]
pub(crate) struct Redactor {
    home: Option<String>,
}

impl Redactor {
    pub(crate) fn new(home: Option<PathBuf>) -> Self {
        Self {
            home: home.map(|path| path.to_string_lossy().into_owned()),
        }
    }

    pub(crate) fn redact(&self, input: &str) -> String {
        let mut output = input.to_string();
        if let Some(home) = self.home.as_deref().filter(|home| !home.is_empty()) {
            output = replace_ascii_case_insensitive(&output, home, "<user-home>");
        }
        output = redact_url_queries(output);
        for key in [
            "authorization",
            "cookie",
            "set-cookie",
            "music_u",
            "__csrf",
            "csrf_token",
            "token",
            "access_token",
            "refresh_token",
            "session_token",
            "device_secret",
            "password",
            "codekey",
            "unikey",
        ] {
            output = redact_assignment(output, key);
        }
        redact_absolute_paths(output)
    }
}

fn replace_ascii_case_insensitive(input: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return input.to_string();
    }
    let mut output = input.to_string();
    let needle_lower = needle.to_ascii_lowercase();
    let mut search_from = 0;
    loop {
        let lower = output.to_ascii_lowercase();
        let Some(relative) = lower[search_from..].find(&needle_lower) else {
            break;
        };
        let start = search_from + relative;
        let end = start + needle.len();
        output.replace_range(start..end, replacement);
        search_from = start + replacement.len();
    }
    output
}

fn redact_url_queries(mut input: String) -> String {
    let mut search_from = 0;
    loop {
        let lower = input.to_ascii_lowercase();
        let http = lower[search_from..].find("http://");
        let https = lower[search_from..].find("https://");
        let relative = match (http, https) {
            (Some(left), Some(right)) => left.min(right),
            (Some(value), None) | (None, Some(value)) => value,
            (None, None) => break,
        };
        let url_start = search_from + relative;
        let url_end = find_delimiter(&input, url_start, is_url_delimiter);
        let Some(query_relative) = input[url_start..url_end].find('?') else {
            search_from = url_end;
            continue;
        };
        let query_start = url_start + query_relative + 1;
        input.replace_range(query_start..url_end, "<redacted>");
        search_from = query_start + "<redacted>".len();
    }
    input
}

fn redact_assignment(mut input: String, key: &str) -> String {
    let key_lower = key.to_ascii_lowercase();
    let mut search_from = 0;
    loop {
        let lower = input.to_ascii_lowercase();
        let Some(relative) = lower[search_from..].find(&key_lower) else {
            break;
        };
        let key_start = search_from + relative;
        let key_end = key_start + key.len();
        if !has_ascii_word_boundaries(&input, key_start, key_end) {
            search_from = key_end;
            continue;
        }

        let bytes = input.as_bytes();
        let mut cursor = key_end;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if !matches!(bytes.get(cursor), Some(b':' | b'=')) {
            search_from = key_end;
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }

        let quote = bytes
            .get(cursor)
            .copied()
            .filter(|byte| matches!(byte, b'\'' | b'"'));
        if quote.is_some() {
            cursor += 1;
        }
        let value_start = cursor;
        let value_end = if let Some(quote) = quote {
            input[value_start..]
                .find(char::from(quote))
                .map_or(input.len(), |relative| value_start + relative)
        } else if key.eq_ignore_ascii_case("authorization")
            && lower[value_start..].starts_with("bearer ")
        {
            find_delimiter(
                &input,
                value_start + "bearer ".len(),
                is_assignment_delimiter,
            )
        } else {
            find_delimiter(&input, value_start, is_assignment_delimiter)
        };

        if value_start == value_end {
            search_from = key_end;
            continue;
        }
        input.replace_range(value_start..value_end, "<redacted>");
        search_from = value_start + "<redacted>".len();
    }
    input
}

fn has_ascii_word_boundaries(input: &str, start: usize, end: usize) -> bool {
    let bytes = input.as_bytes();
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    bytes
        .get(start.wrapping_sub(1))
        .is_none_or(|byte| !is_word(*byte))
        && bytes.get(end).is_none_or(|byte| !is_word(*byte))
}

fn redact_absolute_paths(mut input: String) -> String {
    let mut search_from = 0;
    while let Some(start) = find_path_start(&input, search_from) {
        let end = find_delimiter(&input, start, is_path_delimiter);
        input.replace_range(start..end, "<path>");
        search_from = start + "<path>".len();
    }
    input
}

fn find_path_start(input: &str, from: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    for index in from..bytes.len() {
        if index + 2 < bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'\\' | b'/')
            && index
                .checked_sub(1)
                .and_then(|previous| bytes.get(previous))
                .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        {
            return Some(index);
        }
        let remaining = &bytes[index..];
        if remaining.starts_with(b"<user-home>\\") || remaining.starts_with(b"<user-home>/") {
            return Some(index + "<user-home>".len());
        }
        if [
            b"file://".as_slice(),
            b"/home/",
            b"/Users/",
            b"/mnt/",
            b"/media/",
            b"/Volumes/",
        ]
        .iter()
        .any(|prefix| remaining.starts_with(prefix))
        {
            return Some(index);
        }
    }
    None
}

fn find_delimiter(input: &str, start: usize, delimiter: fn(u8) -> bool) -> usize {
    input.as_bytes()[start..]
        .iter()
        .position(|byte| delimiter(*byte))
        .map_or(input.len(), |relative| start + relative)
}

fn is_url_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b')' | b']' | b'}' | b',')
}

fn is_assignment_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b',' | b'}' | b']')
}

fn is_path_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b',' | b';' | b')' | b']' | b'}')
}

#[derive(Clone)]
struct RedactingMakeWriter {
    sink: EventSink,
    redactor: Arc<Redactor>,
}

impl RedactingMakeWriter {
    fn non_blocking(writer: NonBlocking, redactor: Arc<Redactor>) -> Self {
        Self {
            sink: EventSink::NonBlocking(writer),
            redactor,
        }
    }

    fn stderr(redactor: Arc<Redactor>) -> Self {
        Self {
            sink: EventSink::Stderr,
            redactor,
        }
    }
}

impl<'writer> MakeWriter<'writer> for RedactingMakeWriter {
    type Writer = RedactedEventWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        RedactedEventWriter {
            sink: self.sink.clone(),
            redactor: self.redactor.clone(),
            buffer: Vec::with_capacity(512),
        }
    }
}

#[derive(Clone)]
enum EventSink {
    NonBlocking(NonBlocking),
    Stderr,
}

struct RedactedEventWriter {
    sink: EventSink,
    redactor: Arc<Redactor>,
    buffer: Vec<u8>,
}

impl Write for RedactedEventWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RedactedEventWriter {
    fn drop(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let formatted = String::from_utf8_lossy(&self.buffer);
        let redacted = self.redactor.redact(&formatted);
        match &mut self.sink {
            EventSink::NonBlocking(writer) => {
                let _ = writer.write_all(redacted.as_bytes());
            }
            EventSink::Stderr => {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_all(redacted.as_bytes());
                let _ = stderr.flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = env::temp_dir().join(format!(
                "rustle-observability-{name}-{}-{}",
                std::process::id(),
                SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("create observability test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn redaction_covers_credentials_urls_and_platform_paths() {
        let redactor = Redactor::new(Some(PathBuf::from(r"C:\Users\Alice")));
        let input = concat!(
            "cookie=MUSIC_U=secret;__csrf=hidden ",
            "Authorization: Bearer abc123 ",
            "access_token='token-value' codekey=qr-secret ",
            "url=https://example.test/song?id=7&token=signed ",
            r#"windows=C:\Users\Alice\Music\private.flac "#,
            "linux=/home/alice/Music/private.flac ",
            "mount=/mnt/library/private.flac"
        );
        let output = redactor.redact(input);

        for secret in [
            "secret",
            "hidden",
            "abc123",
            "token-value",
            "qr-secret",
            "id=7",
            "Alice",
            "private.flac",
        ] {
            assert!(
                !output.contains(secret),
                "redaction leaked {secret}: {output}"
            );
        }
        assert!(output.contains("https://example.test/song?<redacted>"));
        assert!(output.contains("<user-home>"));
        assert!(output.contains("<path>"));
    }

    #[test]
    fn redaction_handles_unicode_text_around_paths() {
        let redactor = Redactor::new(None);
        let output = redactor.redact("曲目 路径=/home/用户/音乐/歌曲.flac 标题=你好");

        assert_eq!(output, "曲目 路径=<path> 标题=你好");
    }

    #[test]
    fn rotation_enforces_file_count_total_bytes_and_namespace_safety() {
        let directory = TestDirectory::new("rotation");
        let unrelated = directory.0.join("user-notes.log");
        fs::write(&unrelated, b"keep me").unwrap();
        let policy = RotationPolicy {
            max_file_bytes: 24,
            max_files: 3,
            max_total_bytes: 72,
        };
        let mut writer = RotatingFileWriter::new(&directory.0, "abc123", policy).unwrap();
        for value in [
            b"11111111111111111111\n",
            b"22222222222222222222\n",
            b"33333333333333333333\n",
            b"44444444444444444444\n",
        ] {
            writer.write_all(value).unwrap();
        }
        writer.flush().unwrap();
        drop(writer);

        let owned = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_owned_log_name(&entry.file_name()))
            .collect::<Vec<_>>();
        let total = owned
            .iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>();
        assert!(owned.len() <= policy.max_files);
        assert!(total <= policy.max_total_bytes);
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep me");
    }

    #[test]
    fn oversized_write_is_split_without_exceeding_the_file_limit() {
        let directory = TestDirectory::new("oversized-write");
        let policy = RotationPolicy {
            max_file_bytes: 8,
            max_files: 4,
            max_total_bytes: 32,
        };
        let mut writer = RotatingFileWriter::new(&directory.0, "abc123", policy).unwrap();
        writer.write_all(b"0123456789abcdef").unwrap();
        writer.flush().unwrap();
        drop(writer);

        let sizes = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_owned_log_name(&entry.file_name()))
            .map(|entry| entry.metadata().unwrap().len())
            .collect::<Vec<_>>();
        assert_eq!(sizes.iter().sum::<u64>(), 16);
        assert!(sizes.iter().all(|size| *size <= policy.max_file_bytes));
    }

    #[test]
    fn unusable_preferred_directory_falls_back_without_panicking() {
        let directory = TestDirectory::new("fallback");
        let blocking_file = directory.0.join("not-a-directory");
        fs::write(&blocking_file, b"file").unwrap();
        let fallback = directory.0.join("fallback");
        let candidates = [
            DirectoryCandidate {
                kind: SinkKind::ExplicitDirectory,
                path: blocking_file.join("logs"),
            },
            DirectoryCandidate {
                kind: SinkKind::TemporaryDirectory,
                path: fallback,
            },
        ];

        let sink = open_first_file_sink(&candidates, "abc123", RotationPolicy::default())
            .expect("fallback sink");
        assert_eq!(sink.kind, SinkKind::TemporaryDirectory);
        assert_eq!(sink.failures.len(), 1);
        assert_eq!(sink.failures[0].kind, SinkKind::ExplicitDirectory);
    }

    #[test]
    fn owned_log_namespace_is_strict() {
        assert!(is_owned_log_name(std::ffi::OsStr::new(
            "rustle-session-a1-b2-000.log"
        )));
        assert!(!is_owned_log_name(std::ffi::OsStr::new("rustle.log")));
        assert!(!is_owned_log_name(std::ffi::OsStr::new(
            "rustle-session-notes-000.log"
        )));
        assert!(!is_owned_log_name(std::ffi::OsStr::new(
            "rustle-session-a1-b2.log.bak"
        )));
        assert!(is_owned_crash_name(std::ffi::OsStr::new(
            "rustle-crash-a1-b2-001.jsonl"
        )));
        assert!(!is_owned_crash_name(std::ffi::OsStr::new(
            "rustle-crash-notes-001.jsonl"
        )));
    }

    #[test]
    fn crash_record_is_bounded_json_and_reuses_privacy_redaction() {
        let redactor = Redactor::new(Some(PathBuf::from(r"C:\Users\Alice")));
        let payload = format!(
            "token=hidden url=https://example.test/play?id=7 path={} {}",
            r"C:\Users\Alice\Music\private.flac",
            "x".repeat(MAX_PANIC_PAYLOAD_BYTES * 2)
        );
        let record = format_crash_record(
            "abc123",
            "worker-secret",
            &payload,
            r"C:\Users\Alice\src\main.rs:7:9",
            Some(r"frame C:\Users\Alice\src\main.rs"),
            &redactor,
        );
        let value: serde_json::Value = serde_json::from_slice(&record).unwrap();
        let serialized = String::from_utf8(record).unwrap();

        for secret in ["hidden", "id=7", "Alice", "private.flac"] {
            assert!(!serialized.contains(secret), "crash record leaked {secret}");
        }
        assert_eq!(value["event"], "panic");
        assert_eq!(value["session_id"], "abc123");
        assert!(value["payload"].as_str().unwrap().len() <= MAX_PANIC_PAYLOAD_BYTES);
    }

    #[test]
    fn crash_retention_is_bounded_and_preserves_unrelated_files() {
        let directory = TestDirectory::new("crash-retention");
        let unrelated = directory.0.join("user-crash-notes.jsonl");
        fs::write(&unrelated, b"keep me").unwrap();

        for index in 0..6 {
            let session = format!("abc{index:x}");
            write_crash_file(&directory.0, &session, b"{\"event\":\"panic\"}").unwrap();
        }

        let crashes = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_owned_crash_name(&entry.file_name()))
            .collect::<Vec<_>>();
        let total_bytes = crashes
            .iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>();
        assert!(crashes.len() <= MAX_CRASH_FILES);
        assert!(total_bytes <= MAX_CRASH_TOTAL_BYTES);
        assert_eq!(fs::read(unrelated).unwrap(), b"keep me");
    }

    #[test]
    fn panic_fixture_entrypoint() {
        let Some(directory) = env::var_os("RUSTLE_PANIC_FIXTURE_DIR") else {
            return;
        };
        install_panic_hook(CrashRuntime::new(
            "abc123".to_string(),
            Arc::new(Redactor::new(None)),
            CrashSink::Directory(PathBuf::from(directory)),
        ));
        set_runtime_phase(RuntimePhase::Application);
        panic!("token=hidden url=https://example.test/?signed=yes path=/home/alice/private.flac");
    }

    #[test]
    fn panic_hook_subprocess_writes_a_redacted_synchronous_record() {
        let directory = TestDirectory::new("panic-subprocess");
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "observability::tests::panic_fixture_entrypoint",
                "--test-threads=1",
            ])
            .env("RUSTLE_PANIC_FIXTURE_DIR", &directory.0)
            .env("RUST_BACKTRACE", "0")
            .output()
            .unwrap();
        assert!(!output.status.success());

        let record_path = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| is_owned_crash_name(&entry.file_name()))
            .expect("panic hook crash record")
            .path();
        let record = fs::read_to_string(record_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(record.trim()).unwrap();
        assert_eq!(value["event"], "panic");
        assert_eq!(value["phase"], "application");
        for secret in ["hidden", "signed=yes", "alice", "private.flac"] {
            assert!(!record.contains(secret), "panic hook leaked {secret}");
        }
    }

    #[test]
    fn panic_hook_failure_runs_the_fixed_fallback() {
        let fallback_ran = AtomicBool::new(false);
        run_panic_hook(
            || panic!("sensitive panic-hook implementation failure"),
            || fallback_ran.store(true, Ordering::Relaxed),
        );
        assert!(fallback_ran.load(Ordering::Relaxed));
    }

    #[test]
    fn filter_selection_rejects_invalid_directives() {
        assert!(EnvFilter::try_new("info,rustle::audio=debug").is_ok());
        assert!(EnvFilter::try_new("not a valid filter [").is_err());
    }

    #[test]
    fn pinned_toolchain_metadata_comes_from_the_workspace_manifest() {
        assert_eq!(pinned_toolchain(), "1.98.0");
        assert!(!new_session_id().is_empty());
    }
}
