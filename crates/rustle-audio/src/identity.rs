use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlaybackGeneration(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeekNonce(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreloadIdentity {
    pub generation: PlaybackGeneration,
    pub request_id: u64,
    /// Cancellation ownership captured when this preload was reserved.
    ///
    /// A completion must be rejected when either this token or the active
    /// generation has been cancelled. Keeping the token on the identity also
    /// prevents completion handlers from reconstructing ownership later.
    pub cancellation: GenerationCancellation,
}

#[derive(Debug)]
struct CancellationState {
    cancelled: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct GenerationCancellation(Arc<CancellationState>);

impl PartialEq for GenerationCancellation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for GenerationCancellation {}

impl GenerationCancellation {
    fn new() -> Self {
        Self(Arc::new(CancellationState {
            cancelled: AtomicBool::new(false),
        }))
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackContext {
    pub generation: PlaybackGeneration,
    pub cancellation: GenerationCancellation,
}

#[derive(Debug)]
struct ControllerState {
    next_generation: u64,
    active: Option<PlaybackContext>,
    next_seek_nonce: u64,
    next_request_id: u64,
}

#[derive(Debug, Clone)]
pub struct PlaybackGenerationController(Arc<Mutex<ControllerState>>);

impl Default for PlaybackGenerationController {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackGenerationController {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ControllerState {
            next_generation: 0,
            active: None,
            next_seek_nonce: 0,
            next_request_id: 0,
        })))
    }

    pub fn activate_generation(&self) -> PlaybackContext {
        let mut state = self.0.lock().expect("generation controller poisoned");
        if let Some(active) = &state.active {
            active.cancellation.cancel();
        }
        state.next_generation = state.next_generation.wrapping_add(1).max(1);
        state.next_seek_nonce = 0;
        let context = PlaybackContext {
            generation: PlaybackGeneration(state.next_generation),
            cancellation: GenerationCancellation::new(),
        };
        state.active = Some(context.clone());
        context
    }

    /// Promote a preload without cancelling the immutable operation token
    /// that owns its streaming source. The generation still advances, so all
    /// callbacks from the outgoing logical track become stale immediately.
    pub fn activate_preloaded_generation(
        &self,
        identity: &PreloadIdentity,
    ) -> Option<PlaybackContext> {
        let mut state = self.0.lock().expect("generation controller poisoned");
        let active = state.active.as_ref()?;
        if active.generation != identity.generation
            || active.cancellation != identity.cancellation
            || identity.cancellation.is_cancelled()
        {
            return None;
        }
        state.next_generation = state.next_generation.wrapping_add(1).max(1);
        state.next_seek_nonce = 0;
        let context = PlaybackContext {
            generation: PlaybackGeneration(state.next_generation),
            cancellation: identity.cancellation.clone(),
        };
        state.active = Some(context.clone());
        Some(context)
    }

    pub fn next_request_id(&self) -> u64 {
        let mut state = self.0.lock().expect("generation controller poisoned");
        state.next_request_id = state.next_request_id.wrapping_add(1).max(1);
        state.next_request_id
    }
    pub fn reserve_preload_identity(&self) -> Option<PreloadIdentity> {
        let mut state = self.0.lock().expect("generation controller poisoned");
        let active = state.active.as_ref()?;
        let generation = active.generation;
        let cancellation = active.cancellation.clone();
        state.next_request_id = state.next_request_id.wrapping_add(1).max(1);
        Some(PreloadIdentity {
            generation,
            request_id: state.next_request_id,
            cancellation,
        })
    }

    /// Reserve a new sink identity only from an accepted download identity.
    ///
    /// This keeps the handoff tied to the identity captured when the async
    /// download was created; callers must not reconstruct ownership from the
    /// currently visible playback state.
    pub fn reserve_preload_handoff(&self, parent: &PreloadIdentity) -> Option<PreloadIdentity> {
        let mut state = self.0.lock().expect("generation controller poisoned");
        let active = state.active.as_ref()?;
        if active.generation != parent.generation
            || active.cancellation != parent.cancellation
            || parent.cancellation.is_cancelled()
            || active.cancellation.is_cancelled()
        {
            return None;
        }
        state.next_request_id = state.next_request_id.wrapping_add(1).max(1);
        Some(PreloadIdentity {
            generation: parent.generation,
            request_id: state.next_request_id,
            cancellation: parent.cancellation.clone(),
        })
    }

    pub fn active_context(&self) -> Option<PlaybackContext> {
        self.0
            .lock()
            .expect("generation controller poisoned")
            .active
            .clone()
    }

    pub fn seek_context(&self) -> Option<(PlaybackContext, SeekNonce)> {
        let mut state = self.0.lock().expect("generation controller poisoned");
        let context = state.active.clone()?;
        state.next_seek_nonce = state.next_seek_nonce.wrapping_add(1).max(1);
        Some((context, SeekNonce(state.next_seek_nonce)))
    }

    pub fn cancel_active(&self) {
        if let Some(active) = self
            .0
            .lock()
            .expect("generation controller poisoned")
            .active
            .as_ref()
        {
            active.cancellation.cancel();
        }
    }

    pub fn accepts(&self, context: &PlaybackContext) -> bool {
        let state = self.0.lock().expect("generation controller poisoned");
        state.active.as_ref().is_some_and(|active| {
            active.generation == context.generation && !context.cancellation.is_cancelled()
        })
    }

    pub fn accepts_preload(&self, identity: &PreloadIdentity) -> bool {
        let state = self.0.lock().expect("generation controller poisoned");
        state.active.as_ref().is_some_and(|active| {
            active.generation == identity.generation
                && active.cancellation == identity.cancellation
                && !identity.cancellation.is_cancelled()
                && !active.cancellation.is_cancelled()
        })
    }

    pub fn accepts_seek(&self, context: &PlaybackContext, nonce: SeekNonce) -> bool {
        let state = self.0.lock().expect("generation controller poisoned");
        state.active.as_ref().is_some_and(|active| {
            active.generation == context.generation
                && !context.cancellation.is_cancelled()
                && state.next_seek_nonce != 0
                && nonce == SeekNonce(state.next_seek_nonce)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_invalidates_previous_generation() {
        let controller = PlaybackGenerationController::new();
        let first = controller.activate_generation();
        let second = controller.activate_generation();
        assert!(first.cancellation.is_cancelled());
        assert!(!second.cancellation.is_cancelled());
        assert!(!controller.accepts(&first));
        assert!(controller.accepts(&second));
    }

    #[test]
    fn seek_nonce_is_nonzero_and_preserved_per_context() {
        let controller = PlaybackGenerationController::new();
        let context = controller.activate_generation();
        let (captured, nonce) = controller.seek_context().unwrap();
        assert_eq!(captured.generation, context.generation);
        assert_ne!(nonce, SeekNonce(0));
        assert!(controller.accepts_seek(&captured, nonce));
        assert_eq!(
            controller.active_context().unwrap().generation,
            captured.generation
        );
    }

    #[test]
    fn newer_seek_nonce_rejects_older_seek_results() {
        let controller = PlaybackGenerationController::new();
        controller.activate_generation();
        let (first_context, first_nonce) = controller.seek_context().unwrap();
        let (second_context, second_nonce) = controller.seek_context().unwrap();

        assert!(!controller.accepts_seek(&first_context, first_nonce));
        assert!(controller.accepts_seek(&second_context, second_nonce));
    }

    #[test]
    fn preload_reservation_captures_active_generation_and_unique_nonzero_ids() {
        let controller = PlaybackGenerationController::new();
        assert!(controller.reserve_preload_identity().is_none());

        let context = controller.activate_generation();
        let first = controller.reserve_preload_identity().unwrap();
        let second = controller.reserve_preload_identity().unwrap();

        assert_eq!(first.cancellation, context.cancellation);
        assert_eq!(second.cancellation, context.cancellation);
        assert_ne!(first.request_id, 0);
        assert_ne!(second.request_id, 0);
        assert_ne!(first.request_id, second.request_id);
    }

    #[test]
    fn same_generation_with_different_cancellation_ownership_is_rejected() {
        let controller = PlaybackGenerationController::new();
        let context = controller.activate_generation();
        let identity = PreloadIdentity {
            generation: context.generation,
            request_id: 1,
            cancellation: GenerationCancellation::new(),
        };

        assert!(!controller.accepts_preload(&identity));
        assert!(!identity.cancellation.is_cancelled());
        assert!(!context.cancellation.is_cancelled());
    }

    #[test]
    fn cancelled_preload_identity_is_rejected_without_generation_change() {
        let controller = PlaybackGenerationController::new();
        let context = controller.activate_generation();
        let identity = controller.reserve_preload_identity().unwrap();

        assert!(controller.accepts_preload(&identity));
        identity.cancellation.cancel();
        assert!(!controller.accepts_preload(&identity));
        assert!(context.cancellation.is_cancelled());
    }
    #[test]
    fn stale_preload_identity_is_rejected_after_generation_change() {
        let controller = PlaybackGenerationController::new();
        let first = controller.activate_generation();
        let identity = controller.reserve_preload_identity().unwrap();
        assert!(controller.accepts_preload(&identity));
        let second = controller.activate_generation();
        assert!(!controller.accepts_preload(&identity));
        assert!(controller.accepts(&second));
        assert!(first.cancellation.is_cancelled());
    }

    #[test]
    fn stale_seek_is_rejected_before_side_effect() {
        let controller = PlaybackGenerationController::new();
        let first = controller.activate_generation();
        let (captured, nonce) = controller.seek_context().unwrap();
        let second = controller.activate_generation();
        assert!(!controller.accepts_seek(&captured, nonce));
        assert!(controller.accepts(&second));
        assert!(first.cancellation.is_cancelled());
    }

    #[test]
    fn valid_preload_handoff_preserves_generation_and_cancellation() {
        let controller = PlaybackGenerationController::new();
        let context = controller.activate_generation();
        let parent = controller.reserve_preload_identity().unwrap();

        let handoff = controller.reserve_preload_handoff(&parent).unwrap();

        assert_eq!(handoff.generation, parent.generation);
        assert_eq!(handoff.cancellation, parent.cancellation);
        assert_ne!(handoff.request_id, parent.request_id);
        assert_eq!(handoff.cancellation, context.cancellation);
        assert!(controller.accepts_preload(&handoff));
    }

    #[test]
    fn preload_promotion_advances_generation_without_cancelling_its_source() {
        let controller = PlaybackGenerationController::new();
        let old = controller.activate_generation();
        let identity = controller.reserve_preload_identity().unwrap();
        let promoted = controller.activate_preloaded_generation(&identity).unwrap();
        assert_ne!(promoted.generation, old.generation);
        assert_eq!(promoted.cancellation, identity.cancellation);
        assert!(!identity.cancellation.is_cancelled());
        assert!(!controller.accepts(&old));
        assert!(controller.accepts(&promoted));
    }

    #[test]
    fn stale_preload_handoff_is_rejected_after_generation_change() {
        let controller = PlaybackGenerationController::new();
        controller.activate_generation();
        let parent = controller.reserve_preload_identity().unwrap();
        controller.activate_generation();

        assert!(controller.reserve_preload_handoff(&parent).is_none());
    }

    #[test]
    fn cancelled_preload_handoff_is_rejected() {
        let controller = PlaybackGenerationController::new();
        controller.activate_generation();
        let parent = controller.reserve_preload_identity().unwrap();
        parent.cancellation.cancel();

        assert!(controller.reserve_preload_handoff(&parent).is_none());
    }

    #[test]
    fn preload_handoff_rejects_different_cancellation_owner() {
        let controller = PlaybackGenerationController::new();
        let context = controller.activate_generation();
        let parent = PreloadIdentity {
            generation: context.generation,
            request_id: 99,
            cancellation: GenerationCancellation::new(),
        };

        assert!(controller.reserve_preload_handoff(&parent).is_none());
    }

    #[test]
    fn shutdown_cancels_active_generation() {
        let controller = PlaybackGenerationController::new();
        let context = controller.activate_generation();
        controller.cancel_active();
        assert!(context.cancellation.is_cancelled());
        assert!(!controller.accepts(&context));
    }
}
