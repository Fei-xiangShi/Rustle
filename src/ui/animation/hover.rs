//! Hover animation state with project-owned scalar transitions.
//!
//! Only the active and fading keys are retained, keeping updates O(1) while
//! avoiding a second, incompatible Iced core dependency through `iced_anim`.

use std::time::{Duration, Instant};

/// Default hover animation duration (200ms for a snappy feel).
const HOVER_DURATION: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy)]
enum TransitionCurve {
    Ease,
    EaseOut,
}

impl TransitionCurve {
    fn sample(self, progress: f32) -> f32 {
        match self {
            Self::Ease => cubic_bezier(progress, 0.25, 0.1, 0.25, 1.0),
            Self::EaseOut => cubic_bezier(progress, 0.0, 0.0, 0.58, 1.0),
        }
    }
}

#[derive(Debug, Clone)]
struct ScalarTransition {
    start: f32,
    value: f32,
    target: f32,
    elapsed: Duration,
    duration: Duration,
    last_update: Instant,
    curve: TransitionCurve,
}

impl ScalarTransition {
    fn new(value: f32, duration: Duration, curve: TransitionCurve) -> Self {
        let value = value.clamp(0.0, 1.0);
        Self {
            start: value,
            value,
            target: value,
            elapsed: duration,
            duration,
            last_update: Instant::now(),
            curve,
        }
    }

    fn set_target(&mut self, target: f32) {
        self.set_target_at(target, Instant::now());
    }

    fn set_target_at(&mut self, target: f32, now: Instant) {
        let target = target.clamp(0.0, 1.0);
        if self.target == target {
            return;
        }

        self.start = self.value;
        self.target = target;
        self.elapsed = Duration::ZERO;
        self.last_update = now;

        if self.duration.is_zero() {
            self.value = target;
        }
    }

    fn settle_at(&mut self, value: f32) {
        let value = value.clamp(0.0, 1.0);
        self.start = value;
        self.value = value;
        self.target = value;
        self.elapsed = self.duration;
        self.last_update = Instant::now();
    }

    fn tick(&mut self, now: Instant) {
        if !self.is_animating() {
            return;
        }

        let delta = now
            .checked_duration_since(self.last_update)
            .unwrap_or_default();
        self.last_update = now;
        self.elapsed = self.elapsed.saturating_add(delta).min(self.duration);

        if self.elapsed >= self.duration {
            self.value = self.target;
            return;
        }

        let progress = self.elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let eased = self.curve.sample(progress);
        self.value = self.start + (self.target - self.start) * eased;
    }

    fn is_animating(&self) -> bool {
        self.value != self.target
    }
}

/// Optimized hover animation manager for exclusive hover states.
///
/// Only one item can be hovered at a time, so the manager retains the current
/// item and at most one previous item fading out.
#[derive(Debug, Clone)]
pub struct HoverAnimations<K: Eq + Clone> {
    duration: Duration,
    active_key: Option<K>,
    active_anim: ScalarTransition,
    fading_key: Option<K>,
    fading_anim: ScalarTransition,
}

impl<K: Eq + Clone> Default for HoverAnimations<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Clone> HoverAnimations<K> {
    /// Create a new empty hover animation manager.
    pub fn new() -> Self {
        Self::with_duration(HOVER_DURATION)
    }

    /// Create a hover animation manager with a custom transition duration.
    pub fn with_duration(duration: Duration) -> Self {
        Self {
            duration,
            active_key: None,
            active_anim: ScalarTransition::new(0.0, duration, TransitionCurve::EaseOut),
            fading_key: None,
            fading_anim: ScalarTransition::new(0.0, duration, TransitionCurve::EaseOut),
        }
    }

    /// Set the exclusively hovered item, or `None` to unhover all items.
    pub fn set_hovered_exclusive(&mut self, key: Option<K>) {
        if self.active_key == key {
            return;
        }

        let now = Instant::now();
        match (&self.active_key, key) {
            (Some(_), Some(new_key)) => {
                if let Some(old_key) = self.active_key.take() {
                    self.fading_key = Some(old_key);
                    self.fading_anim = ScalarTransition::new(
                        self.active_anim.value,
                        self.duration,
                        TransitionCurve::EaseOut,
                    );
                    self.fading_anim.set_target_at(0.0, now);
                }

                self.active_key = Some(new_key);
                self.active_anim =
                    ScalarTransition::new(0.0, self.duration, TransitionCurve::EaseOut);
                self.active_anim.set_target_at(1.0, now);
            }
            (None, Some(new_key)) => {
                self.active_key = Some(new_key);
                self.active_anim =
                    ScalarTransition::new(0.0, self.duration, TransitionCurve::EaseOut);
                self.active_anim.set_target_at(1.0, now);
            }
            (Some(_), None) => {
                if let Some(old_key) = self.active_key.take() {
                    self.fading_key = Some(old_key);
                    self.fading_anim = ScalarTransition::new(
                        self.active_anim.value,
                        self.duration,
                        TransitionCurve::EaseOut,
                    );
                    self.fading_anim.set_target_at(0.0, now);
                }
            }
            (None, None) => {}
        }
    }

    /// Return interpolated progress for a key in the inclusive range `[0, 1]`.
    pub fn get_progress(&self, key: &K) -> f32 {
        if self.active_key.as_ref() == Some(key) {
            self.active_anim.value
        } else if self.fading_key.as_ref() == Some(key) {
            self.fading_anim.value
        } else {
            0.0
        }
    }

    /// Interpolate between two scalar values using a key's hover progress.
    pub fn interpolate_f32(&self, key: &K, from: f32, to: f32) -> f32 {
        from + (to - from) * self.get_progress(key)
    }

    /// Return whether either retained transition is still moving.
    pub fn is_animating(&self) -> bool {
        self.active_anim.is_animating() || self.fading_anim.is_animating()
    }

    /// Remove a completed fade-out key.
    pub fn cleanup_completed(&mut self) {
        if self.fading_key.is_some()
            && self.fading_anim.value < 0.01
            && self.fading_anim.value == self.fading_anim.target
        {
            self.fading_key = None;
        }
    }

    /// Clear all animation state.
    pub fn clear(&mut self) {
        self.active_key = None;
        self.fading_key = None;
        self.active_anim = ScalarTransition::new(0.0, self.duration, TransitionCurve::EaseOut);
        self.fading_anim = ScalarTransition::new(0.0, self.duration, TransitionCurve::EaseOut);
    }

    /// Advance retained transitions to `now`.
    pub fn tick(&mut self, now: Instant) {
        self.active_anim.tick(now);
        self.fading_anim.tick(now);
    }
}

/// Single hover animation state for dialogs and standalone buttons.
#[derive(Debug)]
pub struct SingleHoverAnimation {
    animation: ScalarTransition,
}

impl Default for SingleHoverAnimation {
    fn default() -> Self {
        Self::new()
    }
}

impl SingleHoverAnimation {
    /// Create a new single hover animation.
    pub fn new() -> Self {
        Self::with_duration(HOVER_DURATION)
    }

    /// Create a single animation with a custom transition duration.
    pub fn with_duration(duration: Duration) -> Self {
        Self {
            animation: ScalarTransition::new(0.0, duration, TransitionCurve::Ease),
        }
    }

    /// Animate to the active state.
    pub fn start(&mut self) {
        self.animation.set_target(1.0);
    }

    /// Animate to the inactive state.
    pub fn stop(&mut self) {
        self.animation.set_target(0.0);
    }

    /// Immediately finish at a specific progress value.
    pub fn settle_at(&mut self, progress: f32) {
        self.animation.settle_at(progress);
    }

    /// Return current progress in the inclusive range `[0, 1]`.
    pub fn progress(&self) -> f32 {
        self.animation.value
    }

    /// Return whether the transition is still moving.
    pub fn is_animating(&self) -> bool {
        self.animation.is_animating()
    }

    /// Advance the transition to `now`.
    pub fn tick(&mut self, now: Instant) {
        self.animation.tick(now);
    }
}

fn cubic_bezier(progress: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    if progress == 0.0 || progress == 1.0 {
        return progress;
    }

    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..20 {
        let candidate = (low + high) * 0.5;
        if cubic_coordinate(candidate, x1, x2) < progress {
            low = candidate;
        } else {
            high = candidate;
        }
    }
    cubic_coordinate((low + high) * 0.5, y1, y2)
}

fn cubic_coordinate(time: f32, control1: f32, control2: f32) -> f32 {
    let inverse = 1.0 - time;
    3.0 * inverse * inverse * time * control1
        + 3.0 * inverse * time * time * control2
        + time * time * time
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DURATION: Duration = Duration::from_millis(200);

    #[test]
    fn css_easing_curves_match_reference_midpoints() {
        assert!((TransitionCurve::Ease.sample(0.5) - 0.8024).abs() < 0.001);
        assert!((TransitionCurve::EaseOut.sample(0.5) - 0.6846).abs() < 0.001);
    }

    #[test]
    fn scalar_transition_reaches_exact_endpoints() {
        let start = Instant::now();
        let mut transition = ScalarTransition::new(0.0, TEST_DURATION, TransitionCurve::EaseOut);
        transition.set_target_at(1.0, start);
        transition.tick(start + TEST_DURATION / 2);
        assert!((transition.value - 0.6846).abs() < 0.001);
        assert!(transition.is_animating());

        transition.tick(start + TEST_DURATION);
        assert_eq!(transition.value, 1.0);
        assert!(!transition.is_animating());
    }

    #[test]
    fn interrupted_transition_restarts_from_current_value() {
        let start = Instant::now();
        let mut transition = ScalarTransition::new(0.0, TEST_DURATION, TransitionCurve::Ease);
        transition.set_target_at(1.0, start);
        transition.tick(start + TEST_DURATION / 2);
        let interrupted_value = transition.value;

        transition.set_target_at(0.0, start + TEST_DURATION / 2);
        assert_eq!(transition.start, interrupted_value);
        transition.tick(start + TEST_DURATION);
        assert!(transition.value > 0.0);
        assert!(transition.value < interrupted_value);
        transition.tick(start + TEST_DURATION + TEST_DURATION / 2);
        assert_eq!(transition.value, 0.0);
    }

    #[test]
    fn exclusive_hover_switches_and_cleans_up_old_key() {
        let mut animations: HoverAnimations<i64> = HoverAnimations::with_duration(TEST_DURATION);
        animations.set_hovered_exclusive(Some(1));
        animations.tick(Instant::now() + TEST_DURATION);
        assert_eq!(animations.get_progress(&1), 1.0);

        animations.set_hovered_exclusive(Some(2));
        assert_eq!(animations.active_key, Some(2));
        assert_eq!(animations.fading_key, Some(1));
        animations.tick(Instant::now() + TEST_DURATION);
        assert_eq!(animations.get_progress(&2), 1.0);
        assert_eq!(animations.get_progress(&1), 0.0);
        animations.cleanup_completed();
        assert_eq!(animations.fading_key, None);
    }

    #[test]
    fn single_animation_settles_and_clamps() {
        let mut animation = SingleHoverAnimation::new();
        animation.start();
        assert!(animation.is_animating());
        animation.settle_at(2.0);
        assert_eq!(animation.progress(), 1.0);
        assert!(!animation.is_animating());
        animation.stop();
        animation.tick(Instant::now() + TEST_DURATION);
        assert_eq!(animation.progress(), 0.0);
    }
}
