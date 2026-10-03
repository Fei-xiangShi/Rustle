//! Owns prepared lyrics and in-flight shaping for the current/adjacent song window.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::features::lyrics::engine::{CachedShapedLine, LyricLineData};

/// The complete immutable input and cancellation ownership of one background job.
#[derive(Debug, Clone)]
pub struct LyricsShapeRequest {
    pub song_id: i64,
    pub generation: u64,
    pub engine_lines: Arc<Vec<LyricLineData>>,
    pub content_width: f32,
    pub font_size: f32,
    pub font_family: Option<String>,
    pub cancelled: Arc<AtomicBool>,
}

impl LyricsShapeRequest {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn matches(&self, width: f32, size: f32, family: Option<&str>) -> bool {
        self.content_width == width
            && self.font_size == size
            && self.font_family.as_deref() == family
    }
}

#[derive(Debug, Default)]
pub struct LyricsRenderEntry {
    pub engine_lines: Option<Arc<Vec<LyricLineData>>>,
    pub shaped_lines: Option<Arc<Vec<CachedShapedLine>>>,
    pub content_width: f32,
    pub font_size: f32,
    pub font_family: Option<String>,
    pub shape_generation: u64,
    preparation_generation: Option<u64>,
    pending: Option<LyricsShapeRequest>,
}

impl LyricsRenderEntry {
    pub fn shaped_viewport_matches(&self, width: f32, size: f32, family: Option<&str>) -> bool {
        self.content_width == width
            && self.font_size == size
            && self.font_family.as_deref() == family
    }

    fn cancel_pending(&mut self) {
        if let Some(request) = self.pending.take() {
            request.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for LyricsRenderEntry {
    fn drop(&mut self) {
        self.cancel_pending();
    }
}

#[derive(Debug, Default)]
pub struct LyricsRenderManager {
    entries: HashMap<i64, LyricsRenderEntry>,
    next_generation: u64,
}

impl LyricsRenderManager {
    pub fn get(&self, song_id: i64) -> Option<&LyricsRenderEntry> {
        self.entries.get(&song_id)
    }

    /// Reserve adjacent preparation before dispatch, including a two-song queue
    /// where next and previous resolve to the same song.
    pub fn begin_engine_preparation(&mut self, song_id: i64) -> Option<u64> {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.entries.entry(song_id) {
            self.next_generation = self.next_generation.wrapping_add(1);
            let reservation = entry.insert(LyricsRenderEntry::default());
            reservation.preparation_generation = Some(self.next_generation);
            Some(self.next_generation)
        } else {
            None
        }
    }

    /// Release only the matching reservation. A failed read must be retryable;
    /// an evicted or superseded worker cannot release another job's ownership.
    pub fn finish_engine_preparation(
        &mut self,
        song_id: i64,
        generation: u64,
        success: bool,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&song_id) else {
            return false;
        };
        if entry.preparation_generation != Some(generation) {
            return false;
        }
        if success {
            entry.preparation_generation = None;
        } else {
            self.entries.remove(&song_id);
        }
        true
    }

    pub fn store_engine_lines(&mut self, song_id: i64, lines: Arc<Vec<LyricLineData>>) {
        let entry = self.entries.entry(song_id).or_default();
        entry.preparation_generation = None;
        if entry
            .engine_lines
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &lines))
        {
            return;
        }
        entry.cancel_pending();
        entry.shaped_lines = None;
        entry.engine_lines = Some(lines);
    }

    /// Register before spawning. Both display and preload callers use this gate.
    pub fn request_shape(
        &mut self,
        song_id: i64,
        content_width: f32,
        font_size: f32,
        font_family: Option<String>,
    ) -> Option<LyricsShapeRequest> {
        let entry = self.entries.get_mut(&song_id)?;
        let engine_lines = entry.engine_lines.clone()?;
        if entry.shaped_lines.is_some()
            && entry.shaped_viewport_matches(content_width, font_size, font_family.as_deref())
        {
            // Returning to a cached viewport also invalidates a pending resize.
            entry.cancel_pending();
            return None;
        }
        if entry.pending.as_ref().is_some_and(|request| {
            request.matches(content_width, font_size, font_family.as_deref())
        }) {
            return None;
        }
        entry.cancel_pending();
        self.next_generation = self.next_generation.wrapping_add(1);
        let request = LyricsShapeRequest {
            song_id,
            generation: self.next_generation,
            engine_lines,
            content_width,
            font_size,
            font_family,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        entry.pending = Some(request.clone());
        Some(request)
    }

    /// Never create entries or publish metrics for a stale/evicted completion.
    pub fn finish_shape(
        &mut self,
        song_id: i64,
        generation: u64,
        lines: Arc<Vec<CachedShapedLine>>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&song_id) else {
            return false;
        };
        if !entry
            .pending
            .as_ref()
            .is_some_and(|request| request.generation == generation)
        {
            return false;
        }
        let request = entry.pending.take().expect("matching pending request");
        entry.shaped_lines = Some(lines);
        entry.shape_generation = generation;
        entry.content_width = request.content_width;
        entry.font_size = request.font_size;
        entry.font_family = request.font_family;
        true
    }

    pub fn fail_shape(&mut self, song_id: i64, generation: u64) -> bool {
        let Some(entry) = self.entries.get_mut(&song_id) else {
            return false;
        };
        if !entry
            .pending
            .as_ref()
            .is_some_and(|request| request.generation == generation)
        {
            return false;
        }
        entry.cancel_pending();
        true
    }

    pub fn invalidate_shaping(&mut self) {
        for entry in self.entries.values_mut() {
            entry.cancel_pending();
            entry.shaped_lines = None;
        }
    }

    pub fn is_render_ready(
        &self,
        song_id: i64,
        width: f32,
        size: f32,
        family: Option<&str>,
    ) -> bool {
        self.entries.get(&song_id).is_some_and(|entry| {
            entry.shaped_lines.is_some() && entry.shaped_viewport_matches(width, size, family)
        })
    }

    pub fn retain(&mut self, song_ids: &[i64]) {
        self.entries.retain(|id, _| song_ids.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> LyricsRenderManager {
        let mut manager = LyricsRenderManager::default();
        manager.store_engine_lines(1, Arc::new(vec![LyricLineData::default()]));
        manager
    }

    fn request(manager: &mut LyricsRenderManager, family: &str) -> LyricsShapeRequest {
        manager
            .request_shape(1, 600.0, 48.0, Some(family.into()))
            .unwrap()
    }

    #[test]
    fn display_and_adjacent_requests_share_inflight_ownership() {
        let mut manager = manager();
        let job = request(&mut manager, "A");
        for _ in 0..120 {
            assert!(
                manager
                    .request_shape(1, 600.0, 48.0, Some("A".into()))
                    .is_none()
            );
        }
        assert!(!job.is_cancelled());
        assert!(manager.finish_shape(1, job.generation, Arc::new(Vec::new())));
        assert!(manager.is_render_ready(1, 600.0, 48.0, Some("A")));
    }

    #[test]
    fn font_switch_cancels_pending_and_rejects_out_of_order_completion() {
        let mut manager = manager();
        let first = request(&mut manager, "A");
        let second = request(&mut manager, "B");
        assert!(first.is_cancelled());
        assert!(!manager.finish_shape(1, first.generation, Arc::new(Vec::new())));
        assert!(!manager.fail_shape(1, first.generation));
        assert!(manager.finish_shape(1, second.generation, Arc::new(Vec::new())));
        assert!(!manager.finish_shape(1, first.generation, Arc::new(Vec::new())));
        assert!(manager.is_render_ready(1, 600.0, 48.0, Some("B")));
        assert!(!manager.is_render_ready(1, 600.0, 48.0, Some("A")));
    }

    #[test]
    fn returning_to_cached_viewport_cancels_resize() {
        let mut manager = manager();
        let first = request(&mut manager, "A");
        assert!(manager.finish_shape(1, first.generation, Arc::new(Vec::new())));
        let resize = manager
            .request_shape(1, 601.0, 48.0, Some("A".into()))
            .unwrap();
        assert!(
            manager
                .request_shape(1, 600.0, 48.0, Some("A".into()))
                .is_none()
        );
        assert!(resize.is_cancelled());
        assert!(!manager.finish_shape(1, resize.generation, Arc::new(Vec::new())));
        assert!(manager.is_render_ready(1, 600.0, 48.0, Some("A")));
    }

    #[test]
    fn same_length_new_lyrics_invalidate_shaping() {
        let mut manager = manager();
        let old = request(&mut manager, "A");
        manager.store_engine_lines(
            1,
            Arc::new(vec![LyricLineData {
                text: "new".into(),
                ..Default::default()
            }]),
        );
        assert!(old.is_cancelled());
        let new = request(&mut manager, "A");
        assert_ne!(new.generation, old.generation);
        assert_eq!(new.engine_lines[0].text, "new");
    }

    #[test]
    fn invalidation_preserves_source_but_not_jobs_or_font_cache() {
        let mut manager = manager();
        let old = request(&mut manager, "A");
        manager.invalidate_shaping();
        assert!(old.is_cancelled());
        let new = request(&mut manager, "A");
        assert!(Arc::ptr_eq(&old.engine_lines, &new.engine_lines));
        assert!(new.generation > old.generation);
        assert!(manager.fail_shape(1, new.generation));
        assert!(request(&mut manager, "A").generation > new.generation);
    }

    #[test]
    fn eviction_cancels_work_and_old_completion_cannot_resurrect_entry() {
        let mut manager = manager();
        let old = request(&mut manager, "A");
        manager.retain(&[]);
        assert!(old.is_cancelled());
        assert!(!manager.finish_shape(1, old.generation, Arc::new(Vec::new())));
        assert!(manager.get(1).is_none());
        manager.store_engine_lines(1, old.engine_lines.clone());
        let new = request(&mut manager, "A");
        assert!(new.generation > old.generation);
        assert!(!manager.finish_shape(1, old.generation, Arc::new(Vec::new())));
    }

    #[test]
    fn adjacent_engine_preparation_is_reserved_once() {
        let mut manager = manager();
        assert!(manager.begin_engine_preparation(2).is_some());
        assert!(manager.begin_engine_preparation(2).is_none());
    }

    #[test]
    fn failed_preparation_retries_and_old_completion_cannot_release_new_job() {
        let mut manager = manager();
        let first = manager.begin_engine_preparation(2).unwrap();
        assert!(manager.finish_engine_preparation(2, first, false));
        let retry = manager.begin_engine_preparation(2).unwrap();
        assert!(!manager.finish_engine_preparation(2, first, false));
        assert!(!manager.finish_engine_preparation(2, first, true));
        assert!(manager.begin_engine_preparation(2).is_none());
        manager.retain(&[]);
        let replacement = manager.begin_engine_preparation(2).unwrap();
        assert!(!manager.finish_engine_preparation(2, retry, true));
        assert!(manager.finish_engine_preparation(2, replacement, true));
    }

    #[test]
    fn display_result_supersedes_pending_adjacent_preparation() {
        let mut manager = manager();
        let generation = manager.begin_engine_preparation(2).unwrap();
        manager.store_engine_lines(2, Arc::new(vec![LyricLineData::default()]));
        assert!(!manager.finish_engine_preparation(2, generation, false));
        assert!(!manager.finish_engine_preparation(2, generation, true));
        assert!(manager.get(2).unwrap().engine_lines.is_some());
    }
}
