//! Streaming buffer and download utilities for audio playback
//!
//! This module provides:
//! - `SharedBuffer`: Thread-safe memory buffer for streaming audio
//! - `StreamingBuffer`: Read+Seek wrapper for rodio's Decoder
//! - `StreamingEvent`: Download progress events
//! - `start_buffer_download()`: Unified download function
//!
//! Download thread writes to buffer, playback thread reads from it.
//! Blocks when data is not yet available.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use crate::application::ports::cache::{AudioCacheStore, PublishOutcome};
use crate::audio::identity::{PlaybackContext, PreloadIdentity};
use crate::audio::player::{PlaybackError, PlaybackResult};
use parking_lot::{Condvar, Mutex, RwLock};

// ============ Constants ============

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
/// Bound disk-only cache work so active decoder refill is reconsidered
/// frequently even on high-bitrate sources.
const CACHE_BACKFILL_CHUNK_BYTES: u64 = 512 * KIB;

/// One bitrate-aware policy owns every decoder-visible streaming threshold.
///
/// The cache file length never determines retained memory. Bitrate only
/// converts safe playback durations into bounded byte watermarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingBufferPolicy {
    low_water_mark_bytes: u64,
    high_water_mark_bytes: u64,
    range_chunk_bytes: u64,
    capacity_bytes: usize,
}

impl StreamingBufferPolicy {
    const DEFAULT_BITRATE_BPS: u64 = 2_000_000;
    const LOW_SECONDS: u64 = 2;
    const HIGH_SECONDS: u64 = 8;
    const CHUNK_SECONDS: u64 = 2;
    const MIN_LOW_BYTES: u64 = 512 * KIB;
    const MAX_LOW_BYTES: u64 = 4 * MIB;
    const MIN_HIGH_BYTES: u64 = 2 * MIB;
    const MAX_HIGH_BYTES: u64 = 10 * MIB;
    const MIN_CHUNK_BYTES: u64 = 512 * KIB;
    const MAX_CHUNK_BYTES: u64 = 4 * MIB;
    const MAX_CAPACITY_BYTES: u64 = 16 * MIB;

    pub fn from_bitrate(bitrate_bps: Option<u32>) -> Self {
        let bitrate_bps = bitrate_bps
            .filter(|bitrate| *bitrate > 0)
            .map(u64::from)
            .unwrap_or(Self::DEFAULT_BITRATE_BPS);
        let bytes_per_second = bitrate_bps.div_ceil(8);
        let low_water_mark_bytes = bytes_per_second
            .saturating_mul(Self::LOW_SECONDS)
            .clamp(Self::MIN_LOW_BYTES, Self::MAX_LOW_BYTES);
        let range_chunk_bytes = bytes_per_second
            .saturating_mul(Self::CHUNK_SECONDS)
            .clamp(Self::MIN_CHUNK_BYTES, Self::MAX_CHUNK_BYTES);
        let high_water_mark_bytes = bytes_per_second.saturating_mul(Self::HIGH_SECONDS).clamp(
            Self::MIN_HIGH_BYTES.max(low_water_mark_bytes.saturating_add(1)),
            Self::MAX_HIGH_BYTES,
        );
        let capacity_bytes = high_water_mark_bytes
            .saturating_add(range_chunk_bytes)
            .min(Self::MAX_CAPACITY_BYTES) as usize;

        Self {
            low_water_mark_bytes,
            high_water_mark_bytes,
            range_chunk_bytes,
            capacity_bytes,
        }
    }

    pub fn low_water_mark_bytes(self) -> u64 {
        self.low_water_mark_bytes
    }

    pub fn high_water_mark_bytes(self) -> u64 {
        self.high_water_mark_bytes
    }

    pub fn range_chunk_bytes(self) -> u64 {
        self.range_chunk_bytes
    }

    pub fn capacity_bytes(self) -> usize {
        self.capacity_bytes
    }

    pub fn should_refill(self, buffered_ahead: u64) -> bool {
        buffered_ahead < self.high_water_mark_bytes
    }

    pub fn should_enter_buffering(
        self,
        buffered_ahead: u64,
        source_has_all_required_bytes: bool,
    ) -> bool {
        buffered_ahead < self.low_water_mark_bytes && !source_has_all_required_bytes
    }

    pub fn can_start_or_resume(
        self,
        buffered_ahead: u64,
        source_has_all_required_bytes: bool,
    ) -> bool {
        source_has_all_required_bytes || buffered_ahead >= self.high_water_mark_bytes
    }
}

impl Default for StreamingBufferPolicy {
    fn default() -> Self {
        Self::from_bitrate(None)
    }
}

/// Maximum prefix read while identifying a finalized cache file.
const FORMAT_DETECTION_PREFIX_BYTES: usize = 64 * 1024;

/// Network deadlines are explicit so CDN or socket stalls cannot become an
/// unbounded decoder-liveness dependency.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Maximum time a concrete decoder demand may observe no retained-window byte
/// progress while the coordinator still claims to be refillable.
const DECODER_DEMAND_STALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Recoverable Range transport/body failures are retried at most this many
/// times after the initial attempt.
const MAX_RANGE_RETRIES: u8 = 3;

fn range_retry_backoff(retries_completed: u8) -> Option<Duration> {
    (retries_completed < MAX_RANGE_RETRIES)
        .then(|| Duration::from_millis(100 * u64::from(retries_completed + 1)))
}

/// Valid audio extensions for URL parsing
#[cfg(test)]
const VALID_AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "aac", "ogg", "wav", "opus"];

// ============ Format Detection Helpers ============

/// Extract audio file extension from URL path
///
/// # Example
/// ```
/// let ext = extract_extension_from_url("http://example.com/song.flac?token=xxx");
/// assert_eq!(ext, Some("flac".to_string()));
/// ```
#[cfg(test)]
pub fn extract_extension_from_url(url: &str) -> Option<String> {
    // Parse URL and get path
    let url_parsed = reqwest::Url::parse(url).ok()?;
    let path = url_parsed.path();

    // Get the last segment (filename)
    let filename = path.rsplit('/').next()?;

    // Extract extension
    let ext = filename.rsplit('.').next()?.to_lowercase();

    // Validate it's a known audio extension
    if VALID_AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        Some(ext)
    } else {
        None
    }
}

/// Map Content-Type header to file extension
///
/// # Example
/// ```
/// let ext = content_type_to_extension("audio/flac");
/// assert_eq!(ext, Some("flac".to_string()));
/// ```
#[cfg(test)]
pub fn content_type_to_extension(content_type: &str) -> Option<String> {
    // Extract MIME type (ignore parameters like charset)
    let mime = content_type.split(';').next()?.trim().to_lowercase();

    match mime.as_str() {
        "audio/mpeg" | "audio/mp3" => Some("mp3".to_string()),
        "audio/flac" | "audio/x-flac" => Some("flac".to_string()),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => Some("m4a".to_string()),
        "audio/ogg" | "audio/vorbis" | "audio/opus" => Some("ogg".to_string()),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav".to_string()),
        _ => None,
    }
}

/// Parsed byte range advertised by a `Content-Range` response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    end: u64,
    total: u64,
}

fn parse_content_range(value: &str) -> Option<ContentRange> {
    let mut parts = value.split_whitespace();
    let unit = parts.next()?;
    let range = parts.next()?;
    if parts.next().is_some() || !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (bounds, total) = range.split_once('/')?;
    let (start, end) = bounds.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = end.parse::<u64>().ok()?;
    let total = total.parse::<u64>().ok()?;
    if start > end || total == 0 || end >= total {
        return None;
    }
    Some(ContentRange { start, end, total })
}

fn validate_range_response(
    response: &reqwest::Response,
    expected_start: u64,
    expected_end: u64,
) -> PlaybackResult<ContentRange> {
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(PlaybackError::UnsupportedStreaming(format!(
            "expected HTTP 206, got {}",
            response.status()
        )));
    }
    let range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
        .ok_or_else(|| PlaybackError::UnsupportedStreaming("invalid Content-Range".to_string()))?;
    if range.start != expected_start || range.end != expected_end {
        return Err(PlaybackError::UnsupportedStreaming(format!(
            "unexpected range {}-{}, expected {}-{}",
            range.start, range.end, expected_start, expected_end
        )));
    }
    let expected_length = expected_end
        .saturating_sub(expected_start)
        .saturating_add(1);
    if response.content_length() != Some(expected_length) {
        return Err(PlaybackError::UnsupportedStreaming(
            "invalid Range response length".to_string(),
        ));
    }
    Ok(range)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamingIdentity {
    Playback(PlaybackContext),
    Preload(PreloadIdentity),
}

/// Stable identity for a persisted audio cache entry and its in-flight
/// coordinator. Requested quality is deliberately excluded: the server's
/// actual returned quality determines the bytes and cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioCacheKey {
    pub song_id: u64,
    pub actual_quality: crate::domain::audio::QualityLevel,
}

#[derive(Debug, Clone)]
pub enum StreamingEventKind {
    /// Enough data downloaded, playback can start.
    Playable,
    /// Contiguous disk-cache progress update (cached_bytes, total_bytes).
    CacheProgress(u64, u64),
    /// Remote bytes have all been received; persistence is not implied.
    DownloadComplete,
    /// Download and formal cache publication both completed.
    Complete,
    /// Final cache path after atomic rename.
    CacheFinalized(PathBuf),
    /// Cache persistence failed; ring playback may still continue.
    CacheFinalizationFailed(String),
    /// Download error.
    Error(PlaybackError),
}

#[derive(Debug, Clone)]
pub struct StreamingEvent {
    pub identity: StreamingIdentity,
    pub kind: StreamingEventKind,
}

impl StreamingEvent {
    pub fn new(identity: StreamingIdentity, kind: StreamingEventKind) -> Self {
        Self { identity, kind }
    }
}

impl StreamingIdentity {
    pub fn is_cancelled(&self) -> bool {
        match self {
            Self::Playback(context) => context.cancellation.is_cancelled(),
            Self::Preload(identity) => identity.cancellation.is_cancelled(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BufferEvent {
    CacheAdvanced { cached: u64, total: u64 },
    Complete,
}

/// Authoritative health of a streaming coordinator and its retained data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedBufferHealth {
    /// The coordinator can still refill the decoder-visible window.
    Refillable,
    /// The remote object has reached a valid EOF or cache completion.
    Complete,
    /// The owning generation explicitly cancelled the stream.
    Cancelled,
    /// The coordinator stored a terminal failure.
    Failed(PlaybackError),
    /// The coordinator exited without completion or a stored error.
    CoordinatorStopped,
}

impl SharedBufferHealth {
    pub fn promotion_error(&self) -> Option<PlaybackError> {
        Self::health_for_promotion(self).err()
    }

    fn health_for_promotion(&self) -> PlaybackResult<()> {
        match self {
            Self::Refillable | Self::Complete => Ok(()),
            Self::Cancelled => Err(PlaybackError::Cancelled(
                "streaming preload was cancelled".to_string(),
            )),
            Self::Failed(error) => Err(PlaybackError::UnhealthyPreload(error.to_string())),
            Self::CoordinatorStopped => Err(PlaybackError::UnhealthyPreload(
                "streaming preload coordinator stopped before completion".to_string(),
            )),
        }
    }
}

type BufferCallback = Box<dyn Fn(BufferEvent) + Send + Sync>;

struct BufferCallbackEntry {
    subscription_id: u64,
    callback: BufferCallback,
}

// ============ Buffer State ============

/// Inner shared state
struct SharedBufferInner {
    /// Retained decoder bytes. The deque never grows beyond `capacity`.
    data: RwLock<VecDeque<u8>>,
    capacity: usize,
    policy: StreamingBufferPolicy,
    /// Absolute offset represented by the first retained byte.
    base_offset: AtomicU64,
    /// Total file size, when known.
    total_size: AtomicU64,
    /// Absolute end offset of bytes received so far.
    downloaded: AtomicU64,
    /// Only one decoder owns retained-window demand for a cache coordinator.
    /// Other prepared readers keep independent positions and cannot overwrite
    /// this lease until the active reader explicitly releases it.
    next_reader_id: AtomicU64,
    active_reader_id: AtomicU64,
    active_reader_position: AtomicU64,
    /// Latest decoder-requested Range window generation and start offset.
    window_epoch: AtomicU64,
    requested_offset: AtomicU64,
    request_pending: AtomicBool,
    coordinator_active: AtomicBool,
    /// Contiguous prefix persisted from byte zero; used to decide whether the
    /// cache file is complete after out-of-order Range windows.
    cached_prefix: AtomicU64,
    finalized_cache_path: RwLock<Option<PathBuf>>,
    finalized_cache_reader: Mutex<Option<std::fs::File>>,
    download_complete: AtomicBool,
    cache_finalized: AtomicBool,
    cancelled: AtomicBool,
    error: RwLock<Option<PlaybackError>>,
    demand_stall_timeout: Duration,
    data_available: Condvar,
    wait_mutex: Mutex<()>,
    next_callback_subscription: AtomicU64,
    buffer_callback: RwLock<Option<BufferCallbackEntry>>,
}

static AUDIO_IN_FLIGHT: OnceLock<Mutex<HashMap<AudioCacheKey, Weak<SharedBufferInner>>>> =
    OnceLock::new();

fn audio_in_flight() -> &'static Mutex<HashMap<AudioCacheKey, Weak<SharedBufferInner>>> {
    AUDIO_IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

struct CoordinatorGuard {
    buffer: SharedBuffer,
    key: AudioCacheKey,
}

impl Drop for CoordinatorGuard {
    fn drop(&mut self) {
        self.buffer
            .inner
            .coordinator_active
            .store(false, Ordering::Release);
        self.buffer.inner.data_available.notify_all();
        let mut in_flight = audio_in_flight().lock();
        let should_remove = in_flight
            .get(&self.key)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, &self.buffer.inner));
        if should_remove {
            in_flight.remove(&self.key);
        }
    }
}

/// Thread-safe shared buffer for streaming audio
///
/// Download thread calls `append()` to add data.
/// Playback thread uses `StreamingBuffer` which calls `read_at()`.
#[derive(Clone)]
pub struct SharedBuffer {
    inner: Arc<SharedBufferInner>,
    /// Callback ownership belongs to this caller handle, not the shared cache
    /// coordinator. Clones used by the same playback lifecycle share it.
    callback_subscription: Arc<AtomicU64>,
}

/// Cancellation scoped to one `StreamingBuffer` reader.
///
/// A streaming seek may temporarily create a replacement decoder backed by
/// the same `SharedBuffer` as the old decoder. Cancelling the shared buffer in
/// that situation would also invalidate the replacement, so reader lifecycle
/// cancellation is kept separate from download/coordinator cancellation.
#[derive(Clone)]
pub(crate) struct StreamingReaderCancellation {
    cancelled: Arc<AtomicBool>,
    reader_id: u64,
    position: Arc<AtomicU64>,
    shared: std::sync::Weak<SharedBufferInner>,
}

impl StreamingReaderCancellation {
    fn new(shared: &SharedBuffer) -> Self {
        let reader_id = shared
            .inner
            .next_reader_id
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        let reader = Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            reader_id,
            position: Arc::new(AtomicU64::new(0)),
            shared: Arc::downgrade(&shared.inner),
        };
        let _ = reader.activate();
        reader
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(shared) = self.shared.upgrade() {
            let _ = shared.active_reader_id.compare_exchange(
                self.reader_id,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            shared.data_available.notify_all();
        }
    }

    /// Acquire the single retained-window demand lease without stealing it
    /// from another decoder.
    pub(crate) fn activate(&self) -> bool {
        if self.is_cancelled() {
            return false;
        }
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        let active = shared.active_reader_id.load(Ordering::Acquire);
        if active == self.reader_id {
            shared
                .active_reader_position
                .store(self.position.load(Ordering::Acquire), Ordering::Release);
            return true;
        }
        if active != 0 {
            return false;
        }
        match shared.active_reader_id.compare_exchange(
            0,
            self.reader_id,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                shared
                    .active_reader_position
                    .store(self.position.load(Ordering::Acquire), Ordering::Release);
                shared.data_available.notify_all();
                true
            }
            Err(active) if active == self.reader_id => {
                shared
                    .active_reader_position
                    .store(self.position.load(Ordering::Acquire), Ordering::Release);
                true
            }
            Err(_) => false,
        }
    }

    fn update_position(&self, position: u64) {
        self.position.store(position, Ordering::Release);
        if let Some(shared) = self.shared.upgrade()
            && shared.active_reader_id.load(Ordering::Acquire) == self.reader_id
        {
            shared
                .active_reader_position
                .store(position, Ordering::Release);
            shared.data_available.notify_all();
        }
    }

    fn owns_demand(&self) -> bool {
        self.shared
            .upgrade()
            .is_some_and(|shared| shared.active_reader_id.load(Ordering::Acquire) == self.reader_id)
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn same_reader(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}

impl std::fmt::Debug for SharedBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedBuffer")
            .field("total_size", &self.total_size())
            .field("downloaded", &self.downloaded())
            .field("download_complete", &self.is_download_complete())
            .field("cache_finalized", &self.is_cache_finalized())
            .finish()
    }
}

impl SharedBuffer {
    /// Create a fixed-capacity retained-window buffer.
    #[cfg(test)]
    pub fn new(total_size: u64) -> Self {
        Self::with_policy(total_size, StreamingBufferPolicy::default())
    }

    pub fn with_policy(total_size: u64, policy: StreamingBufferPolicy) -> Self {
        Self::with_policy_and_stall_timeout(total_size, policy, DECODER_DEMAND_STALL_TIMEOUT)
    }

    #[cfg(test)]
    pub fn with_capacity(total_size: u64, capacity: usize) -> Self {
        Self::with_capacity_and_stall_timeout(total_size, capacity, DECODER_DEMAND_STALL_TIMEOUT)
    }

    #[cfg(test)]
    fn with_capacity_and_stall_timeout(
        total_size: u64,
        capacity: usize,
        demand_stall_timeout: Duration,
    ) -> Self {
        let policy = StreamingBufferPolicy {
            capacity_bytes: capacity.max(1),
            ..StreamingBufferPolicy::default()
        };
        Self::with_policy_and_stall_timeout(total_size, policy, demand_stall_timeout)
    }

    fn with_policy_and_stall_timeout(
        total_size: u64,
        policy: StreamingBufferPolicy,
        demand_stall_timeout: Duration,
    ) -> Self {
        let capacity = policy.capacity_bytes().max(1);
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(SharedBufferInner {
                data: RwLock::new(VecDeque::with_capacity(capacity)),
                capacity,
                policy,
                base_offset: AtomicU64::new(0),
                total_size: AtomicU64::new(total_size),
                downloaded: AtomicU64::new(0),
                next_reader_id: AtomicU64::new(0),
                active_reader_id: AtomicU64::new(0),
                active_reader_position: AtomicU64::new(0),
                window_epoch: AtomicU64::new(0),
                requested_offset: AtomicU64::new(0),
                request_pending: AtomicBool::new(false),
                coordinator_active: AtomicBool::new(false),
                cached_prefix: AtomicU64::new(0),
                finalized_cache_path: RwLock::new(None),
                finalized_cache_reader: Mutex::new(None),
                download_complete: AtomicBool::new(false),
                cache_finalized: AtomicBool::new(false),
                cancelled: AtomicBool::new(false),
                error: RwLock::new(None),
                demand_stall_timeout,
                data_available: Condvar::new(),
                wait_mutex: Mutex::new(()),
                next_callback_subscription: AtomicU64::new(0),
                buffer_callback: RwLock::new(None),
            }),
            callback_subscription: Arc::new(AtomicU64::new(0)),
        }
    }

    fn from_shared_inner(inner: Arc<SharedBufferInner>) -> Self {
        Self {
            inner,
            callback_subscription: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn set_buffer_callback<F>(&self, callback: F)
    where
        F: Fn(BufferEvent) + Send + Sync + 'static,
    {
        let subscription_id = self
            .inner
            .next_callback_subscription
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        self.callback_subscription
            .store(subscription_id, Ordering::Release);
        *self.inner.buffer_callback.write() = Some(BufferCallbackEntry {
            subscription_id,
            callback: Box::new(callback),
        });
    }

    pub fn clear_buffer_callback(&self) {
        let subscription_id = self.callback_subscription.swap(0, Ordering::AcqRel);
        if subscription_id == 0 {
            return;
        }
        let mut callback = self.inner.buffer_callback.write();
        if callback
            .as_ref()
            .is_some_and(|entry| entry.subscription_id == subscription_id)
        {
            callback.take();
        }
    }

    fn notify_callback(&self, event: BufferEvent) {
        if let Some(entry) = self.inner.buffer_callback.read().as_ref() {
            (entry.callback)(event);
        }
    }

    pub fn policy(&self) -> StreamingBufferPolicy {
        self.inner.policy
    }

    /// Absolute offset of the first byte still retained in memory.
    pub fn base_offset(&self) -> u64 {
        self.inner.base_offset.load(Ordering::Acquire)
    }

    /// Absolute exclusive end offset of received bytes.
    #[cfg(test)]
    pub fn end_offset(&self) -> u64 {
        self.downloaded()
    }

    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }

    #[cfg(test)]
    pub fn retained_len(&self) -> usize {
        self.inner.data.read().len()
    }

    #[cfg(test)]
    pub(crate) fn set_coordinator_active_for_test(&self, active: bool) {
        self.inner
            .coordinator_active
            .store(active, Ordering::Release);
        self.inner.data_available.notify_all();
    }

    /// Bytes retained ahead of the decoder's actual absolute read position.
    pub fn buffered_ahead(&self) -> u64 {
        let reader = self.reader_position();
        let base = self.base_offset();
        let end = self.downloaded();
        if reader < base || reader > end {
            0
        } else {
            end.saturating_sub(reader)
        }
    }

    fn reader_position(&self) -> u64 {
        self.inner.active_reader_position.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn set_reader_position(&self, position: u64) {
        self.inner
            .active_reader_position
            .store(position, Ordering::Release);
        self.inner.data_available.notify_all();
    }

    /// Append bytes, evicting the oldest bytes when the fixed window is full.
    pub fn append(&self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }

        let mut data = self.inner.data.write();
        let mut base = self.base_offset();
        let mut downloaded = self.downloaded();
        let mut input = chunk;

        if input.len() >= self.inner.capacity {
            input = &input[input.len() - self.inner.capacity..];
            base = downloaded.saturating_add(chunk.len() as u64 - input.len() as u64);
            data.clear();
        }
        while data.len() + input.len() > self.inner.capacity {
            let _ = data.pop_front();
            base = base.saturating_add(1);
        }
        data.extend(input.iter().copied());
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        self.inner.base_offset.store(base, Ordering::Release);
        self.inner.downloaded.store(downloaded, Ordering::Release);
        self.inner
            .cached_prefix
            .store(downloaded, Ordering::Release);
        drop(data);

        self.inner.data_available.notify_all();
        let total = self.inner.total_size.load(Ordering::Acquire);
        self.notify_callback(BufferEvent::CacheAdvanced {
            cached: downloaded,
            total,
        });
    }

    fn append_window(&self, start: u64, chunk: &[u8], epoch: u64) -> bool {
        if chunk.is_empty() || self.window_epoch() != epoch {
            return false;
        }
        if chunk.len() > self.inner.capacity {
            return false;
        }
        let mut data = self.inner.data.write();
        if self.window_epoch() != epoch {
            return false;
        }
        let current_end = self.downloaded();
        let mut base = self.base_offset();
        if start != current_end {
            data.clear();
            base = start;
        }

        let required_evict = data
            .len()
            .saturating_add(chunk.len())
            .saturating_sub(self.inner.capacity);
        let consumed = self.reader_position().saturating_sub(base) as usize;
        if required_evict > consumed.min(data.len()) {
            return false;
        }
        for _ in 0..required_evict {
            let _ = data.pop_front();
            base = base.saturating_add(1);
        }
        data.extend(chunk.iter().copied());
        let end = start.saturating_add(chunk.len() as u64);
        self.inner.base_offset.store(base, Ordering::Release);
        self.inner.downloaded.store(end, Ordering::Release);
        self.inner.request_pending.store(false, Ordering::Release);
        drop(data);
        self.inner.data_available.notify_all();
        self.notify_callback(BufferEvent::CacheAdvanced {
            cached: self.cached_prefix(),
            total: self.total_size(),
        });
        true
    }

    fn window_epoch(&self) -> u64 {
        self.inner.window_epoch.load(Ordering::Acquire)
    }

    fn requested_window(&self) -> Option<(u64, u64)> {
        self.inner.request_pending.load(Ordering::Acquire).then(|| {
            (
                self.window_epoch(),
                self.inner.requested_offset.load(Ordering::Acquire),
            )
        })
    }

    fn request_window(&self, offset: u64) -> u64 {
        if self.inner.request_pending.load(Ordering::Acquire)
            && self.inner.requested_offset.load(Ordering::Acquire) == offset
        {
            return self.window_epoch();
        }
        self.inner.requested_offset.store(offset, Ordering::Release);
        self.inner.request_pending.store(true, Ordering::Release);
        let epoch = self
            .inner
            .window_epoch
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        self.inner.data_available.notify_all();
        epoch
    }

    fn set_cached_prefix(&self, prefix: u64) {
        self.inner.cached_prefix.store(prefix, Ordering::Release);
    }

    fn cached_prefix(&self) -> u64 {
        self.inner.cached_prefix.load(Ordering::Acquire)
    }

    fn set_finalized_cache_path(&self, path: PathBuf) {
        *self.inner.finalized_cache_path.write() = Some(path);
        self.inner.finalized_cache_reader.lock().take();
    }

    pub fn finalized_cache_path(&self) -> Option<PathBuf> {
        self.inner.finalized_cache_path.read().clone()
    }

    fn read_finalized_cache(&self, position: u64, buf: &mut [u8]) -> Option<io::Result<usize>> {
        let path = self.inner.finalized_cache_path.read().clone()?;
        Some((|| {
            let mut reader = self.inner.finalized_cache_reader.lock();
            if reader.is_none() {
                *reader = Some(std::fs::File::open(path)?);
            }
            let file = reader.as_mut().expect("finalized cache reader initialized");
            file.seek(SeekFrom::Start(position))?;
            file.read(buf)
        })())
    }

    /// Read data at position, blocking if not available
    ///
    /// Returns number of bytes read, or error if cancelled/failed.
    /// Blocks and waits for data when reading positions not yet downloaded.
    #[cfg(test)]
    pub fn read_at(&self, position: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.read_at_with_reader_cancel(position, buf, None)
    }

    fn read_at_with_reader_cancel(
        &self,
        position: u64,
        buf: &mut [u8],
        reader_cancellation: Option<&StreamingReaderCancellation>,
    ) -> io::Result<usize> {
        // Check for cancellation/error first
        if self.inner.cancelled.load(Ordering::Acquire)
            || reader_cancellation.is_some_and(StreamingReaderCancellation::is_cancelled)
        {
            tracing::debug!("read_at: cancelled at position {}", position);
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Streaming reader cancelled",
            ));
        }

        if let Some(err) = self.inner.error.read().as_ref() {
            tracing::debug!("read_at: error at position {}: {}", position, err);
            return Err(io::Error::other(err.to_string()));
        }

        // Wait for data if needed
        let mut wait_count = 0;
        let mut last_progress = (self.base_offset(), self.downloaded());
        let mut last_progress_at = Instant::now();
        loop {
            let downloaded = self.inner.downloaded.load(Ordering::Acquire);
            let base = self.base_offset();
            let total = self.inner.total_size.load(Ordering::Acquire);
            let is_complete = self.inner.download_complete.load(Ordering::Acquire);
            let progress = (base, downloaded);
            if progress != last_progress {
                last_progress = progress;
                last_progress_at = Instant::now();
            }

            // A validated total size defines remote EOF independently from
            // whether the sparse disk cache has finished backfilling holes.
            if total > 0 && position >= total {
                return Ok(0);
            }

            if (position < base || position >= downloaded)
                && let Some(result) = self.read_finalized_cache(position, buf)
            {
                return result;
            }

            if position < downloaded {
                if position < base {
                    if let Some(result) = self.read_finalized_cache(position, buf) {
                        return result;
                    }
                    if !self.inner.coordinator_active.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                "position {} is outside retained window [{}..{})",
                                position, base, downloaded
                            ),
                        ));
                    }
                    if reader_cancellation.is_some_and(StreamingReaderCancellation::owns_demand) {
                        self.request_window(position);
                    } else if reader_cancellation.is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "streaming reader does not own retained-window demand",
                        ));
                    } else {
                        self.request_window(position);
                    }
                } else {
                    let data = self.inner.data.read();
                    let available = downloaded.saturating_sub(position) as usize;
                    let to_read = buf.len().min(available).min(data.len());
                    if to_read > 0 {
                        let start = (position - base) as usize;
                        if start + to_read > data.len() {
                            return Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "retained window changed unexpectedly",
                            ));
                        }
                        for (dst, src) in buf[..to_read].iter_mut().zip(data.iter().skip(start)) {
                            *dst = *src;
                        }
                        return Ok(to_read);
                    }
                }
            }

            // Check cancellation again before waiting
            if self.inner.cancelled.load(Ordering::Acquire)
                || reader_cancellation.is_some_and(StreamingReaderCancellation::is_cancelled)
            {
                tracing::debug!("read_at: cancelled while waiting at position {}", position);
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Streaming reader cancelled",
                ));
            }

            if let Some(err) = self.inner.error.read().as_ref() {
                tracing::debug!(
                    "read_at: error while waiting at position {}: {}",
                    position,
                    err
                );
                return Err(io::Error::other(err.to_string()));
            }

            if !self.inner.coordinator_active.load(Ordering::Acquire) {
                if is_complete && total == 0 {
                    return Ok(0);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "streaming coordinator stopped before position {} became available",
                        position
                    ),
                ));
            }

            let stalled_for = last_progress_at.elapsed();
            if stalled_for >= self.inner.demand_stall_timeout {
                let message = format!(
                    "decoder demand at byte {position} made no progress for {} ms",
                    stalled_for.as_millis()
                );
                let stored_timeout =
                    self.set_error_if_absent(PlaybackError::NetworkError(message.clone()));
                if stored_timeout {
                    tracing::warn!(
                        position,
                        base,
                        downloaded,
                        stalled_ms = stalled_for.as_millis(),
                        "Streaming decoder demand timed out"
                    );
                }
                if self.is_cancelled()
                    || reader_cancellation.is_some_and(StreamingReaderCancellation::is_cancelled)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Streaming reader cancelled",
                    ));
                }
                if !stored_timeout && let Some(error) = self.error_message() {
                    return Err(io::Error::other(error.to_string()));
                }
                return Err(io::Error::new(io::ErrorKind::TimedOut, message));
            }

            // Data not yet available, wait for download to progress
            // This enables seeking to positions that haven't been downloaded yet
            wait_count += 1;

            if wait_count == 1 {
                tracing::info!(
                    "read_at: blocking at position {} (downloaded: {}/{}, complete: {})",
                    position,
                    downloaded,
                    total,
                    is_complete
                );
            } else if wait_count % 10 == 0 {
                tracing::info!(
                    "read_at: still waiting for data at position {} (downloaded: {}/{}, complete: {}, wait #{})",
                    position,
                    downloaded,
                    total,
                    is_complete,
                    wait_count
                );
            }
            let mut guard = self.inner.wait_mutex.lock();
            let wait_for = Duration::from_millis(100)
                .min(self.inner.demand_stall_timeout.saturating_sub(stalled_for));
            let _ = self.inner.data_available.wait_for(&mut guard, wait_for);
        }
    }

    /// Get downloaded bytes count
    pub fn downloaded(&self) -> u64 {
        self.inner.downloaded.load(Ordering::Acquire)
    }

    /// Get total size
    pub fn total_size(&self) -> u64 {
        self.inner.total_size.load(Ordering::Acquire)
    }

    /// The active decoder window has reached the validated remote EOF. This
    /// is independent from sparse cache backfill/finalization.
    pub fn remote_eof_reached(&self) -> bool {
        let total = self.total_size();
        let reader = self.reader_position();
        let base = self.base_offset();
        let end = self.downloaded();
        total > 0 && end >= total && reader >= base && reader <= end
    }

    /// Update total size (when actual content-length is received)
    pub fn set_total_size(&self, size: u64) {
        self.inner.total_size.store(size, Ordering::Release);
    }

    /// Check if all remote bytes have been received.
    pub fn is_complete(&self) -> bool {
        self.is_download_complete()
    }

    pub fn is_download_complete(&self) -> bool {
        self.inner.download_complete.load(Ordering::Acquire)
    }

    pub fn is_cache_finalized(&self) -> bool {
        self.inner.cache_finalized.load(Ordering::Acquire)
    }

    /// Check if cancelled
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub fn has_error(&self) -> bool {
        self.inner.error.read().is_some()
    }

    pub fn error_message(&self) -> Option<PlaybackError> {
        self.inner.error.read().clone()
    }

    /// Return a composable health snapshot used by current playback and
    /// preload promotion. Stored errors take precedence over coordinator
    /// lifetime so a failed exit remains classifiable after its guard drops.
    pub fn health(&self) -> SharedBufferHealth {
        if let Some(error) = self.error_message() {
            return SharedBufferHealth::Failed(error);
        }
        if self.is_cancelled() {
            return SharedBufferHealth::Cancelled;
        }
        if self.is_download_complete() || self.remote_eof_reached() {
            return SharedBufferHealth::Complete;
        }
        if self.inner.coordinator_active.load(Ordering::Acquire) {
            SharedBufferHealth::Refillable
        } else {
            SharedBufferHealth::CoordinatorStopped
        }
    }

    /// Cancel the cache/download coordinator itself.
    ///
    /// Playback, preload, seek and crossfade lifecycles must cancel their
    /// `StreamingReaderCancellation` instead. This is intentionally scoped to
    /// the audio module so ordinary callers cannot poison a shared cache key.
    pub(crate) fn cancel_coordinator(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.data_available.notify_all();
    }

    /// Set error state
    pub fn set_error(&self, error: PlaybackError) {
        *self.inner.error.write() = Some(error);
        self.inner.data_available.notify_all();
    }

    fn set_error_if_absent(&self, error: PlaybackError) -> bool {
        let mut stored = self.inner.error.write();
        if stored.is_some() || self.is_cancelled() {
            return false;
        }
        *stored = Some(error);
        drop(stored);
        self.inner.data_available.notify_all();
        true
    }

    /// Mark remote download completion. Formal cache publication is tracked
    /// separately by `mark_cache_finalized`.
    pub fn mark_complete(&self) {
        self.inner.download_complete.store(true, Ordering::Release);
        self.inner.data_available.notify_all();
        self.notify_callback(BufferEvent::Complete);
    }

    pub fn mark_cache_finalized(&self) {
        self.inner.cache_finalized.store(true, Ordering::Release);
        self.inner.data_available.notify_all();
    }

    /// Get contiguous cache progress as a fraction (0.0 to 1.0).
    #[cfg(test)]
    pub fn progress(&self) -> f32 {
        let total = self.inner.total_size.load(Ordering::Acquire);
        if total == 0 {
            return 0.0;
        }
        let downloaded = self.cached_prefix();
        (downloaded as f32 / total as f32).min(1.0)
    }

    /// Contiguous bytes persisted from the start of the remote object.
    pub fn cached_bytes(&self) -> u64 {
        self.cached_prefix()
    }
}

/// Streaming buffer that implements Read + Seek for rodio Decoder
///
/// Wraps a SharedBuffer and maintains a read position.
/// Blocks on read() when data is not yet available.
pub struct StreamingBuffer {
    shared: SharedBuffer,
    position: u64,
    reader_cancellation: StreamingReaderCancellation,
}

impl StreamingBuffer {
    /// Create a new streaming buffer
    pub fn new(shared: SharedBuffer) -> Self {
        let reader_cancellation = StreamingReaderCancellation::new(&shared);
        Self {
            shared,
            position: 0,
            reader_cancellation,
        }
    }

    /// Get reference to the shared buffer (for checking state)
    pub fn shared(&self) -> &SharedBuffer {
        &self.shared
    }

    pub(crate) fn reader_cancellation(&self) -> StreamingReaderCancellation {
        self.reader_cancellation.clone()
    }
}

impl Read for StreamingBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader_cancellation.update_position(self.position);
        let bytes_read = self.shared.read_at_with_reader_cancel(
            self.position,
            buf,
            Some(&self.reader_cancellation),
        )?;
        self.position += bytes_read as u64;
        self.reader_cancellation.update_position(self.position);
        Ok(bytes_read)
    }
}

impl Seek for StreamingBuffer {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let total = self.shared.total_size();
        let downloaded = self.shared.downloaded();
        let is_complete = self.shared.is_complete();

        tracing::debug!(
            "StreamingBuffer::seek({:?}) - total: {}, downloaded: {}, complete: {}, current_pos: {}",
            pos,
            total,
            downloaded,
            is_complete,
            self.position
        );

        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::End(offset) => {
                let size = if total > 0 {
                    total
                } else if is_complete {
                    downloaded
                } else {
                    tracing::warn!("SeekFrom::End failed: unknown file size");
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Cannot seek from end: unknown file size",
                    ));
                };
                if offset >= 0 {
                    size.checked_add(offset as u64)
                } else {
                    size.checked_sub(offset.unsigned_abs())
                }
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Seek position overflow")
                })?
            }
            SeekFrom::Current(offset) => if offset >= 0 {
                self.position.checked_add(offset as u64)
            } else {
                self.position.checked_sub(offset.unsigned_abs())
            }
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Seek position overflow"))?,
        };

        self.position = new_pos;
        self.reader_cancellation.update_position(new_pos);
        let base = self.shared.base_offset();
        let end = self.shared.downloaded();
        if new_pos < base || (new_pos >= end && new_pos < total) {
            if !self.reader_cancellation.owns_demand() {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "streaming reader does not own retained-window demand",
                ));
            }
            self.shared.request_window(new_pos);
        } else if self.reader_cancellation.owns_demand() && self.shared.requested_window().is_some()
        {
            // A rapid seek can return to retained data while an older miss is
            // still in flight. Supersede that epoch with the current window's
            // continuation so the stale response cannot replace readable data.
            self.shared.request_window(end);
        }
        tracing::debug!(
            "StreamingBuffer seek to {} (downloaded: {}, total: {}, complete: {})",
            self.position,
            downloaded,
            total,
            is_complete
        );

        Ok(self.position)
    }
}

impl Drop for StreamingBuffer {
    fn drop(&mut self) {
        self.reader_cancellation.cancel();
    }
}

// ============ Unified Download Function ============

enum RangeFetchResult {
    Data(Vec<u8>),
    Superseded,
    Cancelled,
    Fatal(PlaybackError),
}

async fn fetch_range_chunk(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
    buffer: &SharedBuffer,
    epoch: u64,
) -> RangeFetchResult {
    let expected = end.saturating_sub(start).saturating_add(1) as usize;
    let mut retries_completed = 0u8;

    loop {
        if buffer.is_cancelled() {
            return RangeFetchResult::Cancelled;
        }
        if let Some(error) = buffer.error_message() {
            return RangeFetchResult::Fatal(error);
        }
        if buffer.window_epoch() != epoch {
            return RangeFetchResult::Superseded;
        }

        let response = match client
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                if buffer.is_cancelled() {
                    return RangeFetchResult::Cancelled;
                }
                if buffer.window_epoch() != epoch {
                    return RangeFetchResult::Superseded;
                }
                let Some(backoff) = range_retry_backoff(retries_completed) else {
                    return RangeFetchResult::Fatal(PlaybackError::NetworkError(format!(
                        "Range request failed after {} retries: {}",
                        retries_completed,
                        error.without_url()
                    )));
                };
                retries_completed += 1;
                tracing::warn!(
                    retry = retries_completed,
                    max_retries = MAX_RANGE_RETRIES,
                    start,
                    end,
                    epoch,
                    "Retrying failed Range request"
                );
                tokio::time::sleep(backoff).await;
                continue;
            }
        };

        if buffer.is_cancelled() {
            return RangeFetchResult::Cancelled;
        }
        if let Some(error) = buffer.error_message() {
            return RangeFetchResult::Fatal(error);
        }
        if buffer.window_epoch() != epoch {
            return RangeFetchResult::Superseded;
        }

        if let Err(message) = validate_range_response(&response, start, end) {
            return RangeFetchResult::Fatal(message);
        }

        match response.bytes().await {
            Ok(body) => {
                if buffer.is_cancelled() {
                    return RangeFetchResult::Cancelled;
                }
                if let Some(error) = buffer.error_message() {
                    return RangeFetchResult::Fatal(error);
                }
                if buffer.window_epoch() != epoch {
                    return RangeFetchResult::Superseded;
                }
                if body.len() == expected {
                    return RangeFetchResult::Data(body.to_vec());
                }
                return RangeFetchResult::Fatal(PlaybackError::UnsupportedStreaming(format!(
                    "Range body length {}, expected {}",
                    body.len(),
                    expected
                )));
            }
            Err(error) => {
                if buffer.is_cancelled() {
                    return RangeFetchResult::Cancelled;
                }
                if let Some(error) = buffer.error_message() {
                    return RangeFetchResult::Fatal(error);
                }
                if buffer.window_epoch() != epoch {
                    return RangeFetchResult::Superseded;
                }
                let Some(backoff) = range_retry_backoff(retries_completed) else {
                    return RangeFetchResult::Fatal(PlaybackError::NetworkError(format!(
                        "Range body failed after {} retries: {}",
                        retries_completed,
                        error.without_url()
                    )));
                };
                retries_completed += 1;
                tracing::warn!(
                    retry = retries_completed,
                    max_retries = MAX_RANGE_RETRIES,
                    start,
                    end,
                    epoch,
                    "Retrying failed Range response body"
                );
                tokio::time::sleep(backoff).await;
            }
        }
    }
}

/// Mirror coordinator state to a caller that joined an already-running
/// download. Caller cancellation only stops this event follower; the shared
/// cache download remains owned by the global in-flight registry.
fn follow_existing_download(
    buffer: SharedBuffer,
    identity: StreamingIdentity,
    event_tx: tokio::sync::mpsc::Sender<StreamingEvent>,
) {
    tokio::spawn(async move {
        let mut playable_sent = false;
        let mut download_complete_sent = false;
        let mut last_progress = None;

        loop {
            if identity.is_cancelled() {
                return;
            }
            if let Some(message) = buffer.error_message() {
                let _ = event_tx
                    .send(StreamingEvent::new(
                        identity,
                        StreamingEventKind::Error(message),
                    ))
                    .await;
                return;
            }

            let progress = (buffer.cached_prefix(), buffer.total_size());
            if last_progress != Some(progress) {
                last_progress = Some(progress);
                let _ = event_tx.try_send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::CacheProgress(progress.0, progress.1),
                ));
            }
            if !playable_sent
                && buffer.policy().can_start_or_resume(
                    buffer.buffered_ahead(),
                    buffer.remote_eof_reached() || buffer.is_download_complete(),
                )
            {
                let _ = event_tx
                    .send(StreamingEvent::new(
                        identity.clone(),
                        StreamingEventKind::Playable,
                    ))
                    .await;
                playable_sent = true;
            }
            if !download_complete_sent && buffer.is_download_complete() {
                let _ = event_tx
                    .send(StreamingEvent::new(
                        identity.clone(),
                        StreamingEventKind::DownloadComplete,
                    ))
                    .await;
                download_complete_sent = true;
            }
            if buffer.is_cache_finalized() {
                if let Some(path) = buffer.finalized_cache_path() {
                    let _ = event_tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::CacheFinalized(path),
                        ))
                        .await;
                }
                let _ = event_tx
                    .send(StreamingEvent::new(identity, StreamingEventKind::Complete))
                    .await;
                return;
            }
            if !buffer.inner.coordinator_active.load(Ordering::Acquire) {
                let kind = if buffer.is_download_complete() {
                    StreamingEventKind::CacheFinalizationFailed(
                        "shared download finished without a published cache file".to_string(),
                    )
                } else if buffer.is_cancelled() {
                    StreamingEventKind::Error(PlaybackError::Cancelled(
                        "shared download was cancelled".to_string(),
                    ))
                } else {
                    StreamingEventKind::Error(PlaybackError::StreamingFailed(
                        "shared download coordinator stopped".to_string(),
                    ))
                };
                let _ = event_tx.send(StreamingEvent::new(identity, kind)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
}

/// Start downloading audio to a SharedBuffer using strict byte ranges.
///
/// The source must support `HEAD` and a validated `Range: bytes=0-0` probe.
/// Sequential GET downloads are intentionally not used as a fallback.
pub fn start_buffer_download(
    url: String,
    cache_path: PathBuf,
    cache_key: AudioCacheKey,
    cache_store: Arc<dyn AudioCacheStore>,
    bitrate_bps: Option<u32>,
    identity: StreamingIdentity,
    event_tx: Option<tokio::sync::mpsc::Sender<StreamingEvent>>,
) -> SharedBuffer {
    let (shared_buffer, is_new) = {
        let mut in_flight = audio_in_flight().lock();
        if let Some(existing) = in_flight.get(&cache_key).and_then(Weak::upgrade) {
            let existing = SharedBuffer::from_shared_inner(existing);
            if !matches!(
                existing.health(),
                SharedBufferHealth::Failed(_)
                    | SharedBufferHealth::Cancelled
                    | SharedBufferHealth::CoordinatorStopped
            ) {
                (existing, false)
            } else {
                in_flight.remove(&cache_key);
                let shared_buffer =
                    SharedBuffer::with_policy(0, StreamingBufferPolicy::from_bitrate(bitrate_bps));
                shared_buffer
                    .inner
                    .coordinator_active
                    .store(true, Ordering::Release);
                in_flight.insert(cache_key, Arc::downgrade(&shared_buffer.inner));
                (shared_buffer, true)
            }
        } else {
            let shared_buffer =
                SharedBuffer::with_policy(0, StreamingBufferPolicy::from_bitrate(bitrate_bps));
            shared_buffer
                .inner
                .coordinator_active
                .store(true, Ordering::Release);
            in_flight.insert(cache_key, Arc::downgrade(&shared_buffer.inner));
            (shared_buffer, true)
        }
    };
    if !is_new {
        if let Some(event_tx) = event_tx {
            follow_existing_download(shared_buffer.clone(), identity, event_tx);
        }
        return shared_buffer;
    }
    let buffer_clone = shared_buffer.clone();

    tokio::spawn(async move {
        let _coordinator_guard = CoordinatorGuard {
            buffer: buffer_clone.clone(),
            key: cache_key,
        };
        let client = match reqwest::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                let error =
                    PlaybackError::NetworkError(format!("HTTP client setup failed: {error}"));
                buffer_clone.set_error(error.clone());
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
        };
        let fail = |error: PlaybackError, buffer: &SharedBuffer| {
            buffer.set_error(error);
        };

        let _head = match client.head(&url).send().await {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                let error = PlaybackError::UnsupportedStreaming(format!(
                    "HEAD returned HTTP {}",
                    response.status()
                ));
                fail(error.clone(), &buffer_clone);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
            Err(error) => {
                let error = PlaybackError::NetworkError(format!(
                    "HEAD request failed: {}",
                    error.without_url()
                ));
                fail(error.clone(), &buffer_clone);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
        };
        let probe = match client
            .get(&url)
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let error = PlaybackError::NetworkError(format!(
                    "Range probe failed: {}",
                    error.without_url()
                ));
                fail(error.clone(), &buffer_clone);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
        };
        let probe_range = match validate_range_response(&probe, 0, 0) {
            Ok(range) => range,
            Err(error) => {
                fail(error.clone(), &buffer_clone);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
        };
        let probe_body = match probe.bytes().await {
            Ok(body) if body.len() == 1 => body,
            Ok(body) => {
                let error = PlaybackError::UnsupportedStreaming(format!(
                    "Range probe body length {}, expected 1",
                    body.len()
                ));
                fail(error.clone(), &buffer_clone);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
            Err(error) => {
                let error = PlaybackError::NetworkError(format!(
                    "Range probe body failed: {}",
                    error.without_url()
                ));
                fail(error.clone(), &buffer_clone);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
        };
        let total_size = probe_range.total;
        buffer_clone.set_total_size(total_size);

        let temp_path = cache_store.unique_temp_path(&cache_path);
        let mut file = match std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&temp_path)
        {
            Ok(file) => file,
            Err(error) => {
                let error = PlaybackError::IoError(format!("could not create cache file: {error}"));
                fail(error.clone(), &buffer_clone);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error),
                        ))
                        .await;
                }
                return;
            }
        };

        let mut playable_sent = false;
        if let Err(error) = file.write_all(&probe_body) {
            let error = PlaybackError::IoError(format!("cache write failed: {error}"));
            fail(error.clone(), &buffer_clone);
            drop(file);
            cache_store.cleanup_temp_file(&temp_path);
            if let Some(tx) = &event_tx {
                let _ = tx
                    .send(StreamingEvent::new(
                        identity.clone(),
                        StreamingEventKind::Error(error),
                    ))
                    .await;
            }
            return;
        }
        buffer_clone.append(&probe_body);
        buffer_clone.set_cached_prefix(1);
        let mut downloaded = buffer_clone.downloaded();
        if total_size == 1
            && let Some(tx) = &event_tx
        {
            let _ = tx
                .send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::Playable,
                ))
                .await;
            playable_sent = true;
            let _ = tx
                .send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::CacheProgress(downloaded, total_size),
                ))
                .await;
        }
        let mut active_epoch = buffer_clone.window_epoch();

        'download: loop {
            if buffer_clone.is_cancelled() {
                drop(file);
                cache_store.cleanup_temp_file(&temp_path);
                return;
            }
            if buffer_clone.has_error() {
                drop(file);
                cache_store.cleanup_temp_file(&temp_path);
                return;
            }

            let requested_window = buffer_clone.requested_window();
            if let Some((epoch, _)) = requested_window {
                active_epoch = epoch;
            }

            let buffered_ahead = buffer_clone.buffered_ahead();
            if !playable_sent
                && buffer_clone.policy().can_start_or_resume(
                    buffered_ahead,
                    buffer_clone.remote_eof_reached() || buffer_clone.is_download_complete(),
                )
            {
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Playable,
                        ))
                        .await;
                }
                playable_sent = true;
            }

            let ring_needs_refill = buffer_clone.policy().should_refill(buffered_ahead)
                && buffer_clone.downloaded() < total_size;
            let cached_prefix = buffer_clone.cached_prefix();
            let (start, feed_ring, chunk_bytes) = if let Some((_, offset)) = requested_window {
                (
                    offset.min(total_size.saturating_sub(1)),
                    true,
                    buffer_clone.policy().range_chunk_bytes(),
                )
            } else if ring_needs_refill {
                (
                    buffer_clone.downloaded(),
                    true,
                    buffer_clone.policy().range_chunk_bytes(),
                )
            } else if cached_prefix < total_size {
                // Keep the persistent cache moving while playback already has
                // a healthy reserve. Small chunks bound the time before active
                // decoder demand is reconsidered.
                (cached_prefix, false, CACHE_BACKFILL_CHUNK_BYTES)
            } else {
                break;
            };

            let from_disk_cache = feed_ring && start < cached_prefix;
            let readable_end = if from_disk_cache {
                cached_prefix.saturating_sub(1)
            } else {
                total_size.saturating_sub(1)
            };
            let end = (start.saturating_add(chunk_bytes).saturating_sub(1)).min(readable_end);
            let body = if from_disk_cache {
                let len = end.saturating_sub(start).saturating_add(1) as usize;
                let mut body = vec![0; len];
                if let Err(error) = file
                    .seek(SeekFrom::Start(start))
                    .and_then(|_| file.read_exact(&mut body))
                {
                    let error = PlaybackError::IoError(format!("cache read failed: {error}"));
                    fail(error.clone(), &buffer_clone);
                    drop(file);
                    cache_store.cleanup_temp_file(&temp_path);
                    if let Some(tx) = &event_tx {
                        let _ = tx
                            .send(StreamingEvent::new(
                                identity.clone(),
                                StreamingEventKind::Error(error),
                            ))
                            .await;
                    }
                    return;
                }
                if buffer_clone.window_epoch() != active_epoch {
                    continue 'download;
                }
                body
            } else {
                match fetch_range_chunk(&client, &url, start, end, &buffer_clone, active_epoch)
                    .await
                {
                    RangeFetchResult::Data(body) => body,
                    RangeFetchResult::Superseded => continue 'download,
                    RangeFetchResult::Cancelled => {
                        buffer_clone.cancel_coordinator();
                        drop(file);
                        cache_store.cleanup_temp_file(&temp_path);
                        return;
                    }
                    RangeFetchResult::Fatal(error) => {
                        fail(error.clone(), &buffer_clone);
                        drop(file);
                        cache_store.cleanup_temp_file(&temp_path);
                        if let Some(tx) = &event_tx {
                            let _ = tx
                                .send(StreamingEvent::new(
                                    identity.clone(),
                                    StreamingEventKind::Error(error),
                                ))
                                .await;
                        }
                        return;
                    }
                }
            };

            if !from_disk_cache {
                if let Err(error) = file
                    .seek(SeekFrom::Start(start))
                    .and_then(|_| file.write_all(&body))
                {
                    let error = PlaybackError::IoError(format!("cache write failed: {error}"));
                    fail(error.clone(), &buffer_clone);
                    drop(file);
                    cache_store.cleanup_temp_file(&temp_path);
                    if let Some(tx) = &event_tx {
                        let _ = tx
                            .send(StreamingEvent::new(
                                identity.clone(),
                                StreamingEventKind::Error(error),
                            ))
                            .await;
                    }
                    return;
                }
                if start == buffer_clone.cached_prefix() {
                    buffer_clone.set_cached_prefix(end.saturating_add(1));
                }
            }
            if feed_ring {
                while !buffer_clone.append_window(start, &body, active_epoch) {
                    if buffer_clone.is_cancelled() {
                        drop(file);
                        cache_store.cleanup_temp_file(&temp_path);
                        return;
                    }
                    if buffer_clone.window_epoch() != active_epoch {
                        continue 'download;
                    }
                    // The ring is full of bytes the decoder has not consumed.
                    // Keep the fetched chunk bounded and wait instead of
                    // evicting unread audio.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } else {
                buffer_clone.notify_callback(BufferEvent::CacheAdvanced {
                    cached: buffer_clone.cached_prefix(),
                    total: total_size,
                });
            }
            downloaded = downloaded.max(end.saturating_add(1));

            if feed_ring
                && !playable_sent
                && buffer_clone.policy().can_start_or_resume(
                    buffer_clone.buffered_ahead(),
                    buffer_clone.remote_eof_reached() || buffer_clone.is_download_complete(),
                )
            {
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Playable,
                        ))
                        .await;
                }
                playable_sent = true;
            }
            if let Some(tx) = &event_tx {
                let _ = tx.try_send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::CacheProgress(buffer_clone.cached_prefix(), total_size),
                ));
            }
        }

        if let Err(error) = file.flush() {
            let error = PlaybackError::IoError(format!("cache flush failed: {error}"));
            fail(error.clone(), &buffer_clone);
            drop(file);
            cache_store.cleanup_temp_file(&temp_path);
            if let Some(tx) = &event_tx {
                let _ = tx
                    .send(StreamingEvent::new(
                        identity.clone(),
                        StreamingEventKind::Error(error),
                    ))
                    .await;
            }
            return;
        }
        if let Err(error) = file.sync_all() {
            let error = PlaybackError::IoError(format!("cache sync failed: {error}"));
            fail(error.clone(), &buffer_clone);
            drop(file);
            cache_store.cleanup_temp_file(&temp_path);
            if let Some(tx) = &event_tx {
                let _ = tx
                    .send(StreamingEvent::new(
                        identity.clone(),
                        StreamingEventKind::Error(error),
                    ))
                    .await;
            }
            return;
        }
        drop(file);

        let detected_ext = match std::fs::File::open(&temp_path) {
            Ok(mut reader) => {
                let mut prefix = vec![0u8; FORMAT_DETECTION_PREFIX_BYTES];
                let len = std::io::Read::read(&mut reader, &mut prefix).unwrap_or(0);
                match crate::utils::detect_audio_format(&prefix[..len]) {
                    Some(extension) => extension.to_string(),
                    None => {
                        let error = PlaybackError::UnsupportedFormat(
                            "unknown or damaged audio prefix".to_string(),
                        );
                        drop(reader);
                        cache_store.cleanup_temp_file(&temp_path);
                        if let Some(tx) = &event_tx {
                            let _ = tx
                                .send(StreamingEvent::new(
                                    identity.clone(),
                                    StreamingEventKind::Error(error.clone()),
                                ))
                                .await;
                        }
                        fail(error, &buffer_clone);
                        return;
                    }
                }
            }
            Err(error) => {
                let error = PlaybackError::IoError(format!(
                    "could not open cache for format detection: {error}"
                ));
                cache_store.cleanup_temp_file(&temp_path);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::Error(error.clone()),
                        ))
                        .await;
                }
                fail(error, &buffer_clone);
                return;
            }
        };
        let final_ext = detected_ext;
        let stem = cache_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let parent = cache_path.parent().unwrap_or(std::path::Path::new("."));
        let final_path = parent.join(format!("{stem}.{final_ext}"));

        buffer_clone.mark_complete();
        if let Some(tx) = &event_tx {
            let _ = tx
                .send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::DownloadComplete,
                ))
                .await;
        }

        match cache_store.publish_or_reuse(&temp_path, &final_path, Some(total_size)) {
            Err(error) => {
                let message = format!("failed to finalize cache {:?}: {}", final_path, error);
                cache_store.cleanup_temp_file(&temp_path);
                if let Some(tx) = &event_tx {
                    let _ = tx
                        .send(StreamingEvent::new(
                            identity.clone(),
                            StreamingEventKind::CacheFinalizationFailed(message),
                        ))
                        .await;
                }
            }
            Ok(publish_result) => match cache_store.write_audio_manifest(
                &final_path,
                cache_key.song_id,
                cache_key.actual_quality,
                total_size,
                &final_ext,
            ) {
                Err(error) => {
                    let message = format!(
                        "failed to publish cache manifest for {:?}: {}",
                        final_path, error
                    );
                    tracing::warn!(?error, ?final_path, "Failed to write audio cache manifest");
                    if publish_result == PublishOutcome::Published {
                        cache_store.remove_audio_cache(&final_path);
                    }
                    if let Some(tx) = &event_tx {
                        let _ = tx
                            .send(StreamingEvent::new(
                                identity.clone(),
                                StreamingEventKind::CacheFinalizationFailed(message),
                            ))
                            .await;
                    }
                }
                Ok(()) => {
                    buffer_clone.set_finalized_cache_path(final_path.clone());
                    buffer_clone.mark_cache_finalized();
                    if let Some(tx) = &event_tx {
                        let _ = tx
                            .send(StreamingEvent::new(
                                identity.clone(),
                                StreamingEventKind::CacheFinalized(final_path),
                            ))
                            .await;
                    }
                }
            },
        }

        if buffer_clone.is_cache_finalized()
            && let Some(tx) = &event_tx
        {
            let _ = tx
                .send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::Complete,
                ))
                .await;
        }
        if let Some(tx) = &event_tx
            && !playable_sent
        {
            let _ = tx
                .send(StreamingEvent::new(
                    identity.clone(),
                    StreamingEventKind::Playable,
                ))
                .await;
        }
        tracing::debug!("Strict Range download complete: {} bytes", downloaded);
    });

    shared_buffer
}

/// Wait until the retained decoder window itself reaches the startup high
/// watermark. Unlike event-channel waiting, this remains safe for callers
/// that intentionally discard progress events (for example startup restore).
pub async fn wait_for_buffer_playable(buffer: &SharedBuffer, timeout_secs: u64) -> bool {
    tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        loop {
            if buffer.is_cancelled() || buffer.has_error() {
                return false;
            }
            if buffer.policy().can_start_or_resume(
                buffer.buffered_ahead(),
                buffer.remote_eof_reached() || buffer.is_complete(),
            ) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or(false)
}

// ============ Tests ============

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct TestCacheStore;

    impl crate::application::ports::cache::CachePublisher for TestCacheStore {
        fn unique_temp_path(&self, final_path: &std::path::Path) -> PathBuf {
            static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let name = final_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("audio");
            final_path.with_file_name(format!(
                ".{name}.test-{}-{sequence}.part",
                std::process::id()
            ))
        }

        fn cleanup_temp_file(&self, path: &std::path::Path) {
            let _ = std::fs::remove_file(path);
        }

        fn publish_or_reuse(
            &self,
            temp_path: &std::path::Path,
            final_path: &std::path::Path,
            expected_size: Option<u64>,
        ) -> std::io::Result<PublishOutcome> {
            if let Some(expected_size) = expected_size
                && std::fs::metadata(temp_path)?.len() != expected_size
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "test cache size mismatch",
                ));
            }
            std::fs::rename(temp_path, final_path)?;
            Ok(PublishOutcome::Published)
        }
    }

    impl AudioCacheStore for TestCacheStore {
        fn write_audio_manifest(
            &self,
            _path: &std::path::Path,
            _song_id: u64,
            _actual_quality: crate::domain::audio::QualityLevel,
            _size: u64,
            _format: &str,
        ) -> std::io::Result<()> {
            Ok(())
        }

        fn remove_audio_cache(&self, path: &std::path::Path) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn test_cache_store() -> Arc<dyn AudioCacheStore> {
        Arc::new(TestCacheStore)
    }

    #[test]
    fn range_window_refills_until_the_policy_high_water_mark() {
        let policy = StreamingBufferPolicy::from_bitrate(Some(9_200_000));
        assert!(policy.should_refill(policy.low_water_mark_bytes() + 1));
        assert!(policy.should_refill(policy.high_water_mark_bytes() - 1));
        assert!(!policy.should_refill(policy.high_water_mark_bytes()));
    }

    #[test]
    fn streaming_policy_is_bitrate_aware_monotonic_and_bounded() {
        let standard = StreamingBufferPolicy::from_bitrate(Some(320_000));
        let lossless = StreamingBufferPolicy::from_bitrate(Some(2_000_000));
        let high_rate = StreamingBufferPolicy::from_bitrate(Some(9_200_000));

        for policy in [standard, lossless, high_rate] {
            assert!(policy.low_water_mark_bytes() < policy.high_water_mark_bytes());
            assert!(policy.high_water_mark_bytes() < policy.capacity_bytes() as u64);
            assert!(
                policy.capacity_bytes() as u64
                    >= policy.high_water_mark_bytes() + policy.range_chunk_bytes()
            );
        }
        assert!(standard.low_water_mark_bytes() <= lossless.low_water_mark_bytes());
        assert!(lossless.low_water_mark_bytes() <= high_rate.low_water_mark_bytes());
        assert!(standard.high_water_mark_bytes() <= lossless.high_water_mark_bytes());
        assert!(lossless.high_water_mark_bytes() <= high_rate.high_water_mark_bytes());
    }

    #[test]
    fn high_rate_refill_has_a_chunk_of_network_delay_before_low_water() {
        let policy = StreamingBufferPolicy::from_bitrate(Some(9_200_000));
        let mut buffered_ahead = policy.high_water_mark_bytes();
        assert!(!policy.should_refill(buffered_ahead));

        buffered_ahead -= 1;
        assert!(policy.should_refill(buffered_ahead));
        buffered_ahead = buffered_ahead.saturating_sub(policy.range_chunk_bytes());
        assert!(buffered_ahead > policy.low_water_mark_bytes());
        assert!(!policy.should_enter_buffering(buffered_ahead, false));
    }

    #[test]
    fn stale_callback_handle_cannot_clear_a_new_subscription() {
        let coordinator = SharedBuffer::new(4);
        let old = SharedBuffer::from_shared_inner(coordinator.inner.clone());
        let current = SharedBuffer::from_shared_inner(coordinator.inner.clone());
        let old_calls = Arc::new(AtomicU64::new(0));
        let current_calls = Arc::new(AtomicU64::new(0));

        let old_counter = old_calls.clone();
        old.set_buffer_callback(move |_| {
            old_counter.fetch_add(1, Ordering::Relaxed);
        });
        let current_counter = current_calls.clone();
        current.set_buffer_callback(move |_| {
            current_counter.fetch_add(1, Ordering::Relaxed);
        });

        old.clear_buffer_callback();
        coordinator.append(&[1, 2, 3, 4]);

        assert_eq!(old_calls.load(Ordering::Relaxed), 0);
        assert_eq!(current_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn audio_downloads_are_deduplicated_by_song_and_actual_quality() {
        let key = AudioCacheKey {
            song_id: 991,
            actual_quality: crate::domain::audio::QualityLevel::Lossless,
        };
        let existing = SharedBuffer::new(100);
        existing.set_coordinator_active_for_test(true);
        audio_in_flight()
            .lock()
            .insert(key, Arc::downgrade(&existing.inner));
        let controller = crate::audio::identity::PlaybackGenerationController::new();
        controller.activate_generation();

        let reused = start_buffer_download(
            "http://127.0.0.1:1/audio".to_string(),
            std::env::temp_dir().join("rustle-dedupe"),
            key,
            test_cache_store(),
            Some(320_000),
            StreamingIdentity::Preload(controller.reserve_preload_identity().unwrap()),
            None,
        );
        assert!(Arc::ptr_eq(&existing.inner, &reused.inner));
        audio_in_flight().lock().remove(&key);
    }

    #[test]
    fn range_retry_budget_allows_three_retries_then_stops() {
        assert_eq!(range_retry_backoff(0), Some(Duration::from_millis(100)));
        assert_eq!(range_retry_backoff(1), Some(Duration::from_millis(200)));
        assert_eq!(range_retry_backoff(2), Some(Duration::from_millis(300)));
        assert_eq!(range_retry_backoff(3), None);
    }

    #[test]
    fn shared_buffer_health_distinguishes_refillable_complete_and_terminal_states() {
        let refillable = SharedBuffer::new(100);
        refillable
            .inner
            .coordinator_active
            .store(true, Ordering::Release);
        assert_eq!(refillable.health(), SharedBufferHealth::Refillable);
        assert!(refillable.health().promotion_error().is_none());

        let complete = SharedBuffer::new(4);
        complete.append(&[0; 4]);
        complete.mark_complete();
        assert_eq!(complete.health(), SharedBufferHealth::Complete);
        assert!(complete.health().promotion_error().is_none());

        let failed = SharedBuffer::new(100);
        failed.set_error(PlaybackError::NetworkError("connection reset".to_string()));
        assert!(matches!(
            failed.health(),
            SharedBufferHealth::Failed(PlaybackError::NetworkError(_))
        ));
        assert!(failed.health().promotion_error().is_some());

        let cancelled = SharedBuffer::new(100);
        cancelled.cancel_coordinator();
        assert_eq!(cancelled.health(), SharedBufferHealth::Cancelled);

        let stopped = SharedBuffer::new(100);
        assert_eq!(stopped.health(), SharedBufferHealth::CoordinatorStopped);
    }

    #[test]
    fn download_completion_is_distinct_from_cache_finalization() {
        let buffer = SharedBuffer::new(4);
        buffer.append(&[0; 4]);
        buffer.mark_complete();
        assert!(buffer.is_download_complete());
        assert!(!buffer.is_cache_finalized());
        buffer.mark_cache_finalized();
        assert!(buffer.is_cache_finalized());
    }

    #[test]
    fn stored_failure_remains_authoritative_after_coordinator_exit() {
        let buffer = SharedBuffer::new(100);
        buffer
            .inner
            .coordinator_active
            .store(true, Ordering::Release);
        buffer.set_error(PlaybackError::NetworkError("body failed".to_string()));
        drop(CoordinatorGuard {
            buffer: buffer.clone(),
            key: AudioCacheKey {
                song_id: 1,
                actual_quality: crate::domain::audio::QualityLevel::Standard,
            },
        });

        assert!(matches!(
            buffer.health(),
            SharedBufferHealth::Failed(PlaybackError::NetworkError(_))
        ));
    }

    #[test]
    fn decoder_demand_without_progress_becomes_terminal_with_injected_deadline() {
        let buffer =
            SharedBuffer::with_capacity_and_stall_timeout(100, 16, Duration::from_millis(40));
        buffer
            .inner
            .coordinator_active
            .store(true, Ordering::Release);
        let started = Instant::now();
        let error = buffer.read_at(0, &mut [0; 1]).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(matches!(
            buffer.health(),
            SharedBufferHealth::Failed(PlaybackError::NetworkError(_))
        ));
    }

    #[test]
    fn decoder_demand_deadline_resets_when_the_retained_window_progresses() {
        let buffer =
            SharedBuffer::with_capacity_and_stall_timeout(100, 16, Duration::from_millis(200));
        buffer
            .inner
            .coordinator_active
            .store(true, Ordering::Release);
        let reader = buffer.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut byte = [0; 1];
            tx.send(reader.read_at(2, &mut byte).map(|count| (count, byte[0])))
                .unwrap();
        });

        std::thread::sleep(Duration::from_millis(120));
        buffer.append(&[1]);
        std::thread::sleep(Duration::from_millis(120));
        buffer.append(&[2, 3]);

        assert_eq!(
            rx.recv_timeout(Duration::from_millis(500))
                .unwrap()
                .unwrap(),
            (1, 3)
        );
        assert_eq!(buffer.health(), SharedBufferHealth::Refillable);
    }

    #[test]
    fn cancellation_wakes_a_blocked_decoder_before_the_stall_deadline() {
        let buffer = SharedBuffer::with_capacity_and_stall_timeout(100, 16, Duration::from_secs(5));
        buffer
            .inner
            .coordinator_active
            .store(true, Ordering::Release);
        let reader = buffer.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx.send(reader.read_at(0, &mut [0; 1])).unwrap();
        });
        started_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        buffer.cancel_coordinator();

        let error = result_rx
            .recv_timeout(Duration::from_millis(300))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(buffer.health(), SharedBufferHealth::Cancelled);
    }

    #[test]
    fn test_shared_buffer_new() {
        let buffer = SharedBuffer::new(1000);
        assert_eq!(buffer.total_size(), 1000);
        assert_eq!(buffer.downloaded(), 0);
        assert!(!buffer.is_complete());
        assert!(!buffer.is_cancelled());
    }

    #[test]
    fn test_shared_buffer_append() {
        let buffer = SharedBuffer::new(100);

        buffer.append(&[1, 2, 3, 4, 5]);
        assert_eq!(buffer.downloaded(), 5);

        buffer.append(&[6, 7, 8, 9, 10]);
        assert_eq!(buffer.downloaded(), 10);
    }

    #[test]
    fn test_shared_buffer_retained_window_is_fixed_and_evicts_oldest() {
        let buffer = SharedBuffer::with_capacity(10, 4);
        assert_eq!(buffer.capacity(), 4);
        buffer.append(&[0, 1, 2, 3]);
        assert_eq!(buffer.base_offset(), 0);
        assert_eq!(buffer.retained_len(), 4);

        buffer.append(&[4, 5, 6]);
        assert_eq!(buffer.base_offset(), 3);
        assert_eq!(buffer.end_offset(), 7);
        assert_eq!(buffer.retained_len(), 4);

        let mut bytes = [0u8; 4];
        assert_eq!(buffer.read_at(3, &mut bytes).unwrap(), 4);
        assert_eq!(bytes, [3, 4, 5, 6]);
        assert!(buffer.read_at(0, &mut [0u8; 1]).is_err());
    }

    #[test]
    fn retained_window_miss_requests_latest_range_and_rejects_stale_data() {
        let buffer = SharedBuffer::with_capacity(100, 8);
        buffer.append(&[0, 1, 2, 3, 4, 5, 6, 7]);
        buffer
            .inner
            .coordinator_active
            .store(true, Ordering::Release);

        let first = buffer.request_window(40);
        let second = buffer.request_window(60);
        assert_ne!(first, second);
        assert_eq!(buffer.requested_window(), Some((second, 60)));
        assert!(!buffer.append_window(40, &[1, 2, 3], first));
        assert!(buffer.append_window(60, &[9, 8, 7], second));
        assert_eq!(buffer.base_offset(), 60);
        assert_eq!(buffer.end_offset(), 63);
        let mut bytes = [0; 3];
        assert_eq!(buffer.read_at(60, &mut bytes).unwrap(), 3);
        assert_eq!(bytes, [9, 8, 7]);
    }

    #[test]
    fn retained_seek_supersedes_an_older_in_flight_window_miss() {
        let buffer = SharedBuffer::with_capacity(100, 8);
        buffer.append(&[0, 1, 2, 3, 4, 5, 6, 7]);
        buffer
            .inner
            .coordinator_active
            .store(true, Ordering::Release);
        let stale_epoch = buffer.request_window(40);

        let mut streaming = StreamingBuffer::new(buffer.clone());
        assert_eq!(streaming.seek(SeekFrom::Start(2)).unwrap(), 2);
        let (current_epoch, requested_offset) = buffer.requested_window().unwrap();

        assert_ne!(current_epoch, stale_epoch);
        assert_eq!(requested_offset, 8);
        assert!(!buffer.append_window(40, &[9, 9, 9], stale_epoch));
        let mut bytes = [0; 2];
        assert_eq!(buffer.read_at(2, &mut bytes).unwrap(), 2);
        assert_eq!(bytes, [2, 3]);
    }

    #[test]
    fn declared_remote_eof_does_not_wait_for_sparse_cache_finalization() {
        let buffer = SharedBuffer::with_capacity(100, 8);
        buffer.append_window(92, &[1, 2, 3, 4, 5, 6, 7, 8], 0);

        assert!(!buffer.is_complete());
        assert_eq!(buffer.read_at(100, &mut [0; 1]).unwrap(), 0);
    }

    #[test]
    fn retained_window_never_evicts_unread_decoder_bytes() {
        let buffer = SharedBuffer::with_capacity(100, 8);
        assert!(buffer.append_window(0, &[0, 1, 2, 3, 4, 5], 0));

        assert!(!buffer.append_window(6, &[6, 7, 8, 9], 0));
        assert_eq!(buffer.base_offset(), 0);
        assert_eq!(buffer.end_offset(), 6);

        buffer.set_reader_position(2);
        assert!(buffer.append_window(6, &[6, 7, 8, 9], 0));
        assert_eq!(buffer.base_offset(), 2);
        assert_eq!(buffer.end_offset(), 10);
        let mut bytes = [0; 8];
        assert_eq!(buffer.read_at(2, &mut bytes).unwrap(), 8);
        assert_eq!(bytes, [2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn buffered_ahead_tracks_the_decoder_read_position() {
        let buffer = SharedBuffer::with_capacity(100, 16);
        assert!(buffer.append_window(0, &[0; 12], 0));
        assert_eq!(buffer.buffered_ahead(), 12);

        let mut streaming = StreamingBuffer::new(buffer.clone());
        let mut bytes = [0; 5];
        assert_eq!(streaming.read(&mut bytes).unwrap(), 5);
        assert_eq!(buffer.buffered_ahead(), 7);

        assert!(buffer.append_window(12, &[0; 4], 0));
        assert_eq!(buffer.buffered_ahead(), 11);
    }

    #[test]
    fn concurrent_reader_cannot_overwrite_active_demand_position() {
        let buffer = SharedBuffer::with_capacity(100, 16);
        assert!(buffer.append_window(0, &[0; 12], 0));
        let mut active = StreamingBuffer::new(buffer.clone());
        let mut prepared = StreamingBuffer::new(buffer.clone());

        assert!(active.reader_cancellation().owns_demand());
        assert!(!prepared.reader_cancellation().owns_demand());
        assert_eq!(active.read(&mut [0; 5]).unwrap(), 5);
        assert_eq!(buffer.buffered_ahead(), 7);

        assert_eq!(prepared.seek(SeekFrom::Start(2)).unwrap(), 2);
        assert_eq!(buffer.buffered_ahead(), 7);

        active.reader_cancellation().cancel();
        assert!(prepared.reader_cancellation().activate());
        assert_eq!(buffer.buffered_ahead(), 10);
    }

    #[test]
    fn inactive_reader_cannot_replace_the_retained_window() {
        let buffer = SharedBuffer::with_capacity(100, 16);
        assert!(buffer.append_window(0, &[0; 12], 0));
        buffer.set_coordinator_active_for_test(true);
        let active = StreamingBuffer::new(buffer.clone());
        let mut prepared = StreamingBuffer::new(buffer.clone());

        let error = prepared.seek(SeekFrom::Start(40)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(buffer.requested_window().is_none());

        active.reader_cancellation().cancel();
        assert!(prepared.reader_cancellation().activate());
        assert_eq!(prepared.seek(SeekFrom::Start(40)).unwrap(), 40);
        assert!(
            buffer
                .requested_window()
                .is_some_and(|(_, offset)| offset == 40)
        );
    }

    #[test]
    fn remote_eof_is_independent_from_cache_finalization() {
        let buffer = SharedBuffer::with_capacity(12, 12);
        assert!(buffer.append_window(0, &[0; 12], 0));
        assert!(buffer.remote_eof_reached());
        assert!(!buffer.is_complete());
    }

    #[test]
    fn remote_eof_only_applies_while_reader_is_inside_the_eof_window() {
        let buffer = SharedBuffer::with_capacity(100, 8);
        assert!(buffer.append_window(92, &[0; 8], 0));

        assert!(!buffer.remote_eof_reached());
        buffer.set_reader_position(92);
        assert!(buffer.remote_eof_reached());
        buffer.set_reader_position(40);
        assert!(!buffer.remote_eof_reached());
    }

    #[test]
    fn stopped_coordinator_fails_unavailable_reads_instead_of_waiting_forever() {
        let buffer = SharedBuffer::with_capacity(100, 8);
        buffer.append(&[0; 4]);

        let error = buffer.read_at(4, &mut [0; 1]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn cache_progress_does_not_jump_to_a_sparse_decoder_window() {
        let buffer = SharedBuffer::with_capacity(100, 16);
        buffer.append(&[0; 10]);
        let epoch = buffer.request_window(80);
        buffer.set_reader_position(80);
        assert!(buffer.append_window(80, &[0; 8], epoch));

        assert_eq!(buffer.cached_bytes(), 10);
        assert!((buffer.progress() - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn coordinator_guard_clears_active_state_on_every_exit() {
        let buffer = SharedBuffer::new(100);
        buffer
            .inner
            .coordinator_active
            .store(true, Ordering::Release);
        drop(CoordinatorGuard {
            buffer: buffer.clone(),
            key: AudioCacheKey {
                song_id: 1,
                actual_quality: crate::domain::audio::QualityLevel::Standard,
            },
        });
        assert!(!buffer.inner.coordinator_active.load(Ordering::Acquire));
    }

    #[test]
    fn test_shared_buffer_read_at_available_data() {
        let buffer = SharedBuffer::new(100);
        buffer.append(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        let mut buf = [0u8; 5];
        let bytes_read = buffer.read_at(0, &mut buf).unwrap();
        assert_eq!(bytes_read, 5);
        assert_eq!(buf, [1, 2, 3, 4, 5]);

        let bytes_read = buffer.read_at(5, &mut buf).unwrap();
        assert_eq!(bytes_read, 5);
        assert_eq!(buf, [6, 7, 8, 9, 10]);
    }

    #[test]
    fn test_shared_buffer_read_at_partial() {
        let buffer = SharedBuffer::new(100);
        buffer.append(&[1, 2, 3]);

        // Request more than available
        let mut buf = [0u8; 10];
        let bytes_read = buffer.read_at(0, &mut buf).unwrap();
        assert_eq!(bytes_read, 3);
        assert_eq!(&buf[..3], &[1, 2, 3]);
    }

    #[test]
    fn test_shared_buffer_read_at_eof_when_complete() {
        let buffer = SharedBuffer::new(5);
        buffer.append(&[1, 2, 3, 4, 5]);
        buffer.mark_complete();

        // Reading at position beyond downloaded data should return EOF
        let mut buf = [0u8; 5];
        let bytes_read = buffer.read_at(5, &mut buf).unwrap();
        assert_eq!(bytes_read, 0); // EOF
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn persistent_cache_advances_beyond_a_full_playback_window() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const TOTAL_SIZE: u64 = 6 * MIB;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    loop {
                        let Ok(read) = socket.read(&mut chunk).await else {
                            return;
                        };
                        if read == 0 {
                            return;
                        }
                        request.extend_from_slice(&chunk[..read]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&request);
                    if request.starts_with("HEAD ") {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {TOTAL_SIZE}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }

                    let Some(range) = request.lines().find_map(|line| {
                        line.strip_prefix("Range: bytes=")
                            .or_else(|| line.strip_prefix("range: bytes="))
                    }) else {
                        return;
                    };
                    let range = range.trim();
                    let Some((start, end)) = range.split_once('-') else {
                        return;
                    };
                    let (Ok(start), Ok(end)) = (start.parse::<u64>(), end.parse::<u64>()) else {
                        return;
                    };
                    let len = end.saturating_sub(start).saturating_add(1) as usize;
                    let mut body = vec![0u8; len];
                    for (index, byte) in body.iter_mut().enumerate() {
                        let absolute = start + index as u64;
                        *byte = match absolute {
                            0 => b'I',
                            1 => b'D',
                            2 => b'3',
                            _ => (absolute % 251) as u8,
                        };
                    }
                    let response = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {len}\r\nContent-Range: bytes {start}-{end}/{TOTAL_SIZE}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
                    );
                    if socket.write_all(response.as_bytes()).await.is_ok() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        let _ = socket.write_all(&body).await;
                    }
                });
            }
        });

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cache_dir = std::env::temp_dir().join(format!(
            "rustle-cache-progress-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&cache_dir).unwrap();
        let controller = crate::audio::identity::PlaybackGenerationController::new();
        let context = controller.activate_generation();
        let buffer = start_buffer_download(
            format!("http://{address}/audio.mp3"),
            cache_dir.join("range-cache"),
            AudioCacheKey {
                song_id: unique as u64,
                actual_quality: crate::domain::audio::QualityLevel::Standard,
            },
            test_cache_store(),
            Some(320_000),
            StreamingIdentity::Playback(context),
            None,
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if buffer.cached_bytes() > buffer.downloaded()
                && buffer.cached_bytes() > buffer.policy().high_water_mark_bytes()
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "cache prefix did not advance beyond the retained window: cached={}, retained_end={}",
                buffer.cached_bytes(),
                buffer.downloaded()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        while !buffer.is_cache_finalized() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "cache did not finalize after its prefix reached the remote size"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let mut reader = StreamingBuffer::new(buffer.clone());
        let mut consumed = 0u64;
        let mut bytes = [0u8; 64 * 1024];
        while consumed < TOTAL_SIZE {
            let count = reader.read(&mut bytes).unwrap();
            assert!(count > 0, "finalized cache returned an early EOF");
            consumed += count as u64;
        }
        assert_eq!(consumed, TOTAL_SIZE);
        assert_eq!(reader.read(&mut bytes).unwrap(), 0);

        server.abort();
        drop(reader);
        buffer.inner.finalized_cache_reader.lock().take();
        drop(buffer);
        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn test_shared_buffer_cancel() {
        let buffer = SharedBuffer::new(100);
        assert!(!buffer.is_cancelled());

        buffer.cancel_coordinator();
        assert!(buffer.is_cancelled());

        // Read should return error when cancelled
        let mut buf = [0u8; 5];
        let result = buffer.read_at(0, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn reader_cancellation_wakes_only_that_reader_without_poisoning_shared_health() {
        let shared = SharedBuffer::new(100);
        shared.set_coordinator_active_for_test(true);
        let mut reader = StreamingBuffer::new(shared.clone());
        let cancellation = reader.reader_cancellation();
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = reader.read(&mut [0; 1]).map_err(|error| error.kind());
            let _ = result_tx.send(result);
        });

        cancellation.cancel();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_millis(500)).unwrap(),
            Err(io::ErrorKind::Interrupted)
        );
        assert_eq!(shared.health(), SharedBufferHealth::Refillable);
        assert!(!shared.is_cancelled());
    }

    #[test]
    fn shared_cancellation_wakes_all_reader_tokens() {
        let shared = SharedBuffer::new(100);
        shared.set_coordinator_active_for_test(true);
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        for _ in 0..2 {
            let mut reader = StreamingBuffer::new(shared.clone());
            let result_tx = result_tx.clone();
            std::thread::spawn(move || {
                let result = reader.read(&mut [0; 1]).map_err(|error| error.kind());
                let _ = result_tx.send(result);
            });
        }
        drop(result_tx);

        shared.cancel_coordinator();
        for _ in 0..2 {
            assert_eq!(
                result_rx.recv_timeout(Duration::from_millis(500)).unwrap(),
                Err(io::ErrorKind::Interrupted)
            );
        }
        assert_eq!(shared.health(), SharedBufferHealth::Cancelled);
    }

    #[test]
    fn test_shared_buffer_error() {
        let buffer = SharedBuffer::new(100);
        buffer.set_error(PlaybackError::StreamingFailed("Test error".to_string()));

        // Read should return error
        let mut buf = [0u8; 5];
        let result = buffer.read_at(0, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_shared_buffer_cache_progress() {
        let buffer = SharedBuffer::new(100);
        assert_eq!(buffer.progress(), 0.0);

        buffer.append(&[0; 25]);
        assert!((buffer.progress() - 0.25).abs() < 0.001);

        buffer.append(&[0; 25]);
        assert!((buffer.progress() - 0.50).abs() < 0.001);

        buffer.append(&[0; 50]);
        assert!((buffer.progress() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_shared_buffer_set_total_size() {
        let buffer = SharedBuffer::new(0);
        assert_eq!(buffer.total_size(), 0);

        buffer.set_total_size(1000);
        assert_eq!(buffer.total_size(), 1000);
    }

    #[test]
    fn test_streaming_buffer_read() {
        let shared = SharedBuffer::new(100);
        shared.append(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        let mut streaming = StreamingBuffer::new(shared);

        let mut buf = [0u8; 5];
        let bytes_read = streaming.read(&mut buf).unwrap();
        assert_eq!(bytes_read, 5);
        assert_eq!(buf, [1, 2, 3, 4, 5]);

        // Position should advance
        let bytes_read = streaming.read(&mut buf).unwrap();
        assert_eq!(bytes_read, 5);
        assert_eq!(buf, [6, 7, 8, 9, 10]);
    }

    #[test]
    fn test_streaming_buffer_seek() {
        let shared = SharedBuffer::new(100);
        shared.append(&[0; 50]);
        shared.set_total_size(100);

        let mut streaming = StreamingBuffer::new(shared);

        // Seek from start
        let pos = streaming.seek(SeekFrom::Start(10)).unwrap();
        assert_eq!(pos, 10);

        // Seek from current
        let pos = streaming.seek(SeekFrom::Current(5)).unwrap();
        assert_eq!(pos, 15);

        // Seek from current (negative)
        let pos = streaming.seek(SeekFrom::Current(-5)).unwrap();
        assert_eq!(pos, 10);

        // Seek from end
        let pos = streaming.seek(SeekFrom::End(-10)).unwrap();
        assert_eq!(pos, 90);
    }

    #[test]
    fn test_parse_content_range_accepts_whitespace_and_rejects_invalid_ranges() {
        assert_eq!(
            parse_content_range(" bytes   0-0/123 "),
            Some(ContentRange {
                start: 0,
                end: 0,
                total: 123,
            })
        );
        assert!(parse_content_range("items 0-0/123").is_none());
        assert!(parse_content_range("bytes 1-0/123").is_none());
        assert!(parse_content_range("bytes 0-123/123").is_none());
        assert!(parse_content_range("bytes 0-0/*").is_none());
        assert!(parse_content_range("bytes 0-0/0").is_none());
        assert!(parse_content_range("bytes 0-0/123 extra").is_none());
    }

    #[test]
    fn test_streaming_buffer_seek_rejects_underflow_and_overflow() {
        let shared = SharedBuffer::new(100);
        let mut streaming = StreamingBuffer::new(shared);

        assert!(streaming.seek(SeekFrom::Current(-1)).is_err());
        assert!(streaming.seek(SeekFrom::Start(u64::MAX)).is_ok());
        assert!(streaming.seek(SeekFrom::Current(1)).is_err());
    }
    #[test]
    fn test_extract_extension_from_url() {
        assert_eq!(
            extract_extension_from_url("http://example.com/song.mp3"),
            Some("mp3".to_string())
        );
        assert_eq!(
            extract_extension_from_url("http://example.com/song.flac?token=xxx"),
            Some("flac".to_string())
        );
        assert_eq!(
            extract_extension_from_url("http://example.com/song.m4a#section"),
            Some("m4a".to_string())
        );
        assert_eq!(
            extract_extension_from_url("http://example.com/song.txt"),
            None
        );
        assert_eq!(extract_extension_from_url("http://example.com/song"), None);
    }

    #[test]
    fn test_content_type_to_extension() {
        assert_eq!(
            content_type_to_extension("audio/mpeg"),
            Some("mp3".to_string())
        );
        assert_eq!(
            content_type_to_extension("audio/flac"),
            Some("flac".to_string())
        );
        assert_eq!(
            content_type_to_extension("audio/mp4"),
            Some("m4a".to_string())
        );
        assert_eq!(
            content_type_to_extension("audio/ogg"),
            Some("ogg".to_string())
        );
        assert_eq!(
            content_type_to_extension("audio/wav"),
            Some("wav".to_string())
        );
        assert_eq!(content_type_to_extension("text/plain"), None);
        assert_eq!(
            content_type_to_extension("audio/mpeg; charset=utf-8"),
            Some("mp3".to_string())
        );
    }
}
