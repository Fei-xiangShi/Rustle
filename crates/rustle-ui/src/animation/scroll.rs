//! Shared smooth-scrolling motion state.

use iced::time::Instant;
use std::time::Duration;

const SCROLL_DURATION: Duration = Duration::from_millis(190);
const MIN_DISTANCE: f32 = 0.01;

/// A scroll surface whose offset can be advanced by the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmoothScrollTarget {
    /// An Iced scrollable identified by its widget ID.
    Native(&'static str),
    /// The virtualized local-playlist song list.
    PlaylistSongs,
    /// The virtualized search-results song list.
    SearchSongs,
}

/// The input that initiated a smooth-scroll segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmoothScrollSource {
    Wheel,
    Programmatic,
}

/// Input events emitted by scroll widgets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SmoothScrollEvent {
    Requested {
        target: SmoothScrollTarget,
        delta: f32,
    },
    Cancelled {
        target: SmoothScrollTarget,
    },
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    target: SmoothScrollTarget,
    source: SmoothScrollSource,
    total_distance: f32,
    emitted_distance: f32,
    started_at: Instant,
}

impl Segment {
    fn remaining(self) -> f32 {
        self.total_distance - self.emitted_distance
    }
}

/// Owns the single active smooth-scroll segment.
#[derive(Debug, Default)]
pub struct SmoothScrollState {
    active: Option<Segment>,
}

impl SmoothScrollState {
    /// Starts or extends a wheel-driven segment.
    ///
    /// Repeated input in the same direction accumulates the un-emitted
    /// distance. Reverse input discards that remainder so direction changes
    /// are immediately responsive.
    pub fn request_wheel(&mut self, target: SmoothScrollTarget, delta: f32, now: Instant) {
        if delta.abs() < MIN_DISTANCE {
            return;
        }

        let distance = match self.active {
            Some(segment)
                if segment.target == target
                    && segment.source == SmoothScrollSource::Wheel
                    && segment.remaining().signum() == delta.signum() =>
            {
                segment.remaining() + delta
            }
            _ => delta,
        };

        self.start(target, SmoothScrollSource::Wheel, distance, now);
    }

    /// Replaces the active segment with a programmatic relative movement.
    pub fn request_programmatic(&mut self, target: SmoothScrollTarget, delta: f32, now: Instant) {
        self.start(target, SmoothScrollSource::Programmatic, delta, now);
    }

    /// Cancels the active segment when it belongs to `target`.
    pub fn cancel(&mut self, target: SmoothScrollTarget) {
        if self.active.is_some_and(|segment| segment.target == target) {
            self.active = None;
        }
    }

    /// Cancels any active smooth scrolling.
    pub fn cancel_all(&mut self) {
        self.active = None;
    }

    /// Consumes the active segment and returns its exact un-emitted distance.
    pub fn take_remaining(&mut self) -> Option<(SmoothScrollTarget, f32)> {
        self.active
            .take()
            .map(|segment| (segment.target, segment.remaining()))
    }

    /// Advances the active segment and returns the next relative pixel step.
    pub fn tick(&mut self, now: Instant) -> Option<(SmoothScrollTarget, f32)> {
        let segment = self.active.as_mut()?;
        let elapsed = now.saturating_duration_since(segment.started_at);
        let progress = (elapsed.as_secs_f32() / SCROLL_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        let desired_distance = segment.total_distance * eased;
        let step = if progress >= 1.0 {
            segment.total_distance - segment.emitted_distance
        } else {
            desired_distance - segment.emitted_distance
        };
        let target = segment.target;

        segment.emitted_distance += step;
        if progress >= 1.0 {
            self.active = None;
        }

        Some((target, step))
    }

    pub fn is_animating(&self) -> bool {
        self.active.is_some()
    }

    fn start(
        &mut self,
        target: SmoothScrollTarget,
        source: SmoothScrollSource,
        distance: f32,
        now: Instant,
    ) {
        if distance.abs() < MIN_DISTANCE {
            self.active = None;
            return;
        }

        self.active = Some(Segment {
            target,
            source,
            total_distance: distance,
            emitted_distance: 0.0,
            started_at: now,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: SmoothScrollTarget = SmoothScrollTarget::Native("test_scroll");

    #[test]
    fn finishes_with_the_exact_requested_distance() {
        let start = Instant::now();
        let mut state = SmoothScrollState::default();
        state.request_wheel(TARGET, 120.0, start);

        let mut emitted = 0.0;
        for elapsed_ms in [40, 90, 140, 190] {
            emitted += state
                .tick(start + Duration::from_millis(elapsed_ms))
                .unwrap()
                .1;
        }

        assert!((emitted - 120.0).abs() < f32::EPSILON);
        assert!(!state.is_animating());
    }

    #[test]
    fn repeated_wheel_input_accumulates_only_the_remaining_distance() {
        let start = Instant::now();
        let mut state = SmoothScrollState::default();
        state.request_wheel(TARGET, 100.0, start);
        let first_step = state.tick(start + Duration::from_millis(95)).unwrap().1;

        state.request_wheel(TARGET, 50.0, start + Duration::from_millis(95));
        let (_, remaining) = state.take_remaining().unwrap();

        assert!((first_step + remaining - 150.0).abs() < 0.001);
    }

    #[test]
    fn reverse_wheel_input_redirects_immediately() {
        let start = Instant::now();
        let mut state = SmoothScrollState::default();
        state.request_wheel(TARGET, 100.0, start);
        let _ = state.tick(start + Duration::from_millis(60));

        state.request_wheel(TARGET, -40.0, start + Duration::from_millis(60));

        assert_eq!(state.take_remaining(), Some((TARGET, -40.0)));
    }

    #[test]
    fn wheel_input_takes_over_programmatic_scrolling() {
        let start = Instant::now();
        let mut state = SmoothScrollState::default();
        state.request_programmatic(TARGET, 400.0, start);
        let _ = state.tick(start + Duration::from_millis(50));

        state.request_wheel(TARGET, -50.0, start + Duration::from_millis(50));

        assert_eq!(state.take_remaining(), Some((TARGET, -50.0)));
    }

    #[test]
    fn cancellation_only_affects_the_matching_target() {
        let start = Instant::now();
        let mut state = SmoothScrollState::default();
        state.request_wheel(TARGET, 100.0, start);

        state.cancel(SmoothScrollTarget::PlaylistSongs);
        assert!(state.is_animating());

        state.cancel(TARGET);
        assert!(!state.is_animating());
    }
}
