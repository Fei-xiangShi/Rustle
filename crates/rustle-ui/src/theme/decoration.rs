//! Named decorative families, distinct from interaction/status roles.
use crate::color::Oklcha;
use iced::{Color, Theme};
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy)]
pub enum Tone {
    Slate,
    Cyan,
    Green,
    Violet,
    Gold,
    Blue,
    Orange,
}

const HUES: [f32; 7] = [240.0, 210.0, 150.0, 300.0, 85.0, 255.0, 45.0];
static DARK: LazyLock<[Color; 7]> = LazyLock::new(|| swatches(0.80));
static LIGHT: LazyLock<[Color; 7]> = LazyLock::new(|| swatches(0.43));
fn swatches(l: f32) -> [Color; 7] {
    std::array::from_fn(|i| {
        Oklcha::new(l, if i == 0 { 0.015 } else { 0.12 }, HUES[i], 1.0).to_color()
    })
}
pub fn ink(tone: Tone, theme: &Theme) -> Color {
    if theme.palette().is_dark {
        DARK[tone as usize]
    } else {
        LIGHT[tone as usize]
    }
}

static FEATURES: LazyLock<[[Color; 2]; 3]> = LazyLock::new(|| {
    [(185.0, 230.0), (255.0, 290.0), (210.0, 185.0)].map(|(start, end)| {
        [
            Oklcha::new(0.40, 0.085, start, 1.0).to_color(),
            Oklcha::new(0.27, 0.065, end, 1.0).to_color(),
        ]
    })
});
pub fn feature(index: usize) -> [Color; 2] {
    FEATURES[index]
}

static ARTWORK_BUTTONS: LazyLock<[Color; 2]> = LazyLock::new(|| {
    [
        Oklcha::new(0.52, 0.01, 240.0, 1.0).to_color(),
        Oklcha::new(0.65, 0.01, 240.0, 1.0).to_color(),
    ]
});
pub fn artwork_button(hovered: bool) -> Color {
    ARTWORK_BUTTONS[usize::from(hovered)]
}
static CAPSULES: LazyLock<[Color; 2]> = LazyLock::new(|| {
    [
        Oklcha::new(0.94, 0.0, 0.0, 1.0).to_color(),
        Oklcha::new(0.90, 0.0, 0.0, 1.0).to_color(),
    ]
});
pub fn capsule(pressed: bool) -> Color {
    CAPSULES[usize::from(pressed)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn badge_ink_on_tinted_surfaces() {
        for dark in [false, true] {
            let theme = crate::theme::application(dark);
            let surface = crate::theme::surface(&theme);
            for tone in [
                Tone::Slate,
                Tone::Cyan,
                Tone::Green,
                Tone::Violet,
                Tone::Gold,
                Tone::Blue,
                Tone::Orange,
            ] {
                let fg = ink(tone, &theme);
                let bg = crate::color::composite(fg.scale_alpha(0.2), surface);
                assert!(fg.relative_contrast(bg) >= 4.5, "{dark} {tone:?}");
            }
        }
    }
}
