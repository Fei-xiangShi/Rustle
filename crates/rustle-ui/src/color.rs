//! Perceptual color authoring. Iced values are encoded sRGB boundary values.
use iced::Color;
use std::cell::RefCell;
use std::collections::HashMap;
pub mod artwork;

const NEUTRAL_EPSILON: f32 = 0.0001;

/// OKLCH with lightness/alpha in 0..=1 and hue in degrees (not Iced radians).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklcha {
    l: f32,
    c: f32,
    h: f32,
    a: f32,
}

impl Oklcha {
    pub const fn new(l: f32, c: f32, h: f32, a: f32) -> Self {
        assert!(l.is_finite() && l >= 0.0 && l <= 1.0);
        assert!(c.is_finite() && c >= 0.0);
        assert!(h.is_finite());
        assert!(a.is_finite() && a >= 0.0 && a <= 1.0);
        Self { l, c, h, a }
    }

    pub fn try_new(l: f32, c: f32, h: f32, a: f32) -> Option<Self> {
        (l.is_finite()
            && (0.0..=1.0).contains(&l)
            && c.is_finite()
            && c >= 0.0
            && h.is_finite()
            && a.is_finite()
            && (0.0..=1.0).contains(&a))
        .then(|| Self::new(l, c, h.rem_euclid(360.0), a))
    }

    pub fn from_color(color: Color) -> Self {
        let value = color.into_oklch();
        let c = if value.c < NEUTRAL_EPSILON {
            0.0
        } else {
            value.c
        };
        Self::new(
            value.l.clamp(0.0, 1.0),
            c,
            if c == 0.0 { 0.0 } else { value.h.to_degrees() },
            color.a,
        )
    }

    pub const fn lightness(self) -> f32 {
        self.l
    }
    pub const fn chroma(self) -> f32 {
        self.c
    }
    pub fn hue(self) -> f32 {
        self.h.rem_euclid(360.0)
    }
    pub const fn alpha(self) -> f32 {
        self.a
    }
    pub fn with_lightness(self, l: f32) -> Self {
        Self::new(l, self.c, self.h, self.a)
    }
    pub fn with_chroma(self, c: f32) -> Self {
        Self::new(self.l, c, self.h, self.a)
    }
    pub fn with_alpha(self, a: f32) -> Self {
        Self::new(self.l, self.c, self.h, a)
    }

    pub fn lab(self) -> [f32; 3] {
        let h = self.hue().to_radians();
        [self.l, self.c * h.cos(), self.c * h.sin()]
    }

    pub fn from_lab([l, a, b]: [f32; 3], alpha: f32) -> Self {
        let c = a.hypot(b);
        Self::new(
            l.clamp(0.0, 1.0),
            c,
            if c < NEUTRAL_EPSILON {
                0.0
            } else {
                b.atan2(a).to_degrees()
            },
            alpha,
        )
    }

    /// Preserve L/h by reducing C before conversion; Iced itself clips channels.
    pub fn to_color(self) -> Color {
        if self.l == 0.0 {
            return Color::BLACK.scale_alpha(self.a);
        }
        if self.l == 1.0 {
            return Color::WHITE.scale_alpha(self.a);
        }
        let mut rgb = linear_rgb(self.lab());
        if !in_gamut(rgb) {
            let mut low = 0.0;
            // A finite practical bound also handles extremely large valid chroma.
            let mut high = self.c.min(1.0);
            for _ in 0..20 {
                let c = (low + high) * 0.5;
                let candidate = linear_rgb(self.with_chroma(c).lab());
                if in_gamut(candidate) {
                    low = c;
                } else {
                    high = c;
                }
            }
            rgb = linear_rgb(self.with_chroma(low).lab());
        }
        Color::from_linear_rgba(
            rgb[0].clamp(0.0, 1.0),
            rgb[1].clamp(0.0, 1.0),
            rgb[2].clamp(0.0, 1.0),
            self.a,
        )
    }

    /// Hue-directed interpolation; neutral endpoints borrow the chromatic hue.
    pub fn mix_hue(self, other: Self, t: f32) -> Self {
        let t = progress(t);
        if t == 0.0 {
            return self;
        }
        if t == 1.0 {
            return other;
        }
        let h0 = if self.c < NEUTRAL_EPSILON {
            other.hue()
        } else {
            self.hue()
        };
        let h1 = if other.c < NEUTRAL_EPSILON {
            h0
        } else {
            other.hue()
        };
        let delta = (h1 - h0).rem_euclid(360.0);
        let delta = if delta > 180.0 { delta - 360.0 } else { delta };
        let alpha = self.a + (other.a - self.a) * t;
        let weight = if alpha > 0.0 { other.a * t / alpha } else { t };
        Self::new(
            self.l + (other.l - self.l) * weight,
            self.c + (other.c - self.c) * weight,
            h0 + delta * t,
            alpha,
        )
    }
}

/// Unclipped Oklab -> linear sRGB, localized here for gamut detection.
fn linear_rgb([l, a, b]: [f32; 3]) -> [f32; 3] {
    let ll = (l + 0.39633778 * a + 0.21580376 * b).powi(3);
    let m = (l - 0.105561346 * a - 0.06385417 * b).powi(3);
    let s = (l - 0.08948418 * a - 1.2914855 * b).powi(3);
    [
        4.0767417 * ll - 3.3077116 * m + 0.23096994 * s,
        -1.268438 * ll + 2.6097574 * m - 0.34131938 * s,
        -0.0041960863 * ll - 0.7034186 * m + 1.7076147 * s,
    ]
}

fn in_gamut(rgb: [f32; 3]) -> bool {
    rgb.into_iter()
        .all(|v| v.is_finite() && (-0.000001..=1.000001).contains(&v))
}

fn progress(t: f32) -> f32 {
    if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

thread_local! {
    // RGB only: alpha-only animation reuses endpoints. Bounded per UI thread.
    static LAB_CACHE: RefCell<HashMap<[u32; 3], [f32; 3]>> = RefCell::new(HashMap::new());
}

fn cached_lab(color: Color) -> [f32; 3] {
    LAB_CACHE.with_borrow_mut(|cache| {
        let key = [color.r.to_bits(), color.g.to_bits(), color.b.to_bits()];
        if let Some(lab) = cache.get(&key) {
            return *lab;
        }
        let lab = Oklcha::from_color(color).lab();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(key, lab);
        lab
    })
}

/// Prepared Cartesian perceptual transition with alpha-aware interpolation.
#[derive(Debug, Clone, Copy)]
pub struct ColorTransition {
    from: Color,
    to: Color,
    from_lab: [f32; 3],
    to_lab: [f32; 3],
}

impl ColorTransition {
    pub fn new(from: Color, to: Color) -> Self {
        Self {
            from,
            to,
            from_lab: cached_lab(from),
            to_lab: cached_lab(to),
        }
    }

    pub fn sample(self, t: f32) -> Color {
        let t = progress(t);
        if t == 0.0 || self.from == self.to {
            return self.from;
        }
        if t == 1.0 {
            return self.to;
        }
        let alpha = self.from.a + (self.to.a - self.from.a) * t;
        if self.from.r == self.to.r && self.from.g == self.to.g && self.from.b == self.to.b {
            return with_alpha(self.from, alpha);
        }
        if alpha <= 0.0 {
            return Color::TRANSPARENT;
        }
        let weight = self.to.a * t / alpha;
        let lab = std::array::from_fn(|i| {
            self.from_lab[i] + (self.to_lab[i] - self.from_lab[i]) * weight
        });
        lab_color(lab, alpha)
    }
}

pub fn mix(from: Color, to: Color, t: f32) -> Color {
    ColorTransition::new(from, to).sample(t)
}

/// Cartesian working-space output; avoid a polar round trip for in-gamut mixes.
pub fn lab_color(lab: [f32; 3], alpha: f32) -> Color {
    let rgb = linear_rgb(lab);
    if in_gamut(rgb) {
        Color::from_linear_rgba(
            rgb[0].clamp(0.0, 1.0),
            rgb[1].clamp(0.0, 1.0),
            rgb[2].clamp(0.0, 1.0),
            alpha,
        )
    } else {
        Oklcha::from_lab(lab, alpha).to_color()
    }
}

/// Set, rather than multiply, opacity at a renderer boundary.
pub fn with_alpha(color: Color, alpha: f32) -> Color {
    Color {
        a: progress(alpha),
        ..color
    }
}

/// Encoded-sRGB compositing, matching the project's enabled `web-colors` path.
pub fn composite(foreground: Color, background: Color) -> Color {
    let a = foreground.a + background.a * (1.0 - foreground.a);
    if a == 0.0 {
        return Color::TRANSPARENT;
    }
    let channel = |f: f32, b: f32| (f * foreground.a + b * background.a * (1.0 - foreground.a)) / a;
    Color::from_rgba(
        channel(foreground.r, background.r),
        channel(foreground.g, background.g),
        channel(foreground.b, background.b),
        a,
    )
}

/// Sample a perceptual path into Iced's bounded native gradient (an approximation).
pub fn gradient(angle: impl Into<iced::Radians>, from: Color, to: Color) -> iced::gradient::Linear {
    let transition = ColorTransition::new(from, to);
    (0..8).fold(iced::gradient::Linear::new(angle), |gradient, i| {
        let t = i as f32 / 7.0;
        gradient.add_stop(t, transition.sample(t))
    })
}

/// Eight ordered samples with the authored middle stop retained exactly.
pub fn gradient_three(
    angle: impl Into<iced::Radians>,
    colors: [Color; 3],
    middle: f32,
) -> iced::gradient::Linear {
    let middle = middle.clamp(0.01, 0.99);
    let first = ColorTransition::new(colors[0], colors[1]);
    let second = ColorTransition::new(colors[1], colors[2]);
    (0..8).fold(iced::gradient::Linear::new(angle), |gradient, i| {
        let (offset, color) = if i <= 3 {
            let t = i as f32 / 3.0;
            (middle * t, first.sample(t))
        } else {
            let t = (i - 3) as f32 / 4.0;
            (middle + (1.0 - middle) * t, second.sample(t))
        };
        gradient.add_stop(offset, color)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_vectors_and_gamut_mapping() {
        let red = Oklcha::new(0.627955, 0.257683, 29.2339, 1.0).to_color();
        assert!(red.r > 0.999 && red.g < 0.001 && red.b < 0.001);
        for rgb in [Color::BLACK, Color::WHITE, Color::from_rgb(0.2, 0.4, 0.7)] {
            let mapped = Oklcha::from_color(rgb).to_color();
            assert!((mapped.r - rgb.r).abs() < 0.00002);
            assert!((mapped.g - rgb.g).abs() < 0.00002);
            assert!((mapped.b - rgb.b).abs() < 0.00002);
        }
        let source = Oklcha::new(0.7, 0.5, 185.0, 0.4);
        let mapped = Oklcha::from_color(source.to_color());
        assert!(mapped.chroma() < source.chroma());
        assert!((mapped.lightness() - 0.7).abs() < 0.0001);
        assert!((mapped.hue() - 185.0).abs() < 0.01);
        assert_eq!(mapped.alpha(), 0.4);
        assert!(Oklcha::try_new(f32::NAN, 0.2, 0.0, 1.0).is_none());
        assert!(Oklcha::try_new(0.5, -0.1, 0.0, 1.0).is_none());
    }
    #[test]
    fn perceptual_and_transparent_transitions() {
        let mid = mix(Color::BLACK, Color::WHITE, 0.5);
        assert!((mid.r - 0.388573).abs() < 0.0001);
        let color = Oklcha::new(0.7, 0.12, 185.0, 1.0).to_color();
        let fade = mix(Color::TRANSPARENT, color, 0.5);
        assert!((fade.r - color.r).abs() < 0.00002 && (fade.g - color.g).abs() < 0.00002);
        assert_eq!(fade.a, 0.5);
        assert_eq!(mix(Color::TRANSPARENT, color, 0.0), Color::TRANSPARENT);
        assert_eq!(mix(Color::TRANSPARENT, color, 1.0), color);
        let c = Oklcha::new(0.6, 0.1, 359.0, 1.0);
        assert!(c.mix_hue(Oklcha::new(0.6, 0.1, 1.0, 1.0), 0.5).hue() < 0.001);
        assert_eq!(
            Oklcha::new(0.5, 0.1, 0.0, 1.0)
                .mix_hue(Oklcha::new(0.5, 0.1, 180.0, 1.0), 0.5)
                .hue(),
            90.0
        );
        assert_eq!(Oklcha::new(0.5, 0.0, 0.0, 1.0).mix_hue(c, 0.5).hue(), 359.0);
    }
}
