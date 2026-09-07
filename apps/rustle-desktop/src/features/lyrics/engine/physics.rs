//! Physics simulation for lyrics scrolling
//!
//! Implements scrolling physics with:
//! - Temporary manual scroll offsets
//! - Smooth return to auto-follow mode
//! - Bounds clamping for the current line layout

use super::spring::Spring;
use std::time::Instant;

/// Scroll state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollState {
    /// Auto-following playback
    AutoPlay,
    /// Idle, waiting to return to auto-play
    Idle,
}

/// Physics simulation for scrolling
#[derive(Debug)]
pub struct ScrollPhysics {
    /// Current scroll position in pixels
    scroll_y: f32,
    /// Current velocity in pixels per second
    velocity: f32,

    /// State machine
    state: ScrollState,
    /// Last interaction time
    last_interaction_time: Instant,
    /// Manual scroll lower bound (top of document)
    min_scroll_y: f32,
    /// Manual scroll upper bound (bottom of document)
    max_scroll_y: f32,
    /// Spring for snapping with damper/stiffness
    snap_spring: Spring,
}

impl ScrollPhysics {
    /// Create new scroll physics
    pub fn new() -> Self {
        Self {
            scroll_y: 0.0,
            velocity: 0.0,
            state: ScrollState::AutoPlay,
            last_interaction_time: Instant::now(),
            min_scroll_y: 0.0,
            max_scroll_y: 0.0,
            snap_spring: Spring::with_params(0.0, 0.8, 10.0),
        }
    }

    /// 更新物理模拟
    ///
    /// @param dt 距上次更新的时间（秒）
    pub fn update(&mut self, dt: f32, _is_hovering: bool) {
        self.snap_spring.update(dt as f64);

        match self.state {
            ScrollState::AutoPlay => {
                if self.snap_spring.target().abs() > 0.1 {
                    self.snap_spring.set_target(0.0);
                }
                self.scroll_y = self.snap_spring.position() as f32;
            }
            ScrollState::Idle => {
                self.velocity = 0.0;
            }
        }
    }

    /// Directly modify scroll position (for mouse wheel)
    pub fn scroll_by(&mut self, delta: f32) {
        self.state = ScrollState::Idle;
        self.last_interaction_time = Instant::now();
        self.velocity = 0.0;
        self.scroll_y += delta;
        self.clamp_position();
        self.snap_spring.set_position(self.scroll_y as f64);
        self.snap_spring.set_velocity(0.0);
        self.snap_spring.set_target(self.scroll_y as f64);
    }

    /// Start auto-play mode
    pub fn start_auto_play(&mut self) {
        if self.state != ScrollState::AutoPlay {
            self.snap_spring.set_position(self.scroll_y as f64);
            self.snap_spring.set_velocity(self.velocity as f64);
            self.snap_spring.set_target(0.0);
        }
        self.state = ScrollState::AutoPlay;
    }

    /// Set manual scroll bounds.
    pub fn set_scroll_bounds(&mut self, min_scroll_y: f32, max_scroll_y: f32) {
        self.min_scroll_y = min_scroll_y;
        self.max_scroll_y = max_scroll_y;
    }

    /// Get current scroll position
    pub fn position(&self) -> f32 {
        self.scroll_y
    }

    /// Get current state
    pub fn state(&self) -> ScrollState {
        self.state
    }

    /// 获取距上次交互的时间
    pub fn time_since_interaction(&self) -> f32 {
        self.last_interaction_time.elapsed().as_secs_f32()
    }

    /// Clamp current manual scroll position to the latest bounds.
    pub fn clamp_position(&mut self) -> f32 {
        let min_bound = self.min_scroll_y.min(self.max_scroll_y);
        let max_bound = self.min_scroll_y.max(self.max_scroll_y);
        self.scroll_y = self.scroll_y.clamp(min_bound, max_bound);
        self.scroll_y
    }

    /// Reset manual scrolling and return to auto-follow.
    pub fn reset_manual_scroll(&mut self) {
        self.scroll_y = 0.0;
        self.velocity = 0.0;
        self.last_interaction_time = Instant::now();
        self.snap_spring.set_position(0.0);
        self.snap_spring.set_velocity(0.0);
        self.snap_spring.set_target(0.0);
        self.state = ScrollState::AutoPlay;
    }
}

impl Default for ScrollPhysics {
    fn default() -> Self {
        Self::new()
    }
}
