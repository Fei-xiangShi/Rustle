// src/app/state.rs
//! Application state definitions

use iced::time::Instant;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::api::{
    AlbumSummary, ArtistSummary, NcmClient, PlaylistSummary, RadioSummary, Track, VideoSummary,
};
use crate::app::SettingsSection;
use crate::audio::{
    AudioAnalysisData, AudioDevice, AudioProcessingChain, PlaybackInfo, PlaybackStatus,
};
use crate::database::{Database, DbPlaybackState, DbPlaylist, DbSong, DbWatchedFolder};
use crate::features::import::{CoverCache, FolderWatcher, ScanHandle, ScanProgress, ScanState};
use crate::i18n::Locale;
use crate::platform::discord::DiscordPresence;
use crate::platform::media_controls::{MediaCommand, MediaHandle};
use crate::ui::animation::{HoverAnimations, SingleHoverAnimation, SmoothScrollState};
use crate::ui::components::{ImportingPlaylist, NavItem};
use crate::ui::effects::background::LyricsBackgroundProgram;
use crate::ui::effects::textured_background::TexturedBackgroundProgram;
use crate::ui::overlay::OverlayEntry;
use crate::ui::pages;
use crate::ui::widgets::Toast;
use crate::utils::Source;

fn audio_output_device_options(devices: &[AudioDevice]) -> Vec<(String, String)> {
    let mut duplicate_counts = std::collections::HashMap::new();
    for device in devices {
        *duplicate_counts
            .entry(device.name.as_str())
            .or_insert(0_usize) += 1;
    }

    devices
        .iter()
        .map(|device| {
            let label = if duplicate_counts.get(device.name.as_str()) == Some(&1) {
                device.name.clone()
            } else {
                format!("{} ({})", device.name, device.id)
            };
            (device.id.clone(), label)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Unified image-handle cache
// ---------------------------------------------------------------------------

use std::collections::HashMap as StateMap;

type ImageStateKey = (crate::image::ImageKind, u64, crate::image::ImageVariant);

/// Pre-loaded image data keyed by `(ImageKind, id, ImageVariant)`.
///
/// Populated by the unified image pipeline (`handle_image`) so that views
/// never touch disk — they only borrow an `Option<&image::Handle>`.
#[derive(Debug, Default)]
pub struct ImageState {
    pub failures: StateMap<ImageStateKey, std::time::Instant>,
    pub song_lru: VecDeque<ImageStateKey>,
    pub artwork_epoch: u64,
    /// Completion identity is unique even when a source changes within one route.
    next_image_request: u64,
    pub entries: StateMap<ImageStateKey, ImageEntry>,
    pub inflight: StateMap<ImageStateKey, ImageInFlight>,
    pub pending: VecDeque<ImageRequest>,
    pub queued: std::collections::HashSet<ImageStateKey>,
    /// Latest remote source known for a logical resource. This survives the
    /// transition from a remote model URL to a local cached Thumbnail path.
    pub known_sources: StateMap<(crate::image::ImageKind, u64), String>,
    /// Current-song metadata probes used only when persisted state retained a
    /// local Thumbnail path but no remote source URL for a Hero request.
    pub resolving_sources: std::collections::HashSet<(crate::image::ImageKind, u64)>,
    pub source_resolution_attempts: std::collections::HashSet<(crate::image::ImageKind, u64)>,
    /// Monotonic generation for page-owned image work.
    pub generation: u64,
    /// Monotonic generation for the currently published virtual-list range.
    /// This is separate from `generation` so scrolling cannot make an old
    /// viewport completion look like a current page completion.
    pub viewport_generation: u64,
}

#[derive(Debug, Clone)]
pub struct ImageEntry {
    /// Actual file used for extraction; distinct from the stable render identity.
    pub backing_file: Option<PathBuf>,
    pub palette: Option<crate::utils::ColorPalette>,
    pub artwork: Option<std::sync::Arc<image::DynamicImage>>,
    pub path: PathBuf,
    pub handle: iced::widget::image::Handle,
    pub playlist_footer_handle: Option<iced::widget::image::Handle>,
    pub dimensions: Option<(u32, u32)>,
    /// Present for current remote derivatives. `None` marks a local or legacy
    /// fallback that may remain visible while an exact remote variant loads.
    pub source_url: Option<String>,
    pub source_url_digest: Option<u64>,
    pub processing_version: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub kind: crate::image::ImageKind,
    pub id: u64,
    pub variant: crate::image::ImageVariant,
    pub url: String,
    pub generation: u64,
    pub scope: ImageRequestScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRequestScope {
    Global,
    Page,
    Viewport,
}

#[derive(Debug)]
pub struct ImageInFlight {
    /// Completion identity captured by the already-running task.
    pub generation: u64,
    pub handle: iced::task::Handle,
    pub scope: ImageRequestScope,
    /// Cancellation ownership may be promoted without changing the completion
    /// identity emitted by an already-running task.
    pub ownership: ImageRequestScope,
    /// Remote source captured when the request started. Completion uses this
    /// to install the exact cache identity without consulting mutable page
    /// state.
    pub source_url: String,
}

impl ImageState {
    pub fn insert_prepared(
        &mut self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
        mut entry: ImageEntry,
    ) {
        let key = (kind, id, variant);
        // Identical content keeps its GPU handle and prepared background identity.
        if matches!(
            kind,
            crate::image::ImageKind::SongCover | crate::image::ImageKind::LocalSongCover
        ) && let Some(previous) = self
            .entries
            .get(&key)
            .filter(|previous| previous.path == entry.path)
        {
            entry.handle = previous.handle.clone();
            entry.artwork = previous.artwork.clone();
            entry.palette = previous.palette.clone();
        }
        if matches!(
            kind,
            crate::image::ImageKind::SongCover | crate::image::ImageKind::LocalSongCover
        ) {
            if variant == crate::image::ImageVariant::Hero {
                self.entries.retain(|candidate, _| {
                    candidate.2 != crate::image::ImageVariant::Hero
                        || !matches!(
                            candidate.0,
                            crate::image::ImageKind::SongCover
                                | crate::image::ImageKind::LocalSongCover
                        )
                        || *candidate == key
                });
                self.song_lru.retain(|candidate| {
                    candidate.2 != crate::image::ImageVariant::Hero || *candidate == key
                });
            }
            self.song_lru.retain(|existing| *existing != key);
            self.song_lru.push_back(key);
            while self.song_lru.len() > 128 {
                if let Some(old) = self.song_lru.pop_front() {
                    self.entries.remove(&old);
                }
            }
        }
        self.failures.remove(&key);
        self.entries.insert(key, entry);
    }

    pub fn get(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
    ) -> Option<&iced::widget::image::Handle> {
        self.get_variant(kind, id, crate::image::ImageVariant::Thumbnail)
    }

    pub fn get_variant(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
    ) -> Option<&iced::widget::image::Handle> {
        self.entries
            .get(&(kind, id, variant))
            .map(|entry| &entry.handle)
    }

    /// Read the requested role, falling back to Thumbnail while a larger
    /// derivative is pending or unavailable.
    pub fn get_with_fallback(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
    ) -> Option<&iced::widget::image::Handle> {
        self.get_variant(kind, id, variant)
            .or_else(|| self.get(kind, id))
    }

    pub fn get_playlist_footer(&self, id: u64) -> Option<&iced::widget::image::Handle> {
        self.entries
            .get(&(
                crate::image::ImageKind::PlaylistCover,
                id,
                crate::image::ImageVariant::Thumbnail,
            ))
            .and_then(|entry| entry.playlist_footer_handle.as_ref())
    }

    pub fn image_data(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
    ) -> Option<(&PathBuf, u32, u32)> {
        self.image_data_variant(kind, id, crate::image::ImageVariant::Thumbnail)
    }

    pub fn image_data_variant(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
    ) -> Option<(&PathBuf, u32, u32)> {
        self.entries.get(&(kind, id, variant)).map(|entry| {
            let (width, height) = entry.dimensions.unwrap_or((0, 0));
            (&entry.path, width, height)
        })
    }

    #[cfg(test)]
    pub fn insert_path(&mut self, kind: crate::image::ImageKind, id: u64, path: PathBuf) {
        self.insert_variant_path(kind, id, crate::image::ImageVariant::Thumbnail, path);
    }

    #[cfg(test)]
    pub fn insert_variant_path(
        &mut self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
        path: PathBuf,
    ) {
        self.insert_path_variant(kind, id, variant, path, None);
    }

    #[cfg(test)]
    fn insert_path_variant(
        &mut self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
        path: PathBuf,
        source_url: Option<String>,
    ) {
        let dimensions = image_dimensions(&path);
        let handle = iced::widget::image::Handle::from_path(path.clone());
        let playlist_footer_handle = if kind == crate::image::ImageKind::PlaylistCover
            && variant == crate::image::ImageVariant::Thumbnail
        {
            image::open(&path).ok().map(|source| {
                let processed =
                    crate::ui::effects::image_processing::process_image_for_playlist_footer(
                        &source,
                    );
                iced::widget::image::Handle::from_rgba(
                    processed.width,
                    processed.height,
                    processed.data,
                )
            })
        } else {
            None
        };
        self.entries.insert(
            (kind, id, variant),
            ImageEntry {
                backing_file: Some(path.clone()),
                palette: None,
                artwork: None,
                path,
                handle,
                playlist_footer_handle,
                dimensions,
                source_url_digest: source_url.as_deref().map(crate::image::source_url_digest),
                processing_version: source_url
                    .as_ref()
                    .map(|_| crate::image::IMAGE_PROCESSING_VERSION),
                source_url,
            },
        );
        if let Some(request) = self.inflight.remove(&(kind, id, variant)) {
            request.handle.abort();
        }
        self.queued.remove(&(kind, id, variant));
    }

    pub fn clear_variant_inflight(
        &mut self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
    ) -> bool {
        let Some(request) = self.inflight.remove(&(kind, id, variant)) else {
            return false;
        };
        self.queued.remove(&(kind, id, variant));
        // Global promotion preserves running identity, but its latest source
        // must be scheduled immediately after that older operation completes.
        if request.ownership == ImageRequestScope::Global
            && let Some(source) = self.known_sources.get(&(kind, id)).cloned()
            && source != request.source_url
        {
            self.failures.remove(&(kind, id, variant));
            return self.enqueue_variant_with_scope(
                kind,
                id,
                variant,
                source,
                ImageRequestScope::Global,
            );
        }
        false
    }

    /// Drop a cache entry and every request for a resource whose remote
    /// identity is no longer stable. Dynamic resources (such as the synthetic
    /// Daily Recommend playlist) reuse an application-level ID while their
    /// cover URL changes between refreshes.
    pub fn invalidate(&mut self, kind: crate::image::ImageKind, id: u64) {
        self.entries
            .retain(|(entry_kind, entry_id, _), _| (*entry_kind, *entry_id) != (kind, id));
        let mut retained = StateMap::default();
        for (key, request) in self.inflight.drain() {
            if (key.0, key.1) == (kind, id) {
                request.handle.abort();
            } else {
                retained.insert(key, request);
            }
        }
        self.inflight = retained;
        self.pending
            .retain(|request| (request.kind, request.id) != (kind, id));
        self.queued
            .retain(|(entry_kind, entry_id, _)| (*entry_kind, *entry_id) != (kind, id));
        self.failures.retain(|(k, i, _), _| (*k, *i) != (kind, id));
        self.known_sources.remove(&(kind, id));
        self.resolving_sources.remove(&(kind, id));
        self.source_resolution_attempts.remove(&(kind, id));
    }

    /// Keep the visible handle until a replacement succeeds. Pending work for
    /// the previous content is cancelled, without evicting unrelated resources.
    pub fn refresh_retaining_image(&mut self, kind: crate::image::ImageKind, id: u64) {
        let retained: Vec<_> = self
            .entries
            .iter()
            .filter(|(key, _)| key.0 == kind && key.1 == id)
            .map(|(key, entry)| (*key, entry.clone()))
            .collect();
        let source = self.known_sources.get(&(kind, id)).cloned();
        self.invalidate(kind, id);
        for (key, mut entry) in retained {
            entry.processing_version = None;
            self.entries.insert(key, entry);
        }
        if let Some(source) = source {
            self.known_sources.insert((kind, id), source);
        }
    }

    #[cfg(test)]
    pub fn is_current_inflight(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        generation: u64,
        scope: ImageRequestScope,
    ) -> bool {
        self.is_current_variant_inflight(
            kind,
            id,
            crate::image::ImageVariant::Thumbnail,
            generation,
            scope,
        )
    }

    pub fn is_current_variant_inflight(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
        generation: u64,
        scope: ImageRequestScope,
    ) -> bool {
        self.inflight
            .get(&(kind, id, variant))
            .is_some_and(|request| request.generation == generation && request.scope == scope)
    }

    #[cfg(test)]
    pub fn is_inflight(&self, kind: crate::image::ImageKind, id: u64) -> bool {
        self.is_variant_inflight(kind, id, crate::image::ImageVariant::Thumbnail)
    }

    pub fn is_variant_inflight(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
    ) -> bool {
        self.inflight.contains_key(&(kind, id, variant))
    }

    #[cfg(test)]
    pub fn is_queued(&self, kind: crate::image::ImageKind, id: u64) -> bool {
        self.is_variant_queued(kind, id, crate::image::ImageVariant::Thumbnail)
    }

    #[cfg(test)]
    pub fn is_variant_queued(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
    ) -> bool {
        self.queued.contains(&(kind, id, variant))
    }

    #[cfg(test)]
    pub fn enqueue(&mut self, kind: crate::image::ImageKind, id: u64, url: String) -> bool {
        self.enqueue_with_scope(kind, id, url, ImageRequestScope::Page)
    }

    #[cfg(test)]
    pub fn enqueue_with_scope(
        &mut self,
        kind: crate::image::ImageKind,
        id: u64,
        url: String,
        scope: ImageRequestScope,
    ) -> bool {
        self.enqueue_variant_with_scope(kind, id, crate::image::ImageVariant::Thumbnail, url, scope)
    }

    pub fn enqueue_variant_with_scope(
        &mut self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
        url: String,
        scope: ImageRequestScope,
    ) -> bool {
        let key = (kind, id, variant);
        if url.is_empty() {
            return false;
        }
        if self
            .known_sources
            .get(&(kind, id))
            .is_some_and(|previous| previous != &url)
        {
            self.failures.remove(&key);
        }
        if self
            .failures
            .get(&key)
            .is_some_and(|failed| failed.elapsed() < std::time::Duration::from_secs(30))
        {
            return false;
        }
        self.known_sources.insert((kind, id), url.clone());
        if self.has_current_remote_variant(kind, id, variant, &url) {
            return false;
        }

        if scope == ImageRequestScope::Global {
            if let Some(request) = self
                .pending
                .iter_mut()
                .find(|request| (request.kind, request.id, request.variant) == key)
            {
                request.url = url;
                request.scope = ImageRequestScope::Global;
                return false;
            }
            if let Some(request) = self.inflight.get_mut(&key) {
                request.ownership = ImageRequestScope::Global;
                return false;
            }
        }

        let mut effective_scope = scope;
        if let Some(index) = self
            .pending
            .iter()
            .position(|request| (request.kind, request.id, request.variant) == key)
        {
            let request = &mut self.pending[index];
            if request.url == url {
                if scope == ImageRequestScope::Global {
                    request.scope = ImageRequestScope::Global;
                }
                return false;
            }

            let replaced = self
                .pending
                .remove(index)
                .expect("located pending image request must still exist");
            self.queued.remove(&key);
            if replaced.scope == ImageRequestScope::Global {
                effective_scope = ImageRequestScope::Global;
            }
        }

        if let Some(request) = self.inflight.get(&key)
            && request.source_url == url
        {
            if scope == ImageRequestScope::Global
                && let Some(request) = self.inflight.get_mut(&key)
            {
                request.ownership = ImageRequestScope::Global;
            }
            return false;
        }

        if let Some(request) = self.inflight.remove(&key) {
            if request.ownership == ImageRequestScope::Global {
                effective_scope = ImageRequestScope::Global;
            }
            request.handle.abort();
        }

        if self.inflight.contains_key(&key) || self.queued.contains(&key) {
            return false;
        }

        let generation = self.next_image_request;
        self.next_image_request = self.next_image_request.wrapping_add(1);
        self.pending.push_back(ImageRequest {
            kind,
            id,
            variant,
            url,
            generation,
            scope: effective_scope,
        });
        self.queued.insert(key);
        true
    }

    pub fn has_current_remote_variant(
        &self,
        kind: crate::image::ImageKind,
        id: u64,
        variant: crate::image::ImageVariant,
        source_url: &str,
    ) -> bool {
        self.entries.get(&(kind, id, variant)).is_some_and(|entry| {
            entry.source_url_digest == Some(crate::image::source_url_digest(source_url))
                && entry.processing_version == Some(crate::image::IMAGE_PROCESSING_VERSION)
        })
    }

    pub fn begin_source_resolution(&mut self, kind: crate::image::ImageKind, id: u64) -> bool {
        let key = (kind, id);
        if !self.source_resolution_attempts.insert(key) {
            return false;
        }
        self.resolving_sources.insert(key)
    }

    pub fn finish_source_resolution(&mut self, kind: crate::image::ImageKind, id: u64) {
        self.resolving_sources.remove(&(kind, id));
    }

    #[cfg(test)]
    pub fn mark_inflight(
        &mut self,
        kind: crate::image::ImageKind,
        id: u64,
        generation: u64,
        scope: ImageRequestScope,
        handle: iced::task::Handle,
    ) {
        self.insert_inflight(
            (kind, id, crate::image::ImageVariant::Thumbnail),
            generation,
            scope,
            String::new(),
            handle,
        );
    }

    pub fn mark_request_inflight(&mut self, request: &ImageRequest, handle: iced::task::Handle) {
        self.insert_inflight(
            (request.kind, request.id, request.variant),
            request.generation,
            request.scope,
            request.url.clone(),
            handle,
        );
    }

    fn insert_inflight(
        &mut self,
        key: ImageStateKey,
        generation: u64,
        scope: ImageRequestScope,
        source_url: String,
        handle: iced::task::Handle,
    ) {
        self.queued.remove(&key);
        self.inflight.insert(
            key,
            ImageInFlight {
                generation,
                handle,
                scope,
                ownership: scope,
                source_url,
            },
        );
    }

    /// Reconcile viewport-owned work with the latest visible range.
    ///
    /// Requests that still belong to the new range keep running. Only work for
    /// images that actually left the range is aborted or removed. This avoids
    /// restarting overlapping cover downloads at every virtual-list row
    /// boundary during smooth scrolling.
    pub fn reconcile_viewport_requests(
        &mut self,
        desired: &std::collections::HashSet<(crate::image::ImageKind, u64)>,
    ) {
        let mut retained = StateMap::default();
        for (key, request) in self.inflight.drain() {
            if request.ownership == ImageRequestScope::Viewport
                && !desired.contains(&(key.0, key.1))
            {
                request.handle.abort();
            } else {
                retained.insert(key, request);
            }
        }
        self.inflight = retained;
        self.pending.retain(|request| {
            request.scope != ImageRequestScope::Viewport
                || desired.contains(&(request.kind, request.id))
        });
        self.rebuild_queued_keys();
        self.viewport_generation = self.viewport_generation.wrapping_add(1);
    }

    /// Cancel page/viewport image work while retaining global work and
    /// successful cache entries.
    pub fn cancel_pending_and_inflight(&mut self) {
        let mut retained = StateMap::default();
        for (key, request) in self.inflight.drain() {
            if request.ownership == ImageRequestScope::Global {
                retained.insert(key, request);
            } else {
                request.handle.abort();
            }
        }
        self.inflight = retained;
        self.pending
            .retain(|request| request.scope == ImageRequestScope::Global);
        self.rebuild_queued_keys();
        self.generation = self.generation.wrapping_add(1);
        self.viewport_generation = self.viewport_generation.wrapping_add(1);
    }

    pub fn pop_pending(&mut self) -> Option<ImageRequest> {
        let request = self.pending.pop_front()?;
        self.queued
            .remove(&(request.kind, request.id, request.variant));
        Some(request)
    }

    fn rebuild_queued_keys(&mut self) {
        self.queued.clear();
        self.queued.extend(
            self.pending
                .iter()
                .map(|request| (request.kind, request.id, request.variant)),
        );
        self.queued.extend(self.inflight.keys().copied());
    }
}

#[cfg(test)]
mod image_state_tests {
    use super::*;

    #[test]
    fn promoted_request_schedules_latest_source_on_completion() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::SongCover;
        let variant = crate::image::ImageVariant::Thumbnail;
        assert!(state.enqueue_with_scope(kind, 42, "https://old".into(), ImageRequestScope::Page));
        let old = state.pop_pending().unwrap();
        let (_, handle) = iced::Task::<()>::none().abortable();
        state.mark_request_inflight(&old, handle);
        assert!(!state.enqueue_with_scope(
            kind,
            42,
            "https://new".into(),
            ImageRequestScope::Global
        ));
        state.cancel_pending_and_inflight();
        assert!(state.is_current_variant_inflight(kind, 42, variant, old.generation, old.scope));
        assert!(state.clear_variant_inflight(kind, 42, variant));
        let replacement = state.pop_pending().unwrap();
        assert_eq!(replacement.url, "https://new");
        assert_eq!(replacement.scope, ImageRequestScope::Global);
        assert_ne!(replacement.generation, old.generation);
    }

    #[test]
    fn clearing_audio_sources_rejects_prior_global_artwork() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::SongCover;
        assert!(state.enqueue_with_scope(
            kind,
            42,
            "https://example.invalid/cover".into(),
            ImageRequestScope::Global
        ));
        let old = state.pop_pending().unwrap();
        state.artwork_epoch += 1;
        state.invalidate(kind, 42);
        assert!(state.enqueue_with_scope(
            kind,
            42,
            "https://example.invalid/cover".into(),
            ImageRequestScope::Global
        ));
        let new = state.pop_pending().unwrap();
        let (_, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        state.mark_request_inflight(&new, handle);
        assert!(!state.is_current_variant_inflight(
            kind,
            42,
            old.variant,
            old.generation,
            old.scope
        ));
        assert!(state.is_current_variant_inflight(
            kind,
            42,
            new.variant,
            new.generation,
            new.scope
        ));
    }

    #[test]
    fn failed_artwork_is_not_retried_on_every_ui_tick() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::LocalSongCover;
        state.failures.insert(
            (kind, 42, crate::image::ImageVariant::Thumbnail),
            std::time::Instant::now(),
        );
        assert!(!state.enqueue_with_scope(
            kind,
            42,
            "missing.mp3".into(),
            ImageRequestScope::Global
        ));
        state.invalidate(kind, 42);
        assert!(state.enqueue_with_scope(
            kind,
            42,
            "missing.mp3".into(),
            ImageRequestScope::Global
        ));
    }

    #[test]
    fn song_artwork_memory_is_bounded() {
        let mut state = ImageState::default();
        for id in 0..140 {
            let entry = crate::image::artwork::prepare(
                crate::image::ImageKind::SongCover,
                crate::image::ImageVariant::Thumbnail,
                PathBuf::from(format!("artwork://{id}")),
                format!("https://example.invalid/{id}"),
                image::DynamicImage::new_rgb8(1, 1),
            );
            state.insert_prepared(
                crate::image::ImageKind::SongCover,
                id,
                crate::image::ImageVariant::Thumbnail,
                entry,
            );
        }
        assert_eq!(state.song_lru.len(), 128);
        assert_eq!(state.entries.len(), 128);
        assert!(state.get(crate::image::ImageKind::SongCover, 0).is_none());
        assert!(state.get(crate::image::ImageKind::SongCover, 139).is_some());
    }

    #[test]
    fn cancelling_image_work_aborts_active_task_and_advances_generation() {
        let mut state = ImageState::default();
        assert!(state.enqueue(
            crate::image::ImageKind::SongCover,
            7,
            "https://example.invalid/cover.jpg".to_string()
        ));

        let request = state.pop_pending().expect("queued image request");
        let (_task, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let observer = handle.clone();
        state.mark_inflight(
            request.kind,
            request.id,
            request.generation,
            request.scope,
            handle,
        );

        assert!(state.is_current_inflight(request.kind, request.id, 0, request.scope));
        state.cancel_pending_and_inflight();

        assert!(observer.is_aborted());
        assert_eq!(state.generation, 1);
        assert!(!state.is_current_inflight(request.kind, request.id, 0, request.scope));
    }

    #[test]
    fn cancelling_image_work_keeps_successful_cache_entries() {
        let mut state = ImageState::default();
        let key = (crate::image::ImageKind::SongCover, 8);
        state.insert_path(key.0, key.1, PathBuf::from("cached-cover.jpg"));

        state.cancel_pending_and_inflight();

        assert!(state.get(key.0, key.1).is_some());
    }

    #[test]
    fn invalidating_image_work_drops_dynamic_cache_entries() {
        let mut state = ImageState::default();
        let key = (crate::image::ImageKind::PlaylistCover, 0);

        assert!(state.enqueue_with_scope(
            key.0,
            key.1,
            "https://example.invalid/daily-cover.jpg".to_string(),
            ImageRequestScope::Page,
        ));
        let request = state.pop_pending().expect("daily cover request");
        let (_task, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let observer = handle.clone();
        state.mark_inflight(
            request.kind,
            request.id,
            request.generation,
            request.scope,
            handle,
        );

        state.invalidate(key.0, key.1);

        assert!(observer.is_aborted());
        assert!(!state.is_inflight(key.0, key.1));
        assert!(!state.is_queued(key.0, key.1));

        state.insert_path(key.0, key.1, PathBuf::from("daily-cover.jpg"));

        state.invalidate(key.0, key.1);

        assert!(state.get(key.0, key.1).is_none());
    }

    #[test]
    fn route_cancellation_keeps_global_image_work() {
        let mut state = ImageState::default();
        let key = (crate::image::ImageKind::UserAvatar, 10);
        assert!(state.enqueue_with_scope(
            key.0,
            key.1,
            "https://example.invalid/avatar.jpg".to_string(),
            ImageRequestScope::Global,
        ));
        let request = state.pop_pending().expect("global image request");
        let (_task, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let observer = handle.clone();
        state.mark_inflight(
            request.kind,
            request.id,
            request.generation,
            request.scope,
            handle,
        );

        state.cancel_pending_and_inflight();

        assert!(!observer.is_aborted());
        assert!(state.is_current_inflight(key.0, key.1, 0, ImageRequestScope::Global,));
    }

    #[test]
    fn cancelling_viewport_work_keeps_page_work() {
        let mut state = ImageState::default();
        let page_key = (crate::image::ImageKind::PlaylistCover, 1);
        let viewport_key = (crate::image::ImageKind::SongCover, 2);

        assert!(state.enqueue_with_scope(
            page_key.0,
            page_key.1,
            "https://example.invalid/page.jpg".to_string(),
            ImageRequestScope::Page,
        ));
        assert!(state.enqueue_with_scope(
            viewport_key.0,
            viewport_key.1,
            "https://example.invalid/row.jpg".to_string(),
            ImageRequestScope::Viewport,
        ));

        let page_request = state.pop_pending().expect("page request");
        let viewport_request = state.pop_pending().expect("viewport request");
        let (_page_task, page_handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let (_viewport_task, viewport_handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let page_observer = page_handle.clone();
        let viewport_observer = viewport_handle.clone();
        state.mark_inflight(
            page_request.kind,
            page_request.id,
            page_request.generation,
            page_request.scope,
            page_handle,
        );
        state.mark_inflight(
            viewport_request.kind,
            viewport_request.id,
            viewport_request.generation,
            viewport_request.scope,
            viewport_handle,
        );

        state.reconcile_viewport_requests(&std::collections::HashSet::new());

        assert!(!page_observer.is_aborted());
        assert!(viewport_observer.is_aborted());
        assert!(state.is_inflight(page_key.0, page_key.1));
        assert!(!state.is_inflight(viewport_key.0, viewport_key.1));
        assert_eq!(state.viewport_generation, 1);

        assert!(state.enqueue_with_scope(
            viewport_key.0,
            viewport_key.1,
            "https://example.invalid/row.jpg".to_string(),
            ImageRequestScope::Viewport,
        ));
        assert_ne!(
            state
                .pop_pending()
                .expect("replacement viewport request")
                .generation,
            viewport_request.generation
        );
    }

    #[test]
    fn reconciling_viewport_work_keeps_overlapping_requests() {
        let mut state = ImageState::default();
        let retained_inflight = (crate::image::ImageKind::SongCover, 11);
        let dropped_inflight = (crate::image::ImageKind::SongCover, 12);
        let retained_pending = (crate::image::ImageKind::SongCover, 13);
        let dropped_pending = (crate::image::ImageKind::SongCover, 14);

        for key in [
            retained_inflight,
            dropped_inflight,
            retained_pending,
            dropped_pending,
        ] {
            assert!(state.enqueue_with_scope(
                key.0,
                key.1,
                format!("https://example.invalid/{}.jpg", key.1),
                ImageRequestScope::Viewport,
            ));
        }

        let retained_request = state.pop_pending().expect("retained in-flight request");
        let dropped_request = state.pop_pending().expect("dropped in-flight request");
        let (_retained_task, retained_handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let (_dropped_task, dropped_handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let retained_observer = retained_handle.clone();
        let dropped_observer = dropped_handle.clone();
        state.mark_inflight(
            retained_request.kind,
            retained_request.id,
            retained_request.generation,
            retained_request.scope,
            retained_handle,
        );
        state.mark_inflight(
            dropped_request.kind,
            dropped_request.id,
            dropped_request.generation,
            dropped_request.scope,
            dropped_handle,
        );

        let desired = std::collections::HashSet::from([retained_inflight, retained_pending]);
        state.reconcile_viewport_requests(&desired);

        assert!(!retained_observer.is_aborted());
        assert!(dropped_observer.is_aborted());
        assert!(state.is_inflight(retained_inflight.0, retained_inflight.1));
        assert!(!state.is_inflight(dropped_inflight.0, dropped_inflight.1));
        assert!(state.is_queued(retained_pending.0, retained_pending.1));
        assert!(!state.is_queued(dropped_pending.0, dropped_pending.1));
        assert_eq!(state.viewport_generation, 1);
    }

    #[test]
    fn completion_identity_includes_scope() {
        let mut state = ImageState::default();
        let key = (crate::image::ImageKind::SongCover, 9);
        assert!(state.enqueue_with_scope(
            key.0,
            key.1,
            "https://example.invalid/row.jpg".to_string(),
            ImageRequestScope::Viewport,
        ));
        let request = state.pop_pending().expect("viewport request");
        let (_task, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        state.mark_inflight(
            request.kind,
            request.id,
            request.generation,
            request.scope,
            handle,
        );

        assert!(state.is_current_inflight(
            key.0,
            key.1,
            request.generation,
            ImageRequestScope::Viewport,
        ));
        assert!(!state.is_current_inflight(
            key.0,
            key.1,
            request.generation,
            ImageRequestScope::Page,
        ));
    }

    #[test]
    fn pending_viewport_request_is_promoted_to_global_ownership() {
        let mut state = ImageState::default();
        let key = (crate::image::ImageKind::SongCover, 19);
        assert!(state.enqueue_with_scope(
            key.0,
            key.1,
            "https://example.invalid/row.jpg".to_string(),
            ImageRequestScope::Viewport,
        ));

        assert!(!state.enqueue_with_scope(
            key.0,
            key.1,
            "https://example.invalid/current.jpg".to_string(),
            ImageRequestScope::Global,
        ));
        state.reconcile_viewport_requests(&std::collections::HashSet::new());

        let request = state.pop_pending().expect("promoted global request");
        assert_eq!(request.generation, 0);
        assert_eq!(request.scope, ImageRequestScope::Global);
        assert_eq!(request.url, "https://example.invalid/current.jpg");
    }

    #[test]
    fn inflight_viewport_request_keeps_completion_identity_after_global_promotion() {
        let mut state = ImageState::default();
        let key = (crate::image::ImageKind::SongCover, 20);
        assert!(state.enqueue_with_scope(
            key.0,
            key.1,
            "https://example.invalid/row.jpg".to_string(),
            ImageRequestScope::Viewport,
        ));
        let request = state.pop_pending().expect("viewport request");
        let (_task, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let observer = handle.clone();
        state.mark_inflight(
            request.kind,
            request.id,
            request.generation,
            request.scope,
            handle,
        );

        assert!(!state.enqueue_with_scope(
            key.0,
            key.1,
            "https://example.invalid/current.jpg".to_string(),
            ImageRequestScope::Global,
        ));
        state.cancel_pending_and_inflight();

        assert!(!observer.is_aborted());
        assert!(state.is_current_inflight(
            key.0,
            key.1,
            request.generation,
            ImageRequestScope::Viewport,
        ));
    }

    #[test]
    fn image_variants_coexist_and_larger_roles_fall_back_to_thumbnail() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::SongCover;
        state.insert_path(kind, 30, PathBuf::from("thumbnail.jpg"));

        assert!(
            state
                .get_with_fallback(kind, 30, crate::image::ImageVariant::Hero)
                .is_some()
        );
        assert!(
            state
                .get_variant(kind, 30, crate::image::ImageVariant::Hero)
                .is_none()
        );

        state.insert_variant_path(
            kind,
            30,
            crate::image::ImageVariant::Hero,
            PathBuf::from("hero.jpg"),
        );

        assert_eq!(state.entries.len(), 2);
        assert!(
            state
                .get_variant(kind, 30, crate::image::ImageVariant::Thumbnail)
                .is_some()
        );
        assert!(
            state
                .get_variant(kind, 30, crate::image::ImageVariant::Hero)
                .is_some()
        );
    }

    #[test]
    fn legacy_thumbnail_does_not_satisfy_exact_remote_identity() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::PlaylistCover;
        state.insert_path(kind, 31, PathBuf::from("legacy.jpg"));

        assert!(state.enqueue_variant_with_scope(
            kind,
            31,
            crate::image::ImageVariant::Thumbnail,
            "https://example.invalid/current.jpg".to_string(),
            ImageRequestScope::Page,
        ));
        assert!(state.is_variant_queued(kind, 31, crate::image::ImageVariant::Thumbnail));
    }

    #[test]
    fn completion_identity_and_cancellation_are_variant_scoped() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::SongCover;
        assert!(state.enqueue_variant_with_scope(
            kind,
            32,
            crate::image::ImageVariant::Thumbnail,
            "https://example.invalid/thumb.jpg".to_string(),
            ImageRequestScope::Viewport,
        ));
        assert!(state.enqueue_variant_with_scope(
            kind,
            32,
            crate::image::ImageVariant::Hero,
            "https://example.invalid/hero.jpg".to_string(),
            ImageRequestScope::Global,
        ));

        let thumbnail = state.pop_pending().expect("thumbnail request");
        let hero = state.pop_pending().expect("hero request");
        let (_thumbnail_task, thumbnail_handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let (_hero_task, hero_handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let thumbnail_observer = thumbnail_handle.clone();
        let hero_observer = hero_handle.clone();
        state.mark_request_inflight(&thumbnail, thumbnail_handle);
        state.mark_request_inflight(&hero, hero_handle);

        assert!(state.is_current_variant_inflight(
            kind,
            32,
            crate::image::ImageVariant::Hero,
            hero.generation,
            ImageRequestScope::Global,
        ));
        assert!(!state.is_current_variant_inflight(
            kind,
            32,
            crate::image::ImageVariant::Thumbnail,
            0,
            ImageRequestScope::Global,
        ));

        state.reconcile_viewport_requests(&HashSet::new());

        assert!(thumbnail_observer.is_aborted());
        assert!(!hero_observer.is_aborted());
    }

    #[test]
    fn current_song_source_probe_is_bounded_per_resource() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::SongCover;

        assert!(state.begin_source_resolution(kind, 33));
        state.finish_source_resolution(kind, 33);
        assert!(!state.begin_source_resolution(kind, 33));

        state.invalidate(kind, 33);
        assert!(state.begin_source_resolution(kind, 33));
    }

    #[test]
    fn changed_source_replaces_pending_and_inflight_variant_work() {
        let mut state = ImageState::default();
        let kind = crate::image::ImageKind::PlaylistCover;
        let variant = crate::image::ImageVariant::Detail;

        assert!(state.enqueue_variant_with_scope(
            kind,
            34,
            variant,
            "https://example.invalid/old.jpg".to_string(),
            ImageRequestScope::Page,
        ));
        assert!(state.enqueue_variant_with_scope(
            kind,
            34,
            variant,
            "https://example.invalid/new.jpg".to_string(),
            ImageRequestScope::Page,
        ));
        let replacement = state.pop_pending().expect("replacement request");
        assert_eq!(replacement.url, "https://example.invalid/new.jpg");

        let (_, handle) = iced::Task::<()>::none().abortable();
        state.mark_request_inflight(&replacement, handle);
        assert!(state.enqueue_variant_with_scope(
            kind,
            34,
            variant,
            "https://example.invalid/latest.jpg".to_string(),
            ImageRequestScope::Page,
        ));
        assert!(!state.is_variant_inflight(kind, 34, variant));
        let latest = state.pop_pending().expect("latest request");
        assert_eq!(latest.url, "https://example.invalid/latest.jpg");
        assert_eq!(latest.scope, ImageRequestScope::Page);
        assert_ne!(latest.generation, replacement.generation);
        let (_, handle) = iced::Task::<()>::none().abortable();
        state.mark_request_inflight(&latest, handle);
        // A queued failure from the aborted source must not clear its replacement.
        assert!(!state.is_current_variant_inflight(
            kind,
            34,
            variant,
            replacement.generation,
            replacement.scope
        ));
    }
}

#[cfg(test)]
fn image_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
}

/// Main application state
pub struct App {
    /// Core infrastructure (Settings, DB, Audio, System integrations)
    pub core: CoreState,
    /// Library/business data (Songs, Playlists, Import state)
    pub library: LibraryState,
    /// Playback session state (Queue, current track, preload, resume snapshot)
    pub playback: PlaybackSessionState,
    /// UI state (Navigation, Page states, Animations)
    pub ui: UiState,
}

/// Core Infrastructure & Services
pub struct CoreState {
    pub db: Option<Arc<Database>>,
    pub db_error: Option<String>,
    /// Download manager for offline downloads
    pub download_manager: crate::download::DownloadManager,
    /// Owns the audio thread lifetime.
    audio_thread: Option<crate::audio::AudioThreadHandle>,
    /// Audio handle for non-blocking audio control
    audio: Option<crate::audio::AudioHandle>,
    /// Audio processing chain (preamp, EQ, analyzer) - shared with AudioPlayer
    audio_chain: AudioProcessingChain,
    /// Latest live output-device projection. Rendering must not enumerate
    /// native devices on every frame.
    pub audio_output_devices: Vec<AudioDevice>,
    /// Manual selector awaiting a matching successful output recovery.
    pub pending_audio_output_device: Option<Option<String>>,
    pub volume_before_mute: Option<f32>,
    pub settings: crate::features::Settings,
    pub locale: Locale,
    pub is_logged_in: bool,

    // NCM API Client
    pub ncm_client: Option<NcmClient>,
    pub user_info: Option<UserInfo>,

    // System Integrations
    pub cover_cache: Option<Arc<CoverCache>>,
    pub mpris_handle: Option<MediaHandle>,
    pub mpris_rx:
        Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<MediaCommand>>>>,
    pub tray_availability: crate::platform::tray::TrayAvailability,
    pub tray_initialization_requested: bool,
    pub discord_presence: DiscordPresence,
    pub global_hotkeys: Option<crate::platform::global_hotkeys::GlobalHotkeyService>,
    pub window_restore_mode: iced::window::Mode,
    pub window_visibility: WindowVisibilityState,
    pub window_focused: bool,
    pub window_maximized: bool,
    pub window_operation_pending: bool,
    pub window_width: f32,
    pub window_height: f32,
    /// Current mouse Y position for drag area detection
    pub mouse_position: iced::Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowVisibilityState {
    Visible,
    Hiding,
    Hidden,
    Showing,
}

impl CoreState {
    pub fn is_window_visible(&self) -> bool {
        self.window_visibility == WindowVisibilityState::Visible
    }

    pub fn is_window_hidden(&self) -> bool {
        !self.is_window_visible()
    }

    /// Initialize core services with loaded settings
    pub fn new(
        settings: crate::features::Settings,
        locale: Locale,
        audio_thread: Option<crate::audio::AudioThreadHandle>,
        audio: Option<crate::audio::AudioHandle>,
        audio_chain: AudioProcessingChain,
    ) -> Self {
        let discord_enabled = settings.system.discord_enabled;
        Self {
            db: None,
            db_error: None,
            download_manager: Default::default(),
            audio_thread,
            audio,
            audio_chain,
            audio_output_devices: Vec::new(),
            pending_audio_output_device: None,
            volume_before_mute: None,
            settings,
            locale,
            is_logged_in: false,
            ncm_client: None,
            user_info: None,
            cover_cache: None,
            mpris_handle: None,
            mpris_rx: None,
            tray_availability: crate::platform::tray::TrayAvailability::Starting,
            tray_initialization_requested: false,
            discord_presence: DiscordPresence::new(discord_enabled),
            global_hotkeys: None,
            window_restore_mode: iced::window::Mode::Windowed,
            window_visibility: WindowVisibilityState::Visible,
            window_focused: true,
            window_maximized: false,
            window_operation_pending: false,
            window_width: 1280.0,
            window_height: 720.0,
            mouse_position: iced::Point::ORIGIN,
        }
    }
}

impl Drop for CoreState {
    fn drop(&mut self) {
        crate::platform::tray::shutdown();
        self.audio = None;
        let _ = self.audio_thread.take();
    }
}

#[derive(Clone)]
pub struct PlaybackRuntimeState {
    pub info: PlaybackInfo,
    pub display_position: Duration,
    pub cache_progress: Option<f32>,
    pub has_loaded_audio: bool,
    pub analysis: AudioAnalysisData,
}

fn status_with_pause_intent(
    backend_status: PlaybackStatus,
    pause_requested: bool,
) -> PlaybackStatus {
    if pause_requested
        && matches!(
            backend_status,
            PlaybackStatus::Playing | PlaybackStatus::Buffering { .. }
        )
    {
        PlaybackStatus::Paused
    } else {
        backend_status
    }
}

impl Default for PlaybackRuntimeState {
    fn default() -> Self {
        Self {
            info: PlaybackInfo::default(),
            display_position: Duration::ZERO,
            cache_progress: None,
            has_loaded_audio: false,
            analysis: AudioAnalysisData::new(),
        }
    }
}

impl PlaybackRuntimeState {
    pub fn is_playing(&self) -> bool {
        matches!(
            self.info.status,
            PlaybackStatus::Playing | PlaybackStatus::Buffering { .. }
        )
    }

    pub fn is_buffering(&self) -> bool {
        matches!(self.info.status, PlaybackStatus::Buffering { .. })
    }

    pub fn can_seek(&self) -> bool {
        self.has_loaded_audio || !self.info.duration.is_zero()
    }
}

#[cfg(test)]
mod playback_intent_tests {
    use super::*;

    #[test]
    fn pause_intent_overrides_playing_and_buffering_until_backend_confirms() {
        assert_eq!(
            status_with_pause_intent(PlaybackStatus::Playing, true),
            PlaybackStatus::Paused
        );
        assert_eq!(
            status_with_pause_intent(
                PlaybackStatus::Buffering {
                    position: Duration::from_secs(3),
                },
                true,
            ),
            PlaybackStatus::Paused
        );
    }

    #[test]
    fn pause_intent_does_not_relabel_backend_terminal_states() {
        assert_eq!(
            status_with_pause_intent(PlaybackStatus::Paused, true),
            PlaybackStatus::Paused
        );
        assert_eq!(
            status_with_pause_intent(PlaybackStatus::Stopped, true),
            PlaybackStatus::Stopped
        );
        assert_eq!(
            status_with_pause_intent(PlaybackStatus::Playing, false),
            PlaybackStatus::Playing
        );
    }

    #[test]
    fn pause_intent_can_be_reversed_before_the_backend_pauses() {
        let backend_status = PlaybackStatus::Playing;
        assert_eq!(
            status_with_pause_intent(backend_status.clone(), true),
            PlaybackStatus::Paused
        );
        assert_eq!(
            status_with_pause_intent(backend_status, false),
            PlaybackStatus::Playing
        );
    }
}

impl App {
    pub(crate) fn begin_audio_resolution_context(
        &self,
    ) -> crate::audio::PlaybackResult<crate::audio::identity::PlaybackContext> {
        self.require_audio_handle()?.begin_playback_resolution()
    }

    pub(crate) fn current_audio_context(&self) -> Option<crate::audio::identity::PlaybackContext> {
        self.audio_handle()
            .and_then(|audio| audio.current_context())
    }

    pub(crate) fn accepts_audio_preload_identity(
        &self,
        identity: &crate::audio::identity::PreloadIdentity,
    ) -> bool {
        self.core.audio.as_ref().is_some_and(|audio| {
            audio.current_context().is_some_and(|context| {
                context.generation == identity.generation
                    && context.cancellation == identity.cancellation
                    && !identity.cancellation.is_cancelled()
                    && audio.accepts_context(&context)
                    && self
                        .playback
                        .audio_preload_manager
                        .accepts_identity(identity)
            })
        })
    }
    pub(crate) fn accepts_audio_context(
        &self,
        context: &crate::audio::identity::PlaybackContext,
    ) -> bool {
        self.core
            .audio
            .as_ref()
            .is_some_and(|audio| audio.accepts_context(context))
    }

    pub(crate) fn accepts_audio_seek(
        &self,
        context: &crate::audio::identity::PlaybackContext,
        nonce: crate::audio::identity::SeekNonce,
    ) -> bool {
        self.core
            .audio
            .as_ref()
            .is_some_and(|audio| audio.accepts_seek(context, nonce))
    }

    fn audio_handle(&self) -> Option<&crate::audio::AudioHandle> {
        self.core.audio.as_ref()
    }

    fn require_audio_handle(&self) -> crate::audio::PlaybackResult<&crate::audio::AudioHandle> {
        self.audio_handle().ok_or_else(|| {
            crate::audio::PlaybackError::DeviceUnavailable(
                "the audio backend is unavailable".to_string(),
            )
        })
    }

    pub(crate) fn refresh_playback_runtime(&mut self) {
        let analysis = self.core.audio_chain.analysis();
        let pause_requested = self.playback.pause_requested;

        if let Some(audio) = self.audio_handle() {
            let mut info = audio.get_info();
            info.status = status_with_pause_intent(info.status, pause_requested);
            self.playback.runtime = PlaybackRuntimeState {
                info,
                display_position: audio.display_position(),
                cache_progress: audio.cache_progress(),
                has_loaded_audio: !audio.is_empty(),
                analysis,
            };
            return;
        }

        let mut runtime = PlaybackRuntimeState {
            analysis,
            ..PlaybackRuntimeState::default()
        };
        if let Some(saved_state) = &self.playback.saved_state {
            runtime.info.volume = saved_state.volume as f32;
        }
        self.playback.runtime = runtime;
    }

    pub(crate) fn playback_runtime(&self) -> &PlaybackRuntimeState {
        &self.playback.runtime
    }

    pub(crate) fn playback_info(&self) -> &PlaybackInfo {
        &self.playback.runtime.info
    }

    pub(crate) fn playback_status(&self) -> PlaybackStatus {
        self.playback.runtime.info.status.clone()
    }

    pub(crate) fn playback_is_playing(&self) -> bool {
        self.playback.runtime.is_playing()
    }

    pub(crate) fn playback_is_buffering(&self) -> bool {
        self.playback.runtime.is_buffering()
    }

    pub(crate) fn playback_output_available(&self) -> bool {
        self.core.audio.is_some()
    }

    pub(crate) fn playback_can_seek(&self) -> bool {
        self.playback.runtime.can_seek()
    }

    pub(crate) fn playback_cache_progress(&self) -> Option<f32> {
        self.playback.runtime.cache_progress
    }

    pub(crate) fn playback_analysis_data(&self) -> AudioAnalysisData {
        self.playback.runtime.analysis.clone()
    }

    pub(crate) fn play_audio_file(
        &self,
        path: PathBuf,
        fade_in: bool,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        let audio = self.require_audio_handle()?;
        audio.play_with_fade(path, fade_in, track_gain)
    }

    pub(crate) fn play_audio_file_in_context(
        &self,
        context: &crate::audio::identity::PlaybackContext,
        path: PathBuf,
        fade_in: bool,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        self.require_audio_handle()?
            .play_with_fade_in_context(context, path, fade_in, track_gain)
    }

    pub(crate) fn play_audio_file_at_position(
        &self,
        path: PathBuf,
        position: Duration,
        fade_in: bool,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        let audio = self.require_audio_handle()?;
        audio.play_from_position_with_fade(path, position, fade_in, track_gain)
    }

    pub(crate) fn play_audio_file_at_position_in_context(
        &self,
        context: &crate::audio::identity::PlaybackContext,
        path: PathBuf,
        position: Duration,
        fade_in: bool,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        self.require_audio_handle()?
            .play_from_position_with_fade_in_context(context, path, position, fade_in, track_gain)
    }

    pub(crate) fn play_streaming_audio(
        &self,
        buffer: crate::audio::SharedBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        fade_in: bool,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        let audio = self.require_audio_handle()?;
        let streaming_buffer = crate::audio::StreamingBuffer::new(buffer);
        audio.play_streaming(streaming_buffer, duration, cache_path, fade_in, track_gain)
    }

    pub(crate) fn play_streaming_audio_in_context(
        &self,
        context: &crate::audio::identity::PlaybackContext,
        buffer: crate::audio::SharedBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        fade_in: bool,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        let streaming_buffer = crate::audio::StreamingBuffer::new(buffer);
        self.require_audio_handle()?.play_streaming_in_context(
            context,
            streaming_buffer,
            duration,
            cache_path,
            fade_in,
            track_gain,
        )
    }

    pub(crate) fn play_preloaded_audio(
        &self,
        identity: crate::audio::identity::PreloadIdentity,
        fade_in: bool,
        transition: Option<crate::audio::automix::TransitionDirective>,
    ) -> crate::audio::PlaybackResult<u64> {
        let audio = self.require_audio_handle()?;
        audio.play_preloaded(identity, fade_in, transition)
    }

    pub(crate) fn schedule_preloaded_audio_transition(
        &self,
        identity: crate::audio::identity::PreloadIdentity,
        trigger_at: Duration,
        fade_in: bool,
        transition: crate::audio::automix::TransitionDirective,
    ) -> crate::audio::PlaybackResult<u64> {
        self.require_audio_handle()?
            .schedule_preloaded_transition(identity, trigger_at, fade_in, transition)
    }

    pub(crate) fn cancel_scheduled_audio_transition(&self) {
        if let Some(audio) = self.audio_handle()
            && let Err(error) = audio.cancel_scheduled_transition()
        {
            tracing::warn!("Failed to cancel scheduled audio transition: {error}");
        }
    }

    pub(crate) fn clear_scheduled_transition_state(&mut self) {
        if self.playback.scheduled_transition_request_id.is_some() {
            self.cancel_scheduled_audio_transition();
        }
        self.playback.scheduled_transition_buffer.take();
        if let Some(request_id) = self.playback.scheduled_transition_request_id.take()
            && self
                .playback
                .pending_playback_request
                .as_ref()
                .is_some_and(|pending| pending.request_id == request_id)
        {
            self.playback.pending_playback_request = None;
        }
        self.playback.pending_transition = None;
        self.playback.pending_transition_trigger = None;
        self.playback.crossfade_triggered = false;
    }

    pub(crate) fn load_audio_file_paused(
        &self,
        path: PathBuf,
        position: Duration,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        let audio = self.require_audio_handle()?;
        audio.load_paused(path, position, track_gain)
    }

    pub(crate) fn load_audio_file_paused_in_context(
        &self,
        context: &crate::audio::identity::PlaybackContext,
        path: PathBuf,
        position: Duration,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        self.require_audio_handle()?
            .load_paused_in_context(context, path, position, track_gain)
    }

    pub(crate) fn load_streaming_audio_paused_in_context(
        &self,
        context: &crate::audio::identity::PlaybackContext,
        buffer: crate::audio::SharedBuffer,
        duration: Duration,
        cache_path: Option<PathBuf>,
        position: Duration,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<u64> {
        let streaming_buffer = crate::audio::StreamingBuffer::new(buffer);
        self.require_audio_handle()?
            .load_streaming_paused_in_context(
                context,
                streaming_buffer,
                duration,
                cache_path,
                position,
                track_gain,
            )
    }

    pub(crate) fn reserve_preload_identity(
        &self,
    ) -> crate::audio::PlaybackResult<crate::audio::identity::PreloadIdentity> {
        self.require_audio_handle()?
            .reserve_preload_identity()
            .ok_or_else(|| {
                crate::audio::PlaybackError::SourceUnavailable(
                    "preload requires active playback generation".to_string(),
                )
            })
    }

    pub(crate) fn reserve_preload_handoff(
        &self,
        parent: &crate::audio::identity::PreloadIdentity,
    ) -> crate::audio::PlaybackResult<crate::audio::identity::PreloadIdentity> {
        self.require_audio_handle()?
            .reserve_preload_handoff(parent)
            .ok_or_else(|| {
                crate::audio::PlaybackError::Cancelled(
                    "preload handoff identity is stale or cancelled".to_string(),
                )
            })
    }

    pub(crate) fn create_preload_sink_for_file(
        &self,
        identity: crate::audio::identity::PreloadIdentity,
        path: PathBuf,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<()> {
        let audio = self.require_audio_handle()?;
        audio.create_preload_sink(identity, path, track_gain)
    }

    pub(crate) fn create_preload_sink_for_stream(
        &self,
        identity: crate::audio::identity::PreloadIdentity,
        buffer: crate::audio::SharedBuffer,
        duration: Duration,
        track_gain: f32,
    ) -> crate::audio::PlaybackResult<()> {
        let audio = self.require_audio_handle()?;
        let streaming_buffer = crate::audio::StreamingBuffer::new(buffer);
        audio.create_preload_sink_streaming(identity, streaming_buffer, duration, track_gain)
    }

    pub(crate) fn release_preload_request(
        &self,
        identity: crate::audio::identity::PreloadIdentity,
    ) {
        if let Some(audio) = self.audio_handle()
            && let Err(error) = audio.release_preload(identity)
        {
            tracing::warn!("Failed to release audio preload: {error}");
        }
    }

    pub(crate) fn release_preload_requests<I>(&self, request_ids: I)
    where
        I: IntoIterator<Item = crate::audio::identity::PreloadIdentity>,
    {
        if let Some(audio) = self.audio_handle() {
            for identity in request_ids {
                if let Err(error) = audio.release_preload(identity) {
                    tracing::warn!("Failed to release audio preload: {error}");
                }
            }
        }
    }

    pub(crate) fn pause_audio_output_with_fade(
        &self,
        fade_out: bool,
    ) -> crate::audio::PlaybackResult<()> {
        self.require_audio_handle()?.pause_with_fade(fade_out)
    }

    pub(crate) fn resume_audio_output_with_fade(
        &self,
        fade_in: bool,
    ) -> crate::audio::PlaybackResult<()> {
        self.require_audio_handle()?.resume_with_fade(fade_in)
    }

    pub(crate) fn stop_audio_backend(&self) {
        if let Some(audio) = self.audio_handle()
            && let Err(error) = audio.stop()
        {
            tracing::warn!("Failed to enqueue stop: {error}");
        }
    }

    pub(crate) fn seek_audio_output(&mut self, position: Duration) {
        if let Some(audio) = self.audio_handle()
            && audio.seek(position).is_ok()
        {
            self.refresh_playback_runtime();
        }
    }

    pub(crate) fn tick_audio_output(&mut self) {
        if let Some(audio) = self.audio_handle() {
            audio.tick();
        }
        self.refresh_playback_runtime();
    }

    pub(crate) fn apply_output_volume(&mut self, volume: f32) {
        if let Some(audio) = self.audio_handle() {
            audio.set_volume(volume);
        }
        self.playback.runtime.info.volume = volume;
    }

    pub(crate) fn persist_output_volume(&self, volume: f32) {
        if let Some(db) = &self.core.db {
            let db = db.clone();
            tokio::spawn(async move {
                let _ = db.update_volume(volume as f64).await;
            });
        }
    }

    pub(crate) fn persist_personal_fm_mode(&self, enabled: bool) {
        if let Some(db) = &self.core.db {
            let db = db.clone();
            tokio::spawn(async move {
                if let Err(err) = db.update_personal_fm_mode(enabled).await {
                    tracing::warn!("Failed to persist Personal FM mode: {}", err);
                }
            });
        }
    }

    pub(crate) fn persist_queue_snapshot(&self) {
        if let Some(db) = &self.core.db {
            let db = db.clone();
            let queue_snapshot = self.playback.queue.clone();
            tokio::spawn(async move {
                if let Err(err) = db.save_queue_with_songs(&queue_snapshot, None).await {
                    tracing::warn!("Failed to persist queue snapshot: {}", err);
                }
            });
        }
    }

    pub(crate) fn set_output_volume(&mut self, volume: f32, persist: bool) {
        self.apply_output_volume(volume);
        if persist {
            self.persist_output_volume(volume);
        }
    }

    pub(crate) fn switch_audio_output_device(
        &mut self,
        device_name: Option<String>,
    ) -> crate::audio::PlaybackResult<()> {
        self.clear_scheduled_transition_state();
        self.require_audio_handle()?
            .switch_device(device_name.clone())?;
        self.core.pending_audio_output_device = Some(device_name);
        Ok(())
    }

    pub(crate) fn audio_output_devices(&self) -> Vec<(String, String)> {
        audio_output_device_options(&self.core.audio_output_devices)
    }

    /// List all installed font families available for lyrics rendering.
    pub(crate) fn lyrics_font_families(&self) -> Vec<String> {
        self.ui.lyrics.font_families.clone()
    }

    pub(crate) fn set_audio_analysis_enabled(&self, enabled: bool) {
        self.core.audio_chain.set_analysis_enabled(enabled);
    }

    pub(crate) fn set_audio_analysis_decay(&self, decay: f32) {
        self.core.audio_chain.set_analysis_decay(decay);
    }

    pub(crate) fn set_audio_equalizer_enabled(&self, enabled: bool) {
        self.core.audio_chain.set_equalizer_enabled(enabled);
    }

    pub(crate) fn set_audio_equalizer_gains(&self, gains: [f32; 10]) {
        self.core.audio_chain.set_equalizer_gains(gains);
    }

    pub(crate) fn set_audio_preamp(&self, preamp_db: f32) {
        self.core.audio_chain.set_preamp(preamp_db);
    }
}

/// User information from NCM
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub user_id: u64,
    pub nickname: String,
    pub vip_type: i32,
    pub vip: crate::api::VipInfo,
    pub like_songs: HashSet<u64>,
}

impl UserInfo {
    pub fn new(user_id: u64, nickname: String) -> Self {
        Self {
            user_id,
            nickname,
            vip_type: 0,
            vip: crate::api::VipInfo::default(),
            like_songs: HashSet::new(),
        }
    }
}

/// Business Logic Data
#[derive(Default)]
pub struct LibraryState {
    pub audio_index: crate::utils::audio_index::AudioIndexState,
    pub db_songs: Vec<DbSong>,
    pub playlists: Vec<DbPlaylist>,
    pub recently_played: Vec<DbSong>,

    // Import State
    pub scan_state: Option<Arc<ScanState>>,
    pub scan_handle: Option<ScanHandle>,
    pub scan_progress: Option<ScanProgress>,
    pub folder_watcher: Option<FolderWatcher>,
    pub watched_folders: Vec<DbWatchedFolder>,
}

/// Playback-owned session state.
///
/// This is kept separate from `LibraryState` so playback coordination has a
/// single home for queue ownership, preload state, and resume bookkeeping.
#[derive(Default)]
pub struct PlaybackSessionState {
    /// Current playing song reflected in UI/system integrations.
    pub current_song: Option<DbSong>,
    /// Current playing song's artist id when available from NCM metadata.
    pub current_artist_id: Option<u64>,
    /// Complete structured artist list for per-artist player-bar navigation.
    pub current_artists: Vec<crate::api::ArtistSummary>,
    /// Structured NCM artists retained for songs owned by the playback queue.
    pub queue_artists_by_song_id: StateMap<u64, Vec<ArtistSummary>>,
    /// Requested and actual NCM quality for the current playback generation.
    pub current_quality: Option<crate::app::update::song_resolver::ResolvedAudioQuality>,
    /// Last saved playback snapshot loaded from the database.
    pub saved_state: Option<DbPlaybackState>,
    /// Active playback queue.
    pub queue: Vec<DbSong>,
    /// Current position within the active queue.
    pub current_index: Option<usize>,
    /// Whether personal FM playback mode is active.
    pub personal_fm_mode: bool,
    /// Source id used when reporting NCM scrobble events for the active queue.
    pub ncm_scrobble_source_id: Option<u64>,
    /// Queue navigation cache used as the single source of truth for shuffle order.
    pub shuffle_cache: crate::app::update::queue_navigator::ShuffleCache,
    /// Audio preload state machine for adjacent tracks.
    pub audio_preload_manager: crate::app::update::audio_preload_manager::AudioPreloadManager,
    /// Background lyrics preload state.
    pub lyrics_preload_manager: crate::app::update::lyrics_preload_manager::LyricsPreloadManager,
    /// Per-song lyrics render cache (engine lines, shaped lines).
    pub lyrics_render_manager: crate::app::update::lyrics_render_manager::LyricsRenderManager,
    /// Central preload coordinator - owns the preload window lifecycle.
    pub preload_coordinator: crate::app::update::preload_coordinator::PreloadCoordinator,
    /// Track index currently being resolved before playback starts.
    pub pending_resolution_index: Option<usize>,
    /// Playback request waiting for audio thread confirmation.
    pub pending_playback_request: Option<PendingPlaybackRequest>,
    /// Active streaming buffer shared with the audio thread.
    pub active_streaming_buffer: Option<crate::audio::SharedBuffer>,
    /// Cached runtime snapshot consumed by UI/system integrations.
    pub runtime: PlaybackRuntimeState,
    /// Logical pause intent exposed immediately while the backend finishes its
    /// sample-driven Fade and physical Sink pause.
    pub pause_requested: bool,
    /// Consecutive playback failure counter.
    pub consecutive_failures: u8,
    /// Prevents repeated natural Crossfade triggers while the next generation
    /// is being promoted.
    pub crossfade_triggered: bool,
    /// One-shot transition intent consumed only by the next preloaded handoff.
    pub pending_transition: Option<crate::audio::automix::TransitionDirective>,
    /// Audio-clock deadline paired with `pending_transition` for natural handoff.
    pub pending_transition_trigger: Option<Duration>,
    /// Request whose UI commit is deferred until the scheduled handoff starts.
    pub scheduled_transition_request_id: Option<u64>,
    /// Incoming streaming buffer retained without cancelling the outgoing source.
    pub scheduled_transition_buffer: Option<crate::audio::SharedBuffer>,
    /// Throttles finalized-cache discovery for streaming Automix analysis.
    pub automix_analysis_poll_at: Option<std::time::Instant>,
    /// Startup restore coordination state.
    pub startup_restore: StartupRestoreState,
}

#[derive(Debug, Clone, Default)]
pub struct StartupRestoreState {
    pub playback_state_loaded: bool,
    pub queue_loaded: bool,
    pub songs_loaded: bool,
    pub in_progress: bool,
    pub completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPlaybackKind {
    StartPlaying,
    LoadPaused,
    RestartCurrent,
}

#[derive(Debug, Clone)]
pub struct PendingPlaybackRequest {
    pub request_id: u64,
    pub queue_index: Option<usize>,
    pub song: DbSong,
    pub kind: PendingPlaybackKind,
}

/// Unified route model for page rendering and navigation history
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Discover(DiscoverViewMode),
    Radio,
    Downloads,
    Settings(SettingsSection),
    AudioEngine,
    Playlist(i64),
    NcmPlaylist(u64),
    Podcast(u64),
    User(u64),
    Artist(u64),
    Album(u64),
    RecentlyPlayed,
    Search {
        keyword: String,
        tab: SearchTab,
        page: u32,
    },
}

impl Route {
    pub fn nav_item(&self) -> Option<NavItem> {
        match self {
            Self::Discover(_) => Some(NavItem::Home),
            Self::Radio => Some(NavItem::Radio),
            Self::Downloads => Some(NavItem::Downloads),
            Self::Settings(_) => Some(NavItem::Settings),
            Self::AudioEngine => Some(NavItem::AudioEngine),
            Self::Playlist(_)
            | Self::NcmPlaylist(_)
            | Self::Podcast(_)
            | Self::User(_)
            | Self::Artist(_)
            | Self::Album(_)
            | Self::RecentlyPlayed
            | Self::Search { .. } => None,
        }
    }

    /// Whether the page supplies its own cover-derived background gradient.
    ///
    /// These routes render underneath the top-bar overlay, so adding another
    /// themed surface there would hide the gradient instead of letting it
    /// continue through the controls area.
    pub fn has_gradient_background(&self) -> bool {
        matches!(
            self,
            Self::Playlist(_)
                | Self::NcmPlaylist(_)
                | Self::Podcast(_)
                | Self::User(_)
                | Self::Artist(_)
                | Self::Album(_)
                | Self::RecentlyPlayed
        )
    }
}

#[cfg(test)]
mod route_tests {
    use super::{DiscoverViewMode, Route, SearchTab, SettingsSection};

    #[test]
    fn detail_routes_with_cover_gradients_are_identified() {
        let routes = [
            Route::Playlist(1),
            Route::NcmPlaylist(2),
            Route::User(3),
            Route::Artist(4),
            Route::Album(5),
            Route::RecentlyPlayed,
        ];

        assert!(routes.iter().all(Route::has_gradient_background));
    }

    #[test]
    fn ordinary_routes_keep_the_top_bar_background() {
        let routes = [
            Route::Discover(DiscoverViewMode::Overview),
            Route::Radio,
            Route::Downloads,
            Route::Settings(SettingsSection::Display),
            Route::AudioEngine,
            Route::Search {
                keyword: String::new(),
                tab: SearchTab::Songs,
                page: 1,
            },
        ];

        assert!(routes.iter().all(|route| !route.has_gradient_background()));
    }
}

/// Navigation history entry
#[derive(Debug, Clone, PartialEq)]
pub enum NavigationEntry {
    Route(Route),
}

impl NavigationEntry {
    fn should_record(&self) -> bool {
        // Entering Personal FM starts playback as a route side effect. Keeping
        // it out of history prevents back/forward navigation from starting a
        // new FM batch and changing the current song.
        !matches!(self, Self::Route(Route::Radio))
    }
}

/// Navigation history for back/forward functionality
#[derive(Debug, Default)]
pub struct NavigationHistory {
    /// History stack
    pub entries: Vec<NavigationEntry>,
    /// Current position in history (index)
    pub current_index: Option<usize>,
    /// The visible route is intentionally outside the recorded history.
    ///
    /// Personal FM is a playback action with route side effects. Tracking the
    /// transient state separately keeps the history cursor aligned with the
    /// last recorded route without making FM replayable through back/forward.
    current_is_transient: bool,
}

impl NavigationHistory {
    /// Push a new entry to history, clearing forward history
    pub fn push(&mut self, entry: NavigationEntry) {
        if !entry.should_record() {
            self.current_is_transient = true;
            return;
        }

        self.current_is_transient = false;

        // Don't push if it's the same as current
        if let Some(idx) = self.current_index {
            if idx < self.entries.len() && self.entries[idx] == entry {
                return;
            }
            // Clear forward history
            self.entries.truncate(idx + 1);
        }
        self.entries.push(entry);
        self.current_index = Some(self.entries.len() - 1);
    }

    /// Replace the current history entry without changing stack length
    pub fn replace_current(&mut self, entry: NavigationEntry) {
        if !entry.should_record() {
            self.current_is_transient = true;
            return;
        }

        self.current_is_transient = false;

        if let Some(idx) = self.current_index
            && idx < self.entries.len()
        {
            self.entries[idx] = entry;
            return;
        }

        self.push(entry);
    }

    /// Go back in history, returns the entry to navigate to
    pub fn go_back(&mut self) -> Option<NavigationEntry> {
        if self.current_is_transient {
            self.current_is_transient = false;
            return self
                .current_index
                .and_then(|idx| self.entries.get(idx).cloned());
        }

        if let Some(idx) = self.current_index
            && idx > 0
        {
            self.current_index = Some(idx - 1);
            return self.entries.get(idx - 1).cloned();
        }
        None
    }

    /// Go forward in history, returns the entry to navigate to
    pub fn go_forward(&mut self) -> Option<NavigationEntry> {
        if let Some(idx) = self.current_index
            && idx + 1 < self.entries.len()
        {
            self.current_is_transient = false;
            self.current_index = Some(idx + 1);
            return self.entries.get(idx + 1).cloned();
        }
        None
    }

    /// Check if can go back
    pub fn can_go_back(&self) -> bool {
        self.current_is_transient && self.current_index.is_some()
            || self.current_index.map(|idx| idx > 0).unwrap_or(false)
    }

    /// Check if can go forward
    pub fn can_go_forward(&self) -> bool {
        self.current_index
            .map(|idx| idx + 1 < self.entries.len())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod navigation_history_tests {
    use super::{DiscoverViewMode, NavigationEntry, NavigationHistory, Route};

    #[test]
    fn personal_fm_is_excluded_from_back_forward_history() {
        let mut history = NavigationHistory::default();
        let home = Route::Discover(DiscoverViewMode::Overview);
        history.push(NavigationEntry::Route(home.clone()));
        history.push(NavigationEntry::Route(Route::Downloads));
        assert_eq!(
            history.go_back(),
            Some(NavigationEntry::Route(home.clone()))
        );

        history.push(NavigationEntry::Route(Route::Radio));

        assert_eq!(
            history.entries,
            vec![
                NavigationEntry::Route(home.clone()),
                NavigationEntry::Route(Route::Downloads),
            ]
        );
        assert_eq!(history.current_index, Some(0));
        assert!(history.can_go_back());
        assert!(history.can_go_forward());

        // Back exits the transient FM route to the route that was current
        // before FM, without stepping past it in recorded history.
        assert_eq!(
            history.go_back(),
            Some(NavigationEntry::Route(home.clone()))
        );
        assert_eq!(history.current_index, Some(0));

        // Forward continues through the pre-existing history. FM is never a
        // target and therefore cannot restart playback as a route side effect.
        assert_eq!(
            history.go_forward(),
            Some(NavigationEntry::Route(Route::Downloads))
        );

        assert_eq!(history.go_back(), Some(NavigationEntry::Route(home)));
        history.replace_current(NavigationEntry::Route(Route::Radio));
        assert_eq!(
            history.go_forward(),
            Some(NavigationEntry::Route(Route::Downloads))
        );
    }
}

/// UI View State
pub struct UiState {
    pub current_route: Route,
    pub search_query: String,
    pub toast: Option<Toast>,
    pub toast_visible: bool,

    /// Overlay stack — LIFO: last = topmost rendered overlay
    pub overlay_stack: Vec<OverlayEntry>,

    /// Navigation history for back/forward
    pub nav_history: NavigationHistory,

    // Sub-modules
    pub playlist_page: PlaylistPageState,
    pub lyrics: LyricsState,
    pub dialogs: DialogState,
    pub home: HomePageState,
    pub discover: DiscoverPageState,
    pub search: SearchPageState,

    // Global UI Layout
    pub active_settings_section: SettingsSection,
    /// Runtime-measured Y positions of each settings section (for scroll→tab mapping)
    pub section_positions: Vec<(SettingsSection, f32)>,
    /// Whether section positions have been measured from the actual widget tree
    pub positions_measured: bool,
    /// Latest absolute offset reported by the settings page scrollable.
    pub settings_scroll_offset: f32,
    /// Shared motion state for native and virtual smooth scrolling.
    pub smooth_scroll: SmoothScrollState,
    pub editing_keybinding: Option<(crate::features::Action, crate::features::ShortcutScope)>,
    /// Native event already observed for the key currently being recorded.
    pub global_hotkey_seen_while_recording: Option<u32>,
    /// A native event expected just after recording finishes. The deadline
    /// prevents a missing event from suppressing a later real key press.
    pub suppressed_recording_hotkey: Option<(u32, Instant)>,
    pub queue_visible: bool,

    // Playback Controls UI
    pub seek_preview_position: Option<f32>,
    pub save_position_counter: u32,
    pub last_mpris_sync: Option<Instant>,

    // Sidebar
    pub importing_playlist: Option<ImportingPlaylist>,
    pub sidebar_animations: HoverAnimations<crate::app::message::SidebarId>,
    /// Draggable sidebar width in 1080P reference pixels.
    pub sidebar_width: f32,
    /// Whether the sidebar resize handle is being dragged
    pub sidebar_dragging: bool,
    /// Whether the compact navigation drawer is currently open.
    pub sidebar_drawer_open: bool,
    /// Frame-synchronized transition for the compact navigation drawer.
    pub sidebar_drawer_animation: SingleHoverAnimation,
    /// Whether the owned cloud playlists section is expanded
    pub my_playlists_expanded: bool,
    /// Whether the collected cloud playlists section is expanded
    pub collected_playlists_expanded: bool,

    // Cache statistics
    pub cache_stats: Option<crate::cache::CacheStats>,
    pub cache_limit_in_flight: bool,

    // Context menu
    pub context_menu: Option<ContextMenuState>,

    // Song edit form state (visibility managed by overlay_stack)
    pub song_edit_dialog: Option<SongEditDialogState>,

    // Download panel tab
    pub download_tab: DownloadTab,

    // Unified image-handle cache (pre-loaded handles, no disk I/O in views)
    pub image_state: ImageState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DownloadTab {
    #[default]
    Active,
    Completed,
    Failed,
}

/// Context menu state
#[derive(Debug, Clone)]
pub struct ContextMenuState {
    pub song_id: i64,
    pub x: f32,
    pub y: f32,
    /// Song source origin (Local / Cached / Online)
    pub source: Source,
    /// Whether this song is liked/favorited by the current user
    pub is_liked: bool,
}

/// Song edit dialog state
#[derive(Debug, Clone)]
pub struct SongEditDialogState {
    pub cover_handle: Option<iced::widget::image::Handle>,
    pub song_id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_number: Option<u32>,
    pub year: Option<u32>,
    pub genre: String,
    pub cover_path: Option<PathBuf>,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            current_route: Route::Discover(DiscoverViewMode::Overview),
            search_query: String::new(),
            toast: None,
            toast_visible: false,
            overlay_stack: Vec::new(),
            nav_history: {
                let mut history = NavigationHistory::default();
                history.push(NavigationEntry::Route(Route::Discover(
                    DiscoverViewMode::Overview,
                )));
                history
            },
            active_settings_section: SettingsSection::Account,
            section_positions: vec![
                (SettingsSection::Account, 0.0),
                (SettingsSection::Playback, 150.0),
                (SettingsSection::Display, 500.0),
                (SettingsSection::System, 850.0),
                (SettingsSection::Network, 1000.0),
                (SettingsSection::Storage, 1150.0),
                (SettingsSection::Shortcuts, 1390.0),
                (SettingsSection::About, 1965.0),
            ],
            positions_measured: false,
            settings_scroll_offset: 0.0,
            smooth_scroll: SmoothScrollState::default(),
            editing_keybinding: None,
            global_hotkey_seen_while_recording: None,
            suppressed_recording_hotkey: None,
            queue_visible: false,
            seek_preview_position: None,
            save_position_counter: 0,
            last_mpris_sync: None,
            importing_playlist: None,
            sidebar_animations: Default::default(),
            sidebar_width: 280.0,
            sidebar_dragging: false,
            sidebar_drawer_open: false,
            sidebar_drawer_animation: SingleHoverAnimation::with_duration(
                std::time::Duration::from_millis(220),
            ),
            my_playlists_expanded: true,
            collected_playlists_expanded: true,
            cache_stats: None,
            cache_limit_in_flight: false,
            context_menu: None,
            song_edit_dialog: None,
            download_tab: Default::default(),
            image_state: ImageState::default(),

            playlist_page: PlaylistPageState {
                current: None,
                viewing_recently_played: false,
                song_animations: Default::default(),
                icon_animations: Default::default(),
                search_expanded: false,
                search_query: String::new(),
                search_animation: Default::default(),
                gradient_animation: SingleHoverAnimation::with_duration(
                    std::time::Duration::from_millis(450),
                ),
                gradient_palette_key: None,
                gradient_source: None,
                retained_gradient: None,
                scroll_state: std::rc::Rc::new(std::cell::RefCell::new(
                    crate::ui::widgets::VirtualListState::default(),
                )),
                load_state: Default::default(),
                description_expanded: false,
                ncm_cache_baseline: None,
                ncm_replace_songs_on_chunk: false,
                ncm_load_generation: 0,
                podcast_request: None,
                online_tracks: Vec::new(),
                online_track_owner: None,
            },

            lyrics: LyricsState {
                is_open: false,
                display_mode: LyricsDisplayMode::default(),
                animation: Default::default(),
                displayed_song_id: None,
                pending_song_id: None,
                lines: Vec::new(),
                current_line_idx: None,
                last_update: None,
                bg_colors: crate::utils::DominantColors::dark_default(),
                // Initialized directly as requested
                bg_shader: LyricsBackgroundProgram::new(),
                textured_bg_shader: TexturedBackgroundProgram::new(),
                // Engine will be created lazily when FontSystem is ready
                // This avoids blocking app startup with FontSystem::new()
                engine: None,
                shader_start_time: None,
                cached_engine_lines: None,
                cached_shaped_lines: None,
                // FontSystem will be created asynchronously
                shared_font_system: None,
                font_families: Vec::new(),
                display_font: iced::Font::DEFAULT,
                // Conservative bootstrap values; the mounted renderer's
                // Sensor supplies the actual viewport before shaping.
                viewport_width: 800.0,
                viewport_height: 600.0,
                viewport_initialized: false,
                pending_viewport_size: None,
                shaped_content_width: 0.0,
                shaped_font_size: 0.0,
                is_loading: false,
                load_error: None,
            },

            dialogs: DialogState { import_open: false },

            home: HomePageState {
                login_popup_open: false,
                qr_code_path: None,
                qr_unikey: None,
                qr_status: None,
                user_playlists: Vec::new(),
                current_ncm_playlist_songs: Vec::new(),
            },

            discover: DiscoverPageState::default(),

            search: SearchPageState {
                scroll_state: std::rc::Rc::new(std::cell::RefCell::new(
                    crate::ui::widgets::VirtualListState::default(),
                )),
                ..Default::default()
            },
        }
    }

    /// Check if any global or submodule animation is currently active
    /// Optimized: O(1) check for hover animations, only checks active/fading states
    pub fn has_active_animations(&self, _now: Instant) -> bool {
        // Hover animations are O(1): they retain only active + fading state.
        // The timestamp is consumed by cleanup_animations when frames advance.
        self.sidebar_animations.is_animating()
            || self.sidebar_drawer_animation.is_animating()
            || self.playlist_page.song_animations.is_animating()
            || self.playlist_page.icon_animations.is_animating()
            || self.playlist_page.search_animation.is_animating()
            || self.playlist_page.gradient_animation.is_animating()
            || self.lyrics.animation.is_animating()
            || self.discover.card_animations.is_animating()
            || self.search.song_animations.is_animating()
            || self.search.card_animations.is_animating()
            || self.smooth_scroll.is_animating()
            // The import card is rendered from live scan progress. Keep the
            // frame subscription alive while it is present so progress and
            // the completion affordance are painted immediately.
            || self.importing_playlist.is_some()
    }

    /// Whether a visible UI layer owns pointer input instead of the main content.
    ///
    /// Toasts intentionally do not count here: they are notifications and remain pointer
    /// transparent. Lyrics are included while opening/closing so the underlying player cannot
    /// receive input during the transition either.
    pub fn has_blocking_pointer_overlay(&self) -> bool {
        self.queue_visible
            || self.sidebar_drawer_open
            || self.sidebar_drawer_animation.is_animating()
            || self.sidebar_drawer_animation.progress() > 0.01
            || self.home.login_popup_open
            || !self.overlay_stack.is_empty()
            || self.context_menu.is_some()
            || self.lyrics.is_open
            || self.lyrics.animation.is_animating()
            || self.lyrics.animation.progress() > 0.01
    }

    /// Clean up completed animations to prevent memory leaks
    /// Call this periodically (e.g., on AnimationTick)
    pub fn cleanup_animations(&mut self, now: Instant) {
        // Tick all animations to advance time
        self.sidebar_animations.tick(now);
        self.sidebar_drawer_animation.tick(now);
        self.playlist_page.song_animations.tick(now);
        self.playlist_page.icon_animations.tick(now);
        self.playlist_page.search_animation.tick(now);
        self.playlist_page.gradient_animation.tick(now);
        self.lyrics.animation.tick(now);
        self.discover.card_animations.tick(now);
        self.search.song_animations.tick(now);
        self.search.card_animations.tick(now);

        // Clean up completed fade-out animations
        self.sidebar_animations.cleanup_completed();
        self.playlist_page.song_animations.cleanup_completed();
        self.playlist_page.icon_animations.cleanup_completed();
        self.discover.card_animations.cleanup_completed();
        self.search.song_animations.cleanup_completed();
        self.search.card_animations.cleanup_completed();
    }

    /// Clear all playlist-related animations when navigating away
    pub fn clear_playlist_animations(&mut self) {
        self.playlist_page.song_animations.clear();
        self.playlist_page.icon_animations.clear();
    }

    /// Start or reverse the compact navigation drawer transition.
    pub fn set_sidebar_drawer_open(&mut self, open: bool, animate: bool) {
        self.sidebar_drawer_open = open;
        if animate {
            if open {
                self.sidebar_drawer_animation.start();
            } else {
                self.sidebar_drawer_animation.stop();
            }
        } else {
            self.sidebar_drawer_animation
                .settle_at(if open { 1.0 } else { 0.0 });
        }
    }

    /// Whether the drawer must remain in the view tree during its opening or
    /// closing transition.
    pub fn sidebar_drawer_visible(&self) -> bool {
        self.sidebar_drawer_open
            || self.sidebar_drawer_animation.is_animating()
            || self.sidebar_drawer_animation.progress() > 0.01
    }

    /// Current drawer transition progress in the inclusive range `[0, 1]`.
    pub fn sidebar_drawer_progress(&self) -> f32 {
        self.sidebar_drawer_animation.progress().clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod smooth_scroll_animation_tests {
    use super::UiState;
    use crate::ui::animation::SmoothScrollTarget;
    use iced::time::Instant;

    #[test]
    fn smooth_scroll_requests_participate_in_animation_frames() {
        let now = Instant::now();
        let mut state = UiState::new();

        state
            .smooth_scroll
            .request_wheel(SmoothScrollTarget::Native("test_scroll"), 60.0, now);

        assert!(state.has_active_animations(now));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistTrackOwner {
    pub page_id: i64,
    pub generation: u64,
}

pub struct PlaylistPageState {
    pub current: Option<pages::PlaylistView>,
    pub viewing_recently_played: bool,
    pub song_animations: HoverAnimations<i64>,
    pub icon_animations: HoverAnimations<crate::app::message::IconId>,
    pub search_expanded: bool,
    pub search_query: String,
    pub search_animation: SingleHoverAnimation,
    /// Fade-in animation for cover-derived detail-page gradients.
    pub gradient_animation: SingleHoverAnimation,
    /// Palette identity already installed into the gradient animation.
    pub gradient_palette_key: Option<(
        crate::ui::pages::playlist::DetailPageKind,
        i64,
        Option<String>,
    )>,
    /// Gradient displayed while the current page palette is pending and used
    /// as the fixed start of the next transition.
    gradient_source: Option<crate::ui::pages::playlist::DetailGradientSnapshot>,
    /// Most recently computed detail-page gradient, retained across routes.
    retained_gradient: Option<crate::ui::pages::playlist::DetailGradientSnapshot>,
    /// Virtual list scroll state for efficient rendering
    pub scroll_state: std::rc::Rc<std::cell::RefCell<crate::ui::widgets::VirtualListState>>,
    /// Loading state for async playlist loading
    pub load_state: crate::app::update::page_loader::PlaylistLoadState,
    /// Whether the playlist description is expanded (vs clamped to 2 lines)
    pub description_expanded: bool,
    /// Cached NCM snapshot used to decide whether a background refresh changed
    /// the playlist contents.
    pub ncm_cache_baseline: Option<crate::api::PlaylistDetail>,
    /// Replace cached songs when the first refreshed batch arrives.
    pub ncm_replace_songs_on_chunk: bool,
    /// Monotonic token for online detail-page requests. Every route entry gets
    /// a new token so a request from an earlier visit cannot update the page
    /// after the user has switched away and back.
    pub ncm_load_generation: u64,
    pub podcast_request: Option<iced::task::Handle>,
    /// Raw NCM tracks that produced the current detail-page rows.
    online_tracks: Vec<Track>,
    /// Exact page/generation allowed to read or mutate `online_tracks`.
    online_track_owner: Option<PlaylistTrackOwner>,
}

impl Default for PlaylistPageState {
    fn default() -> Self {
        Self {
            current: None,
            viewing_recently_played: false,
            song_animations: Default::default(),
            icon_animations: Default::default(),
            search_expanded: false,
            search_query: String::new(),
            search_animation: Default::default(),
            gradient_animation: SingleHoverAnimation::with_duration(
                std::time::Duration::from_millis(450),
            ),
            gradient_palette_key: None,
            gradient_source: None,
            retained_gradient: None,
            scroll_state: std::rc::Rc::new(std::cell::RefCell::new(
                crate::ui::widgets::VirtualListState::default(),
            )),
            load_state: Default::default(),
            description_expanded: false,
            ncm_cache_baseline: None,
            ncm_replace_songs_on_chunk: false,
            ncm_load_generation: 0,
            podcast_request: None,
            online_tracks: Vec::new(),
            online_track_owner: None,
        }
    }
}

impl PlaylistPageState {
    pub fn advance_load_generation(&mut self) -> u64 {
        self.cancel_podcast_request();
        self.ncm_load_generation = self.ncm_load_generation.wrapping_add(1);
        self.clear_online_tracks();
        self.ncm_load_generation
    }

    pub fn cancel_podcast_request(&mut self) {
        if let Some(handle) = self.podcast_request.take() {
            handle.abort();
        }
    }

    pub fn begin_online_tracks(&mut self, page_id: i64, generation: u64) -> bool {
        if generation != self.ncm_load_generation {
            return false;
        }
        self.online_track_owner = Some(PlaylistTrackOwner {
            page_id,
            generation,
        });
        self.online_tracks.clear();
        true
    }

    pub fn clear_online_tracks(&mut self) {
        self.online_tracks.clear();
        self.online_track_owner = None;
    }

    pub fn replace_online_tracks(
        &mut self,
        page_id: i64,
        generation: u64,
        tracks: Vec<Track>,
    ) -> bool {
        if !self.online_track_owner_matches(page_id, generation) {
            return false;
        }
        self.online_tracks = tracks;
        true
    }

    pub fn append_online_tracks(
        &mut self,
        page_id: i64,
        generation: u64,
        tracks: &[Track],
    ) -> bool {
        if !self.online_track_owner_matches(page_id, generation) {
            return false;
        }
        self.online_tracks.extend_from_slice(tracks);
        true
    }

    pub fn online_tracks_for(&self, page_id: i64, generation: u64) -> Option<&[Track]> {
        self.online_track_owner_matches(page_id, generation)
            .then_some(self.online_tracks.as_slice())
    }

    pub fn current_online_tracks(&self) -> Option<&[Track]> {
        let page_id = self.current.as_ref()?.id;
        self.online_tracks_for(page_id, self.ncm_load_generation)
    }

    pub fn current_online_track(&self, ncm_id: u64) -> Option<&Track> {
        self.current_online_tracks()?
            .iter()
            .find(|track| track.id == ncm_id)
    }

    pub fn remove_current_online_track(&mut self, ncm_id: u64) {
        let Some(page_id) = self.current.as_ref().map(|page| page.id) else {
            return;
        };
        if self.online_track_owner_matches(page_id, self.ncm_load_generation) {
            self.online_tracks.retain(|track| track.id != ncm_id);
        }
    }

    fn online_track_owner_matches(&self, page_id: i64, generation: u64) -> bool {
        self.online_track_owner
            == Some(PlaylistTrackOwner {
                page_id,
                generation,
            })
            && generation == self.ncm_load_generation
    }

    /// Synchronize the gradient fade with the current page palette.
    pub fn sync_gradient_animation(&mut self, power_saving: bool) {
        let target = self
            .current
            .as_ref()
            .and_then(crate::ui::pages::PlaylistView::gradient_snapshot);
        let palette_key = self
            .current
            .as_ref()
            .and_then(|page| target.map(|_| (page.kind, page.id, page.cover_path.clone())));

        self.sync_gradient_target(palette_key, target, power_saving);
    }

    fn sync_gradient_target(
        &mut self,
        palette_key: Option<(
            crate::ui::pages::playlist::DetailPageKind,
            i64,
            Option<String>,
        )>,
        target: Option<crate::ui::pages::playlist::DetailGradientSnapshot>,
        power_saving: bool,
    ) {
        if palette_key != self.gradient_palette_key {
            self.gradient_palette_key = palette_key;
            self.gradient_source = self.retained_gradient;
            self.gradient_animation.settle_at(0.0);

            if let Some(target) = target {
                let unchanged = self.gradient_source == Some(target);
                self.retained_gradient = Some(target);

                if power_saving || unchanged {
                    self.gradient_animation.settle_at(1.0);
                } else {
                    self.gradient_animation.start();
                }
            }
        } else if power_saving && self.gradient_animation.is_animating() {
            self.gradient_animation.settle_at(1.0);
        }
    }

    pub fn reset_gradient_animation(&mut self) {
        self.gradient_source = self.retained_gradient;
        self.gradient_palette_key = None;
        self.gradient_animation.settle_at(0.0);
    }

    /// Clear detail-page gradient state so the next gradient starts at the
    /// theme background instead of reusing a previous detail page's colors.
    pub fn reset_gradient_to_background(&mut self) {
        self.gradient_source = None;
        self.retained_gradient = None;
        self.gradient_palette_key = None;
        self.gradient_animation.settle_at(0.0);
    }

    pub fn gradient_source(&self) -> Option<crate::ui::pages::playlist::DetailGradientSnapshot> {
        self.gradient_source
    }
}

#[cfg(test)]
mod detail_gradient_state_tests {
    use super::PlaylistPageState;
    use crate::ui::pages::playlist::{DetailGradientSnapshot, DetailPageKind};
    use iced::Color;

    #[test]
    fn completed_gradient_becomes_the_next_page_transition_source() {
        let first = DetailGradientSnapshot {
            kind: DetailPageKind::Playlist,
            primary: Color::from_rgb(0.2, 0.4, 0.7),
        };
        let second = DetailGradientSnapshot {
            kind: DetailPageKind::Artist,
            primary: Color::from_rgb(0.8, 0.25, 0.15),
        };
        let mut state = PlaylistPageState::default();

        state.sync_gradient_target(
            Some((DetailPageKind::Playlist, 1, Some("first".into()))),
            Some(first),
            true,
        );
        assert_eq!(state.retained_gradient, Some(first));
        assert_eq!(state.gradient_animation.progress(), 1.0);

        state.reset_gradient_animation();
        assert_eq!(state.gradient_source(), Some(first));
        assert_eq!(state.gradient_animation.progress(), 0.0);

        state.sync_gradient_target(
            Some((DetailPageKind::Artist, 2, Some("second".into()))),
            Some(second),
            false,
        );
        assert_eq!(state.gradient_source(), Some(first));
        assert_eq!(state.retained_gradient, Some(second));
        assert!(state.gradient_animation.is_animating());
    }

    #[test]
    fn resetting_to_background_drops_the_retained_gradient() {
        let first = DetailGradientSnapshot {
            kind: DetailPageKind::Playlist,
            primary: Color::from_rgb(0.2, 0.4, 0.7),
        };
        let second = DetailGradientSnapshot {
            kind: DetailPageKind::Artist,
            primary: Color::from_rgb(0.8, 0.25, 0.15),
        };
        let mut state = PlaylistPageState::default();

        state.sync_gradient_target(
            Some((DetailPageKind::Playlist, 1, Some("first".into()))),
            Some(first),
            true,
        );
        state.reset_gradient_to_background();

        assert_eq!(state.gradient_source(), None);
        assert_eq!(state.retained_gradient, None);
        assert_eq!(state.gradient_palette_key, None);
        assert_eq!(state.gradient_animation.progress(), 0.0);

        state.sync_gradient_target(
            Some((DetailPageKind::Artist, 2, Some("second".into()))),
            Some(second),
            false,
        );
        assert_eq!(state.gradient_source(), None);
        assert!(state.gradient_animation.is_animating());
    }
}

#[cfg(test)]
mod playlist_online_track_state_tests {
    use super::PlaylistPageState;
    use crate::api::Track;

    fn track(id: u64) -> Track {
        Track {
            id,
            title: format!("track-{id}"),
            ..Track::default()
        }
    }

    #[test]
    fn leaving_or_reloading_a_podcast_aborts_pending_network_work() {
        let mut state = PlaylistPageState::default();
        let (_, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let observer = handle.clone();
        state.podcast_request = Some(handle);
        state.cancel_podcast_request();
        assert!(observer.is_aborted());
        assert!(state.podcast_request.is_none());

        let (_, handle) = iced::Task::perform(async {}, |_| ()).abortable();
        let observer = handle.clone();
        state.podcast_request = Some(handle);
        state.advance_load_generation();
        assert!(observer.is_aborted());
        assert!(state.podcast_request.is_none());
    }

    #[test]
    fn owned_tracks_replace_append_and_reject_stale_generations() {
        let mut state = PlaylistPageState::default();
        let generation = state.advance_load_generation();
        assert!(state.begin_online_tracks(-10, generation));
        assert!(state.replace_online_tracks(-10, generation, vec![track(1), track(2)]));
        assert!(state.append_online_tracks(-10, generation, &[track(3)]));
        assert_eq!(
            state
                .online_tracks_for(-10, generation)
                .expect("current tracks")
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        assert!(!state.append_online_tracks(-11, generation, &[track(4)]));
        let next_generation = state.advance_load_generation();
        assert!(state.online_tracks_for(-10, generation).is_none());
        assert!(!state.replace_online_tracks(-10, generation, vec![track(5)]));
        assert!(state.begin_online_tracks(-10, next_generation));
        assert!(state.replace_online_tracks(-10, next_generation, vec![track(6)]));
        assert_eq!(
            state.online_tracks_for(-10, next_generation).unwrap()[0].id,
            6
        );
    }
}

/// Presentation selected inside the full-screen player.
///
/// This state is intentionally independent from the displayed song and lyrics
/// loading pipeline so changing tracks does not reset the user's current view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LyricsDisplayMode {
    #[default]
    Artwork,
    Lyrics,
}

#[cfg(test)]
mod lyrics_display_mode_tests {
    use super::LyricsDisplayMode;

    #[test]
    fn full_screen_player_opens_from_artwork_mode() {
        assert_eq!(LyricsDisplayMode::default(), LyricsDisplayMode::Artwork);
    }
}

pub struct LyricsState {
    pub is_open: bool,
    pub display_mode: LyricsDisplayMode,
    pub animation: SingleHoverAnimation,
    /// Song currently displayed in the lyrics page.
    pub displayed_song_id: Option<i64>,
    /// Song currently being loaded for display.
    pub pending_song_id: Option<i64>,
    pub lines: Vec<crate::application::lyrics::LyricLine>,
    pub current_line_idx: Option<usize>,
    pub last_update: Option<Instant>,

    // Visuals & Shaders
    pub bg_colors: crate::utils::DominantColors,
    pub bg_shader: LyricsBackgroundProgram,
    pub textured_bg_shader: TexturedBackgroundProgram,
    /// 歌词引擎 (RefCell 用于 view() 中的内部可变性)
    pub engine: Option<std::cell::RefCell<crate::features::lyrics::engine::LyricsEngine>>,
    pub shader_start_time: Option<Instant>,
    /// Cached engine lines to avoid recreating every frame
    /// Using Arc for O(1) clone in view function (thread-safe for iced Primitive)
    pub cached_engine_lines:
        Option<std::sync::Arc<Vec<crate::features::lyrics::engine::LyricLineData>>>,
    /// Cached shaped lines (pre-computed in background thread)
    /// 文本布局的唯一数据源
    pub cached_shaped_lines:
        Option<std::sync::Arc<Vec<crate::features::lyrics::engine::CachedShapedLine>>>,
    /// Shared font system for async text shaping (created asynchronously at app startup)
    pub shared_font_system: Option<crate::features::lyrics::engine::SharedFontSystem>,
    /// Resolved once on initialization or selection, shared by all text modes.
    pub display_font: iced::Font,
    pub font_families: Vec<String>,

    // Viewport info for line height calculations
    /// Last known viewport width (in logical pixels)
    pub viewport_width: f32,
    /// Last known viewport height (in logical pixels)
    pub viewport_height: f32,
    /// Whether viewport metrics have been initialized from a real layout/window event.
    pub viewport_initialized: bool,
    /// Last viewport size reported while the lyrics page transition was animating.
    pub pending_viewport_size: Option<iced::Size>,
    /// Content width used by the latest accepted shaped lines.
    pub shaped_content_width: f32,
    /// Main font size used by the latest accepted shaped lines.
    pub shaped_font_size: f32,
    // Display loading state
    /// Whether lyrics for the current display target are currently being loaded.
    pub is_loading: bool,
    /// Error message for the current display target.
    pub load_error: Option<String>,
}

pub struct DialogState {
    pub import_open: bool,
}

/// Discover page view mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverViewMode {
    /// Default view showing all home sections with single-row limits
    #[default]
    Overview,
    /// Full view of recommended playlists
    AllRecommended,
    /// Full view of ordinary hot playlists
    AllHot,
    /// Full view of official high-quality playlists
    AllOfficial,
}

/// Search tab types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SearchTab {
    #[default]
    Songs,
    Artists,
    Albums,
    Playlists,
    Videos,
    Radios,
}

impl SearchTab {
    /// Get the NCM API search type code
    pub fn to_search_type(self) -> crate::api::SearchType {
        match self {
            SearchTab::Songs => crate::api::SearchType::Songs,
            SearchTab::Artists => crate::api::SearchType::Artists,
            SearchTab::Albums => crate::api::SearchType::Albums,
            SearchTab::Playlists => crate::api::SearchType::Playlists,
            SearchTab::Videos => crate::api::SearchType::Videos,
            SearchTab::Radios => crate::api::SearchType::Radios,
        }
    }
}

/// Search page state
pub const SEARCH_PAGE_SIZE: u32 = 50;

#[derive(Debug, Default)]
pub struct SearchSuggestionsState {
    pub generation: u64,
    pub open: bool,
    pub loading: bool,
    pub items: Vec<crate::api::SearchSuggestion>,
    pub selected: Option<usize>,
    pub pending: Option<iced::task::Handle>,
}

impl SearchSuggestionsState {
    pub fn accepts(&self, generation: u64) -> bool {
        self.open && self.generation == generation
    }

    pub fn close(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.open = false;
        self.loading = false;
        self.selected = None;
        if let Some(handle) = self.pending.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod search_suggestion_tests {
    use super::SearchSuggestionsState;

    #[test]
    fn editing_dismissal_and_reopening_reject_queued_responses_and_cancel_work() {
        let (_, pending) = iced::Task::perform(async {}, |_| ()).abortable();
        let observer = pending.clone();
        let mut state = SearchSuggestionsState {
            open: true,
            loading: true,
            pending: Some(pending),
            selected: Some(2),
            ..Default::default()
        };
        let old_generation = state.generation;
        assert!(state.accepts(old_generation));
        state.close();
        assert!(observer.is_aborted());
        assert!(!state.loading);
        assert!(state.selected.is_none());
        assert!(!state.accepts(old_generation));
        state.open = true;
        assert!(!state.accepts(old_generation));
        assert!(state.accepts(state.generation));
        state.close();
        assert!(!state.accepts(state.generation));
    }
}

pub struct SearchPageState {
    pub request_generation: u64,
    pub error: Option<crate::error::AppError>,
    pub suggestions: SearchSuggestionsState,
    pub page_input: String,
    pub song_views: Vec<crate::ui::components::playlist_view::SongItem>,
    /// Current search keyword
    pub keyword: String,
    /// Active search tab
    pub active_tab: SearchTab,
    /// Song search results
    pub tracks: Vec<Track>,
    /// Album search results
    pub albums: Vec<AlbumSummary>,
    /// Artist search results
    pub artists: Vec<ArtistSummary>,
    /// Playlist search results
    pub playlists: Vec<PlaylistSummary>,
    /// Video / MV search results
    pub videos: Vec<VideoSummary>,
    /// Podcast / DJ radio search results
    pub radios: Vec<RadioSummary>,
    /// Total count for pagination
    pub total_count: u32,
    /// Current page (0-indexed)
    pub current_page: u32,
    /// Loading state
    pub loading: bool,
    /// Virtual list scroll state for efficient rendering of search results
    pub scroll_state: std::rc::Rc<std::cell::RefCell<crate::ui::widgets::VirtualListState>>,
    /// Hover animations for song list
    pub song_animations: HoverAnimations<i64>,
    /// Hover animations for grid cards
    pub card_animations: HoverAnimations<u64>,
}

impl Default for SearchPageState {
    fn default() -> Self {
        Self {
            request_generation: 0,
            error: None,
            suggestions: Default::default(),
            page_input: "1".into(),
            song_views: Vec::new(),
            keyword: String::new(),
            active_tab: SearchTab::default(),
            tracks: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            playlists: Vec::new(),
            videos: Vec::new(),
            radios: Vec::new(),
            total_count: 0,
            current_page: 0,
            loading: false,
            scroll_state: std::rc::Rc::new(std::cell::RefCell::new(
                crate::ui::widgets::VirtualListState::default(),
            )),
            song_animations: Default::default(),
            card_animations: Default::default(),
        }
    }
}

/// Discover page state for browsing playlists
const DISCOVER_CARD_HOVER_DURATION: Duration = Duration::from_millis(240);

pub struct DiscoverPageState {
    /// Current view mode
    pub view_mode: DiscoverViewMode,
    /// Identity of the latest discover-page request batch.
    pub load_generation: u64,
    /// Recommended playlists (for logged-in users)
    pub recommended_playlists: Vec<PlaylistSummary>,
    /// First Daily Recommend track used as the feature-card cover.
    pub daily_recommend_preview: Option<Track>,
    /// First prefetched Personal FM track used as the feature-card cover.
    pub personal_fm_preview: Option<Track>,
    /// Prefetched Personal FM batch consumed by the next Radio-route activation.
    pub personal_fm_prefetched_tracks: Vec<Track>,
    /// SPlayer-compatible Private Radar playlist metadata.
    pub private_radar: Option<PlaylistSummary>,
    /// Ordinary hot playlists (for all users).
    pub hot_playlists: Vec<PlaylistSummary>,
    /// Official high-quality playlists (for all users).
    pub official_playlists: Vec<PlaylistSummary>,
    /// Hover animations for playlist cards
    pub card_animations: HoverAnimations<u64>,
    /// Loading state for recommended playlists
    pub recommended_loading: bool,
    /// Loading state for the Private Radar feature card.
    pub private_radar_loading: bool,
    /// Loading state for ordinary hot playlists.
    pub hot_loading: bool,
    /// Loading state for official high-quality playlists.
    pub official_loading: bool,
    /// Whether data has been loaded (to avoid re-fetching)
    pub data_loaded: bool,
}

impl Default for DiscoverPageState {
    fn default() -> Self {
        Self {
            view_mode: DiscoverViewMode::default(),
            load_generation: 0,
            recommended_playlists: Vec::new(),
            daily_recommend_preview: None,
            personal_fm_preview: None,
            personal_fm_prefetched_tracks: Vec::new(),
            private_radar: None,
            hot_playlists: Vec::new(),
            official_playlists: Vec::new(),
            card_animations: HoverAnimations::with_duration(DISCOVER_CARD_HOVER_DURATION),
            recommended_loading: false,
            private_radar_loading: false,
            hot_loading: false,
            official_loading: false,
            data_loaded: false,
        }
    }
}

/// Homepage state for NCM data
pub struct HomePageState {
    // Login popup
    pub login_popup_open: bool,
    pub qr_code_path: Option<PathBuf>,
    pub qr_unikey: Option<String>,
    pub qr_status: Option<String>,

    pub user_playlists: Vec<PlaylistSummary>,
    /// Current NCM playlist songs (for playback)
    pub current_ncm_playlist_songs: Vec<Track>,
}

#[cfg(test)]
mod audio_output_device_option_tests {
    use super::*;

    #[test]
    fn duplicate_names_keep_unique_stable_id_labels_and_values() {
        let devices = [
            AudioDevice {
                id: "wasapi:device-a".to_string(),
                name: "Speakers".to_string(),
                is_default: true,
            },
            AudioDevice {
                id: "wasapi:device-b".to_string(),
                name: "Speakers".to_string(),
                is_default: false,
            },
        ];

        assert_eq!(
            audio_output_device_options(&devices),
            vec![
                (
                    "wasapi:device-a".to_string(),
                    "Speakers (wasapi:device-a)".to_string(),
                ),
                (
                    "wasapi:device-b".to_string(),
                    "Speakers (wasapi:device-b)".to_string(),
                ),
            ]
        );
    }
}
