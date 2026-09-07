//! InterludeDots component for lyric player
//!
//! Shows animated dots during instrumental interludes.

const DOT_PADDING_Y_EM: f32 = 0.8;
const DOT_TRAILING_MARGIN_EM: f32 = 4.0 / 60.48;

/// Easing function: easeInOutBack
fn ease_in_out_back(x: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C2: f32 = C1 * 1.525;

    if x < 0.5 {
        ((2.0 * x).powi(2) * ((C2 + 1.0) * 2.0 * x - C2)) / 2.0
    } else {
        ((2.0 * x - 2.0).powi(2) * ((C2 + 1.0) * (x * 2.0 - 2.0) + C2) + 2.0) / 2.0
    }
}

/// Easing function: easeOutExpo
fn ease_out_expo(x: f32) -> f32 {
    if x == 1.0 {
        1.0
    } else {
        1.0 - 2.0_f32.powf(-10.0 * x)
    }
}

fn clamp(min: f32, cur: f32, max: f32) -> f32 {
    cur.max(min).min(max)
}

pub fn dot_size(font_size: f32, _viewport_height: f32) -> f32 {
    font_size * 0.5
}

pub fn dot_padding_x(font_size: f32) -> f32 {
    font_size * 0.75
}

pub fn dot_padding_y(font_size: f32) -> f32 {
    font_size * DOT_PADDING_Y_EM
}

pub fn dot_spacing(font_size: f32) -> f32 {
    font_size * (0.25 + DOT_TRAILING_MARGIN_EM)
}

pub fn dot_margin(font_size: f32) -> f32 {
    font_size * 0.4
}

fn dot_gap(font_size: f32) -> f32 {
    font_size * 0.25
}

fn dot_trailing_margin(font_size: f32) -> f32 {
    font_size * DOT_TRAILING_MARGIN_EM
}

pub fn dot_trailing_margin_px(font_size: f32) -> f32 {
    dot_trailing_margin(font_size)
}

pub fn dot_container_width(font_size: f32, _viewport_width: f32, viewport_height: f32) -> f32 {
    let dot_size = dot_size(font_size, viewport_height);
    let pad_x = dot_padding_x(font_size);
    let gap = dot_gap(font_size);
    let trailing_margin = dot_trailing_margin(font_size);

    pad_x * 2.0 + dot_size * 3.0 + gap * 2.0 + trailing_margin * 3.0
}

pub fn dot_container_height(font_size: f32, _viewport_width: f32, viewport_height: f32) -> f32 {
    let dot_size = dot_size(font_size, viewport_height);
    let pad_y = dot_padding_y(font_size);
    dot_size + pad_y * 2.0
}

pub fn dot_total_height(font_size: f32, viewport_width: f32, viewport_height: f32) -> f32 {
    let dot_margin = dot_margin(font_size);
    dot_container_height(font_size, viewport_width, viewport_height) + dot_margin * 2.0
}

/// State of an interlude (start_time_ms, end_time_ms)
pub type InterludeRange = (f32, f32);

/// InterludeDots animation state
///
/// Three dots that appear during instrumental interludes with breathing animation
#[derive(Debug, Clone)]
pub struct InterludeDots {
    /// Position X
    pub left: f32,
    /// Position Y
    pub top: f32,
    /// Whether the dots are currently playing animation
    pub playing: bool,
    /// Current interlude range (start_ms, end_ms)
    current_interlude: Option<InterludeRange>,
    /// Current time in milliseconds
    current_time: f32,
    /// Target breathe duration in ms
    target_breathe_duration: f32,
    /// Dot opacities (0-1)
    pub dot_opacities: [f32; 3],
    /// Overall scale
    pub scale: f32,
    /// Whether dots are enabled (visible)
    pub enabled: bool,
}

impl Default for InterludeDots {
    fn default() -> Self {
        Self::new()
    }
}

impl InterludeDots {
    pub fn new() -> Self {
        Self {
            left: 0.0,
            top: 0.0,
            playing: true,
            current_interlude: None,
            current_time: 0.0,
            target_breathe_duration: 1500.0,
            dot_opacities: [0.0, 0.0, 0.0],
            scale: 0.0,
            enabled: false,
        }
    }

    /// Set position
    pub fn set_transform(&mut self, left: f32, top: f32) {
        self.left = left;
        self.top = top;
    }

    /// Set the current interlude range
    /// Only resets animation state when the range actually changes,
    /// to avoid resetting the animation every frame.
    pub fn set_interlude(&mut self, interlude: Option<InterludeRange>) {
        // Only reset if the interlude range actually changed
        let changed = match (self.current_interlude, interlude) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some((s1, e1)), Some((s2, e2))) => (s1 - s2).abs() > 0.5 || (e1 - e2).abs() > 0.5,
        };

        if !changed {
            return;
        }

        self.current_interlude = interlude;
        self.current_time = interlude.map(|(start, _)| start).unwrap_or(0.0);
        self.enabled = interlude.is_some();

        if interlude.is_none() {
            self.scale = 0.0;
            self.dot_opacities = [0.0, 0.0, 0.0];
        }
    }

    /// Sync the internal time to real playback time (used during seeks)
    pub fn sync_time(&mut self, time_ms: f32) {
        self.current_time = time_ms;
    }

    /// Pause animation
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Resume animation
    pub fn resume(&mut self) {
        self.playing = true;
    }

    /// Update animation state
    /// delta: time elapsed in seconds
    pub fn update(&mut self, delta: f32) {
        if !self.playing {
            return;
        }

        // Convert delta to milliseconds
        self.current_time += delta * 1000.0;

        if let Some((start, end)) = self.current_interlude {
            let interlude_duration = end - start;
            let current_duration = self.current_time - start;

            if current_duration <= interlude_duration && current_duration >= 0.0 {
                let breathe_duration =
                    interlude_duration / (interlude_duration / self.target_breathe_duration).ceil();

                let mut scale = 1.0_f32;
                let mut global_opacity = 1.0_f32;

                // Breathing animation
                scale *= (1.5 * std::f32::consts::PI - (current_duration / breathe_duration) * 2.0)
                    .sin()
                    / 20.0
                    + 1.0;

                // Fade in at start
                if current_duration < 2000.0 {
                    scale *= ease_out_expo(current_duration / 2000.0);
                }

                // Opacity fade in
                if current_duration < 500.0 {
                    global_opacity = 0.0;
                } else if current_duration < 1000.0 {
                    global_opacity *= (current_duration - 500.0) / 500.0;
                }

                // Fade out at end
                if interlude_duration - current_duration < 750.0 {
                    scale *= 1.0
                        - ease_in_out_back(
                            (750.0 - (interlude_duration - current_duration)) / 750.0 / 2.0,
                        );
                }
                if interlude_duration - current_duration < 375.0 {
                    global_opacity *=
                        clamp(0.0, (interlude_duration - current_duration) / 375.0, 1.0);
                }

                let dots_duration = (interlude_duration - 750.0).max(0.0);

                scale = scale.max(0.0) * 0.7;
                self.scale = scale;

                // Calculate individual dot opacities (sequential lighting)
                self.dot_opacities[0] =
                    clamp(0.25, ((current_duration * 3.0) / dots_duration) * 0.75, 1.0)
                        * global_opacity;

                self.dot_opacities[1] = clamp(
                    0.25,
                    (((current_duration - dots_duration / 3.0) * 3.0) / dots_duration) * 0.75,
                    1.0,
                ) * global_opacity;

                self.dot_opacities[2] = clamp(
                    0.25,
                    (((current_duration - (dots_duration / 3.0) * 2.0) * 3.0) / dots_duration)
                        * 0.75,
                    1.0,
                ) * global_opacity;
            } else {
                // Outside interlude range
                self.scale = 0.0;
                self.dot_opacities = [0.0, 0.0, 0.0];
            }
        } else {
            self.scale = 0.0;
            self.dot_opacities = [0.0, 0.0, 0.0];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interlude_dots_animation() {
        let mut dots = InterludeDots::new();

        // Set a 5 second interlude
        dots.set_interlude(Some((0.0, 5000.0)));
        assert!(dots.enabled);

        // Update for 1 second (should be fading in)
        dots.update(1.0);
        assert!(dots.scale > 0.0);
        assert!(dots.dot_opacities[0] > 0.0);
    }

    #[test]
    fn test_interlude_dots_disabled() {
        let mut dots = InterludeDots::new();

        dots.set_interlude(None);
        assert!(!dots.enabled);

        dots.update(1.0);
        assert_eq!(dots.scale, 0.0);
    }
}
