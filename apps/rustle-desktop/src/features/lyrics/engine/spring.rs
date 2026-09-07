//! Spring physics system for smooth animations
//!
//! Uses time-based analytical solutions rather than frame-by-frame integration.
//!
//! ## Spring Parameters (defaults)
//!
//! | Usage | mass | damping | stiffness |
//! |-------|------|---------|-----------|
//! | Position Y | 0.9 | 15 | 90 |
//! | Scale (normal) | 2 | 25 | 100 |
//! | Scale (background) | 1 | 20 | 50 |
//!
//! ## Key Algorithm
//!
//! Overdamped condition: `soft || 1.0 <= damping / (2.0 * sqrt(stiffness * mass))`
//!
//! ### Overdamped formula
//! ```text
//! angular_frequency = -sqrt(stiffness / mass)
//! leftover = -angular_frequency * delta - velocity
//! position(t) = to - (delta + t * leftover) * e^(t * angular_frequency)
//! ```
//!
//! ### Underdamped formula
//! ```text
//! damping_frequency = sqrt(4 * mass * stiffness - damping^2)
//! leftover = (damping * delta - 2 * mass * velocity) / damping_frequency
//! dfm = 0.5 * damping_frequency / mass
//! dm = -0.5 * damping / mass
//! position(t) = to - (cos(t * dfm) * delta + sin(t * dfm) * leftover) * e^(t * dm)
//! ```

use std::f64::consts::E;
use std::sync::Arc;

pub type Num = f64;

/// Numerical derivative step size
const H: Num = 0.001;

/// Spring parameters for physics simulation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringParams {
    pub mass: Num,
    pub damping: Num,
    pub stiffness: Num,
    pub soft: bool,
}

impl SpringParams {
    /// Position Y spring (default)
    pub const POS_Y: Self = Self {
        mass: 0.9,
        damping: 15.0,
        stiffness: 90.0,
        soft: false,
    };

    /// Scale spring for normal lines (default)
    pub const SCALE: Self = Self {
        mass: 2.0,
        damping: 25.0,
        stiffness: 100.0,
        soft: false,
    };

    /// Scale spring for background lines (default)
    pub const SCALE_BG: Self = Self {
        mass: 1.0,
        damping: 20.0,
        stiffness: 50.0,
        soft: false,
    };

    /// Check if overdamped: 1.0 <= damping / (2.0 * sqrt(stiffness * mass))
    pub fn is_overdamped(&self) -> bool {
        self.soft || 1.0 <= self.damping / (2.0 * (self.stiffness * self.mass).sqrt())
    }
}

impl Default for SpringParams {
    fn default() -> Self {
        Self {
            mass: 1.0,
            damping: 10.0,
            stiffness: 100.0,
            soft: false,
        }
    }
}

/// Solver function type
type SolverFn = Arc<dyn Fn(Num) -> Num + Send + Sync>;

/// Create solver function for spring animation
fn solve_spring(from: Num, velocity: Num, to: Num, delay: Num, params: &SpringParams) -> SolverFn {
    let stiffness = params.stiffness;
    let damping = params.damping;
    let mass = params.mass;
    let delta = to - from;

    if params.is_overdamped() {
        // Overdamped
        let angular_frequency = -(stiffness / mass).sqrt();
        let leftover = -angular_frequency * delta - velocity;

        Arc::new(move |t: Num| {
            let t = t - delay;
            if t < 0.0 {
                return from;
            }
            to - (delta + t * leftover) * E.powf(t * angular_frequency)
        })
    } else {
        // Underdamped
        let damping_frequency = (4.0 * mass * stiffness - damping.powi(2)).sqrt();
        let leftover = (damping * delta - 2.0 * mass * velocity) / damping_frequency;
        let dfm = 0.5 * damping_frequency / mass;
        let dm = -0.5 * damping / mass;

        Arc::new(move |t: Num| {
            let t = t - delay;
            if t < 0.0 {
                return from;
            }
            to - ((t * dfm).cos() * delta + (t * dfm).sin() * leftover) * E.powf(t * dm)
        })
    }
}

/// Create velocity function from position function (numerical derivative)
fn get_velocity(f: SolverFn) -> SolverFn {
    Arc::new(move |t: Num| (f(t + H) - f(t - H)) / (2.0 * H))
}

/// Queued parameter update
#[derive(Debug, Clone)]
struct QueuedParams {
    params: SpringParams,
    time: Num,
}

/// Queued position update
#[derive(Debug, Clone, Copy)]
struct QueuedPosition {
    position: Num,
    time: Num,
}

/// Spring animation with analytical solution
///
/// Key features:
/// - Uses analytical solution, not numerical integration
/// - Caches solver functions for performance
/// - Supports delayed target changes for staggered animations
pub struct Spring {
    current_position: Num,
    target_position: Num,
    current_time: Num,
    params: SpringParams,
    /// Cached position solver
    current_solver: SolverFn,
    /// Cached velocity function (first derivative)
    get_v: SolverFn,
    /// Cached acceleration function (second derivative)
    get_v2: SolverFn,
    /// Queued parameter update
    queue_params: Option<QueuedParams>,
    /// Queued position update
    queue_position: Option<QueuedPosition>,
}

impl std::fmt::Debug for Spring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spring")
            .field("current_position", &self.current_position)
            .field("target_position", &self.target_position)
            .field("current_time", &self.current_time)
            .field("params", &self.params)
            .finish()
    }
}

impl Spring {
    /// Create spring at initial position
    pub fn new(current_position: Num) -> Self {
        let target = current_position;
        Self {
            current_position,
            target_position: target,
            current_time: 0.0,
            params: SpringParams::default(),
            current_solver: Arc::new(move |_| target),
            get_v: Arc::new(|_| 0.0),
            get_v2: Arc::new(|_| 0.0),
            queue_params: None,
            queue_position: None,
        }
    }

    /// Create spring with custom params
    pub fn from_params(current_position: Num, params: SpringParams) -> Self {
        let mut spring = Self::new(current_position);
        spring.params = params;
        spring
    }

    /// Create spring with damper/speed (legacy compatibility)
    pub fn with_params(start_position: Num, damper: Num, speed: Num) -> Self {
        let mass = 1.0;
        let stiffness = speed * speed * mass;
        let damping = damper * 2.0 * (stiffness * mass).sqrt();
        Self::from_params(
            start_position,
            SpringParams {
                mass,
                damping,
                stiffness,
                soft: false,
            },
        )
    }

    /// Reset solver with current state
    fn reset_solver(&mut self) {
        let cur_v = (self.get_v)(self.current_time);
        self.current_time = 0.0;
        self.current_solver = solve_spring(
            self.current_position,
            cur_v,
            self.target_position,
            0.0,
            &self.params,
        );
        self.get_v = get_velocity(Arc::clone(&self.current_solver));
        self.get_v2 = get_velocity(Arc::clone(&self.get_v));
    }

    /// Check if spring has arrived at target
    pub fn arrived(&self) -> bool {
        (self.target_position - self.current_position).abs() < 0.01
            && (self.get_v)(self.current_time) < 0.01
            && (self.get_v2)(self.current_time) < 0.01
            && self.queue_params.is_none()
            && self.queue_position.is_none()
    }

    /// Set position immediately without animation
    pub fn set_position(&mut self, target_position: Num) {
        self.target_position = target_position;
        self.current_position = target_position;
        let target = target_position;
        self.current_solver = Arc::new(move |_| target);
        self.get_v = Arc::new(|_| 0.0);
        self.get_v2 = Arc::new(|_| 0.0);
    }

    /// 更新弹簧状态 - 每帧调用
    ///
    /// @param delta 距上次更新的时间（秒）
    pub fn update(&mut self, delta: Num) {
        self.current_time += delta;
        self.current_position = (self.current_solver)(self.current_time);

        // Process queued params
        if let Some(mut queued) = self.queue_params.take() {
            queued.time -= delta;
            if queued.time <= 0.0 {
                self.update_params(queued.params);
            } else {
                self.queue_params = Some(queued);
            }
        }

        // Process queued position
        if let Some(mut queued) = self.queue_position.take() {
            queued.time -= delta;
            if queued.time <= 0.0 {
                self.set_target_position(queued.position);
            } else {
                self.queue_position = Some(queued);
            }
        }

        // Snap to target if arrived
        if self.arrived() {
            self.set_position(self.target_position);
        }
    }

    /// Update spring parameters
    pub fn update_params(&mut self, params: SpringParams) {
        self.update_params_with_delay(params, 0.0);
    }

    /// Update spring parameters with delay
    pub fn update_params_with_delay(&mut self, params: SpringParams, delay: Num) {
        if delay > 0.0 {
            self.queue_params = Some(QueuedParams {
                params,
                time: delay,
            });
        } else {
            self.queue_params = None;
            self.params = params;
            self.reset_solver();
        }
    }

    /// Set target position
    pub fn set_target_position(&mut self, target_position: Num) {
        self.set_target_position_with_delay(target_position, 0.0);
    }

    /// Set target position with delay
    pub fn set_target_position_with_delay(&mut self, target_position: Num, delay: Num) {
        if delay > 0.0 {
            self.queue_position = Some(QueuedPosition {
                position: target_position,
                time: delay,
            });
        } else {
            self.queue_position = None;
            self.target_position = target_position;
            self.reset_solver();
        }
    }

    pub fn position(&self) -> Num {
        self.current_position
    }

    /// Alias for set_target_position
    pub fn set_target(&mut self, value: Num) {
        self.set_target_position(value);
    }

    /// Alias for set_target_position_with_delay
    pub fn set_target_with_delay(&mut self, value: Num, delay: Num) {
        self.set_target_position_with_delay(value, delay);
    }

    /// Get target position
    pub fn target(&self) -> Num {
        self.target_position
    }

    /// Set velocity (resets solver)
    pub fn set_velocity(&mut self, value: Num) {
        // Store current position, set new velocity, reset solver
        let pos = self.current_position;
        self.current_time = 0.0;
        self.current_solver = solve_spring(pos, value, self.target_position, 0.0, &self.params);
        self.get_v = get_velocity(Arc::clone(&self.current_solver));
        self.get_v2 = get_velocity(Arc::clone(&self.get_v));
    }
}

impl Clone for Spring {
    fn clone(&self) -> Self {
        Self {
            current_position: self.current_position,
            target_position: self.target_position,
            current_time: self.current_time,
            params: self.params,
            current_solver: Arc::clone(&self.current_solver),
            get_v: Arc::clone(&self.get_v),
            get_v2: Arc::clone(&self.get_v2),
            queue_params: self.queue_params.clone(),
            queue_position: self.queue_position,
        }
    }
}

impl From<f64> for Spring {
    fn from(p: f64) -> Self {
        Self::new(p)
    }
}

impl Default for Spring {
    fn default() -> Self {
        Self::new(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spring_basic() {
        let mut spring = Spring::new(0.0);
        spring.set_target(100.0);

        for _ in 0..10 {
            spring.update(0.01);
        }

        let pos = spring.position();
        assert!(pos > 0.0, "Spring should move from 0");
        assert!(pos < 100.0, "Spring should not reach target yet");
    }

    #[test]
    fn test_spring_params() {
        let spring = Spring::from_params(0.0, SpringParams::POS_Y);
        assert_eq!(spring.params.mass, 0.9);
        assert_eq!(spring.params.damping, 15.0);
        assert_eq!(spring.params.stiffness, 90.0);
    }

    #[test]
    fn test_spring_delay() {
        let mut spring = Spring::new(0.0);
        spring.set_target_with_delay(100.0, 0.1);

        // Before delay
        spring.update(0.05);
        assert!(
            spring.position().abs() < 0.01,
            "Should not move before delay"
        );

        // After delay
        spring.update(0.06);
        for _ in 0..5 {
            spring.update(0.02);
        }
        assert!(spring.position() > 0.0, "Should move after delay");
    }

    #[test]
    fn test_overdamped() {
        // High damping = overdamped
        let params = SpringParams {
            mass: 1.0,
            damping: 100.0,
            stiffness: 100.0,
            soft: false,
        };
        assert!(params.is_overdamped());

        // Low damping = underdamped
        let params2 = SpringParams {
            mass: 1.0,
            damping: 5.0,
            stiffness: 100.0,
            soft: false,
        };
        assert!(!params2.is_overdamped());
    }

    #[test]
    fn test_default_params() {
        // Verify default params
        assert_eq!(SpringParams::POS_Y.mass, 0.9);
        assert_eq!(SpringParams::POS_Y.damping, 15.0);
        assert_eq!(SpringParams::POS_Y.stiffness, 90.0);

        assert_eq!(SpringParams::SCALE.mass, 2.0);
        assert_eq!(SpringParams::SCALE.damping, 25.0);
        assert_eq!(SpringParams::SCALE.stiffness, 100.0);

        assert_eq!(SpringParams::SCALE_BG.mass, 1.0);
        assert_eq!(SpringParams::SCALE_BG.damping, 20.0);
        assert_eq!(SpringParams::SCALE_BG.stiffness, 50.0);
    }
}
