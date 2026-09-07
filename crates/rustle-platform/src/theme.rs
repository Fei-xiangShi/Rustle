//! Platform-specific theme constants
//!
//! Provides font weights and other theme values that vary by platform.

use cosmic_text::{FontSystem, fontdb::Database};
/// Configures the global `iced` font database so generic sans-serif text
/// resolves to the intended platform UI family.
pub fn configure_iced_font_system() {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let mut font_system = iced::advanced::graphics::text::font_system()
            .write()
            .expect("lock iced font system");

        if let Some(family) = preferred_sans_serif_family(font_system.raw().db()) {
            font_system.raw().db_mut().set_sans_serif_family(family);
        }
    }
}

/// Configures a standalone `cosmic-text` font system so `Family::SansSerif`
/// resolves to a stable platform UI family instead of cosmic-text's default
/// generic mapping.
pub fn configure_cosmic_font_system(font_system: &mut FontSystem) {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    if let Some(family) = preferred_sans_serif_family(font_system.db()) {
        font_system.db_mut().set_sans_serif_family(family);
    }
}

/// Resolves the preferred lyrics font family.
///
/// Lyrics need a sans family that can cover Latin and CJK consistently.
/// The general UI can keep using the platform default sans-serif mapping, but
/// the lyrics renderer should prefer a CJK-capable sans family first to avoid
/// falling into an arbitrary serif/bitmap fallback for Chinese glyphs.
pub fn preferred_lyrics_font_family(db: &Database) -> Option<&'static str> {
    preferred_lyrics_sans_serif_candidates()
        .iter()
        .copied()
        .find(|family| has_font_family(db, family))
}

fn preferred_sans_serif_family(db: &Database) -> Option<&'static str> {
    preferred_sans_serif_candidates()
        .iter()
        .copied()
        .find(|family| has_font_family(db, family))
}

/// List all installed font family names from the font database.
///
/// Returns a sorted, deduplicated list of all font families available
/// on the system. Useful for populating a font picker in the UI.
pub fn list_installed_font_families(db: &Database) -> Vec<String> {
    let mut families: Vec<String> = db
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
        .collect();
    families.sort_by_key(|a| a.to_lowercase());
    families.dedup();
    families
}

fn has_font_family(db: &Database, candidate: &str) -> bool {
    db.faces().any(|face| {
        face.families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(candidate))
    })
}

#[cfg(target_os = "linux")]
fn preferred_sans_serif_candidates() -> &'static [&'static str] {
    &[
        "Noto Sans",
        "Noto Sans SC",
        "Noto Sans CJK SC",
        "Source Han Sans CN",
        "Segoe UI Variable",
        "Segoe UI",
        "Microsoft YaHei UI",
        "Microsoft YaHei",
        "Arial",
    ]
}

#[cfg(target_os = "linux")]
fn preferred_lyrics_sans_serif_candidates() -> &'static [&'static str] {
    &[
        "Noto Sans CJK SC",
        "Noto Sans SC",
        "Source Han Sans CN",
        "Microsoft YaHei UI",
        "Microsoft YaHei",
        "Noto Sans",
        "Segoe UI Variable",
        "Segoe UI",
        "Arial",
    ]
}

#[cfg(target_os = "macos")]
fn preferred_sans_serif_candidates() -> &'static [&'static str] {
    &[
        ".SF NS",
        "PingFang SC",
        "Hiragino Sans GB",
        "Helvetica Neue",
        "Arial",
    ]
}

#[cfg(target_os = "macos")]
fn preferred_lyrics_sans_serif_candidates() -> &'static [&'static str] {
    &[
        "PingFang SC",
        ".SF NS",
        "Hiragino Sans GB",
        "Helvetica Neue",
        "Arial",
    ]
}

#[cfg(target_os = "windows")]
fn preferred_sans_serif_candidates() -> &'static [&'static str] {
    &[
        "Segoe UI Variable",
        "Segoe UI",
        "Microsoft YaHei UI",
        "Microsoft YaHei",
        "Yu Gothic UI",
        "Meiryo UI",
        "Arial",
    ]
}

#[cfg(target_os = "windows")]
fn preferred_lyrics_sans_serif_candidates() -> &'static [&'static str] {
    &[
        "Microsoft YaHei UI",
        "Segoe UI Variable",
        "Segoe UI",
        "Microsoft YaHei",
        "Yu Gothic UI",
        "Meiryo UI",
        "Arial",
    ]
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn preferred_sans_serif_candidates() -> &'static [&'static str] {
    &[]
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn preferred_lyrics_sans_serif_candidates() -> &'static [&'static str] {
    &[]
}
