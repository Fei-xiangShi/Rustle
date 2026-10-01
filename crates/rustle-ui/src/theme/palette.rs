//! Cached semantic palettes authored in OKLCH. Hue 185 is Rustle's cyan-teal brand.
use crate::color::{Oklcha, mix};
use iced::{Color, Theme, theme::palette as iced_palette};
use std::sync::LazyLock;

#[derive(Debug)]
pub struct Palette {
    pub canvas: Color,
    pub sidebar: Color,
    pub surface: Color,
    pub raised: Color,
    pub hover: Color,
    pub border: Color,
    pub control_border: Color,
    pub text: Color,
    pub secondary: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub on_accent: Color,
    pub accent_subtle: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub danger_hover: Color,
    pub info: Color,
    pub dark: bool,
}

fn color(l: f32, c: f32, h: f32) -> Color {
    Oklcha::new(l, c, h, 1.0).to_color()
}

impl Palette {
    fn new(dark: bool) -> Self {
        let neutral = |l| color(l, 0.006, 185.0);
        let choose = |d, l| if dark { d } else { l };
        let accent = color(choose(0.76, 0.46), choose(0.14, 0.09), 185.0);
        Self {
            canvas: neutral(choose(0.105, 0.99)),
            sidebar: neutral(choose(0.16, 0.975)),
            surface: neutral(choose(0.20, 0.95)),
            raised: neutral(choose(0.25, 0.99)),
            hover: neutral(choose(0.28, 0.91)),
            border: neutral(choose(0.38, 0.82)),
            control_border: neutral(choose(0.56, 0.59)),
            text: neutral(choose(0.96, 0.20)),
            secondary: neutral(choose(0.80, 0.37)),
            muted: neutral(choose(0.71, 0.47)),
            accent,
            accent_hover: color(choose(0.70, 0.40), choose(0.14, 0.08), 185.0),
            accent_pressed: color(choose(0.65, 0.35), choose(0.12, 0.07), 185.0),
            on_accent: on_color(accent),
            accent_subtle: color(choose(0.28, 0.92), choose(0.035, 0.03), 185.0),
            success: color(choose(0.76, 0.44), 0.13, 150.0),
            warning: color(choose(0.82, 0.45), 0.13, 85.0),
            danger: color(choose(0.72, 0.49), 0.18, 25.0),
            danger_hover: color(choose(0.78, 0.43), 0.17, 25.0),
            info: color(choose(0.76, 0.48), 0.12, 240.0),
            dark,
        }
    }

    fn iced(&self) -> iced_palette::Palette {
        use iced_palette::{Background, Pair, Swatch};
        let pair = |color, text| Pair { color, text };
        let background = |color| pair(color, self.text);
        let swatch = |base: Color| Swatch {
            base: pair(base, on_color(base)),
            weak: pair(mix(self.surface, base, 0.14), self.text),
            strong: pair(base, on_color(base)),
        };
        iced_palette::Palette {
            background: Background {
                base: background(self.canvas),
                weakest: background(self.sidebar),
                weaker: background(self.surface),
                weak: background(self.raised),
                neutral: background(self.hover),
                strong: background(self.border),
                stronger: pair(self.control_border, on_color(self.control_border)),
                strongest: pair(self.muted, on_color(self.muted)),
            },
            primary: Swatch {
                base: pair(self.accent, self.on_accent),
                weak: pair(self.accent_subtle, self.text),
                strong: pair(self.accent_hover, on_color(self.accent_hover)),
            },
            secondary: Swatch {
                base: background(self.surface),
                weak: background(self.sidebar),
                strong: background(self.hover),
            },
            success: swatch(self.success),
            warning: swatch(self.warning),
            danger: swatch(self.danger),
            is_dark: self.dark,
        }
    }
}

/// Opaque endpoint foregrounds are mathematical neutral tokens (L=0 and L=1).
pub fn on_color(background: Color) -> Color {
    if background.relative_contrast(Color::BLACK) >= background.relative_contrast(Color::WHITE) {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

pub static DARK: LazyLock<Palette> = LazyLock::new(|| Palette::new(true));
pub static LIGHT: LazyLock<Palette> = LazyLock::new(|| Palette::new(false));

pub fn colors(theme: &Theme) -> &'static Palette {
    if theme.palette().is_dark {
        &DARK
    } else {
        &LIGHT
    }
}

fn make_theme(dark: bool) -> Theme {
    let p = if dark { &*DARK } else { &*LIGHT };
    let seed = iced_palette::Seed {
        background: p.canvas,
        text: p.text,
        primary: p.accent,
        success: p.success,
        warning: p.warning,
        danger: p.danger,
    };
    Theme::custom_with_fn(
        if dark { "Rustle Night" } else { "Rustle Day" },
        seed,
        |_| p.iced(),
    )
}

static DARK_THEME: LazyLock<Theme> = LazyLock::new(|| make_theme(true));
static LIGHT_THEME: LazyLock<Theme> = LazyLock::new(|| make_theme(false));

pub fn application(dark: bool) -> Theme {
    if dark {
        DARK_THEME.clone()
    } else {
        LIGHT_THEME.clone()
    }
}

fn meter_ramp(p: &Palette) -> [Color; 256] {
    std::array::from_fn(|i| {
        let t = i as f32 / 255.0;
        if t < 0.5 {
            mix(p.info, p.accent, t * 2.0)
        } else if t < 0.8 {
            mix(p.accent, p.warning, (t - 0.5) / 0.3)
        } else {
            mix(p.warning, p.danger, (t - 0.8) / 0.2)
        }
    })
}
static DARK_METER: LazyLock<[Color; 256]> = LazyLock::new(|| meter_ramp(&DARK));
static LIGHT_METER: LazyLock<[Color; 256]> = LazyLock::new(|| meter_ramp(&LIGHT));
pub fn meter(theme: &Theme, level: f32) -> Color {
    let i = (level.clamp(0.0, 1.0) * 255.0).round() as usize;
    if theme.palette().is_dark {
        DARK_METER[i]
    } else {
        LIGHT_METER[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_contrast_and_custom_modes() {
        for dark in [false, true] {
            let theme = application(dark);
            assert_eq!(theme.palette().is_dark, dark);
            let p = colors(&theme);
            for bg in [
                p.canvas,
                p.sidebar,
                p.surface,
                p.raised,
                p.hover,
                p.accent_subtle,
            ] {
                for fg in [p.text, p.secondary, p.muted] {
                    assert!(
                        fg.relative_contrast(bg) >= 4.5,
                        "dark={dark}, {fg:?} on {bg:?}"
                    );
                }
            }
            for bg in [
                p.accent,
                p.accent_hover,
                p.accent_pressed,
                p.danger,
                p.danger_hover,
                p.success,
                p.warning,
                p.info,
            ] {
                assert!(on_color(bg).relative_contrast(bg) >= 4.5);
            }
            assert!(p.on_accent.relative_contrast(p.accent_hover) >= 4.5);
            assert!(p.on_accent.relative_contrast(p.accent_pressed) >= 4.5);
            assert!(p.control_border.relative_contrast(p.surface) >= 3.0);
        }
    }
}
