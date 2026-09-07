//! SVG icons for the music streaming UI
//! Using filled SVG icons for better visual consistency

/// Music note icon for logo
pub const MUSIC_LOGO: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z"/>
</svg>"#;

/// Music note icon (filled)
pub const MUSIC: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z"/>
</svg>"#;

/// Home icon (filled)
pub const HOME: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"/>
</svg>"#;

/// Browse/Compass icon (filled) - compass style
pub const BROWSE: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm-5.5-2.5l7.51-3.49L17.5 6.5 9.99 9.99 6.5 17.5zm5.5-6.6c.61 0 1.1.49 1.1 1.1s-.49 1.1-1.1 1.1-1.1-.49-1.1-1.1.49-1.1 1.1-1.1z"/>
</svg>"#;

/// Radio/broadcast icon (filled) - radio tower style
pub const RADIO: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M3.24 6.15C2.51 6.43 2 7.17 2 8v12c0 1.1.89 2 2 2h16c1.11 0 2-.9 2-2V8c0-1.11-.89-2-2-2H8.3l8.26-3.34-.37-.92L3.24 6.15zM7 20c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm13-8h-2v-2h-2v2H4V8h16v4z"/>
</svg>"#;

/// Heart/favorite icon (filled)
pub const HEART: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/>
</svg>"#;

/// Heart outline icon (for unhovered state)
pub const HEART_OUTLINE: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
</svg>"#;

/// Search icon (filled)
pub const SEARCH: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/>
</svg>"#;

/// Play icon (filled triangle)
pub const PLAY: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <polygon points="5 3 19 12 5 21 5 3"/>
</svg>"#;

/// Pause icon
pub const PAUSE: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <rect x="6" y="4" width="4" height="16"/>
    <rect x="14" y="4" width="4" height="16"/>
</svg>"#;

/// Skip next icon
pub const SKIP_NEXT: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <polygon points="5 4 15 12 5 20 5 4"/>
    <line x1="19" y1="5" x2="19" y2="19" stroke="currentColor" stroke-width="2"/>
</svg>"#;

/// Skip previous icon
pub const SKIP_PREV: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <polygon points="19 20 9 12 19 4 19 20"/>
    <line x1="5" y1="5" x2="5" y2="19" stroke="currentColor" stroke-width="2"/>
</svg>"#;

/// Plus/add icon (filled)
pub const PLUS: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"/>
</svg>"#;

/// Clock/recent icon (filled)
pub const CLOCK: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z"/>
</svg>"#;

/// Calendar icon used by Daily Recommend.
pub const CALENDAR: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M19 4h-1V2h-2v2H8V2H6v2H5c-1.11 0-1.99.9-1.99 2L3 20c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 16H5V9h14v11zM7 11h5v5H7z"/>
</svg>"#;

/// Download icon (filled)
pub const DOWNLOAD: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"/>
</svg>"#;

/// Settings/gear icon (filled)
pub const SETTINGS: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M19.14 12.94c.04-.31.06-.63.06-.94 0-.31-.02-.63-.06-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.04.31-.06.63-.06.94s.02.63.06.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/>
</svg>"#;

/// Volume icon (filled)
pub const VOLUME: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/>
</svg>"#;

/// Shuffle icon (filled)
pub const SHUFFLE: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"/>
</svg>"#;

/// Queue/playlist icon (filled)
pub const QUEUE: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z"/>
</svg>"#;

/// User/profile icon (filled)
pub const USER: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"/>
</svg>"#;

/// Chevron right (filled)
pub const CHEVRON_RIGHT: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z"/>
</svg>"#;

/// List/menu icon (filled)
pub const LIST: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z"/>
</svg>"#;

/// Check/checkmark icon (filled)
pub const CHECK: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/>
</svg>"#;

/// Error/X icon (filled)
pub const ERROR: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/>
</svg>"#;

/// Warning icon (filled triangle)
pub const WARNING: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z"/>
</svg>"#;

/// Info icon (filled circle with cutout details)
pub const INFO: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path fill-rule="evenodd" clip-rule="evenodd" d="M12 2a10 10 0 1 1 0 20 10 10 0 0 1 0-20zm1 15v-6h-2v6h2zm0-8V7h-2v2h2z"/>
</svg>"#;

/// Edit/pencil icon (filled)
pub const EDIT: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34c-.39-.39-1.02-.39-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/>
</svg>"#;

/// Trash/delete icon (filled)
pub const TRASH: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/>
</svg>"#;

/// Playing indicator icon (equalizer bars - static representation)
pub const PLAYING: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <rect x="4" y="12" width="3" height="6" rx="1"/>
    <rect x="9" y="8" width="3" height="10" rx="1"/>
    <rect x="14" y="10" width="3" height="8" rx="1"/>
    <rect x="19" y="6" width="3" height="12" rx="1"/>
</svg>"#;

/// Close/X icon (filled)
pub const CLOSE: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/>
</svg>"#;

/// Sequential play icon (arrow right with line)
pub const PLAY_SEQUENTIAL: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M4 18l8.5-6L4 6v12zm9-12v12l8.5-6L13 6z"/>
</svg>"#;

/// Loop all icon (repeat arrows)
pub const LOOP_ALL: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"/>
</svg>"#;

/// Loop one icon (repeat with 1)
pub const LOOP_ONE: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"/>
    <rect x="11" y="9.5" width="2" height="5.5" rx="0.5"/>
    <path d="M10 10.5h2.5V8.8l-1.2.9H10z"/>
</svg>"#;

/// Minimize icon (line)
pub const MINIMIZE: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
    <line x1="5" y1="12" x2="19" y2="12"/>
</svg>"#;

/// Maximize/fullscreen icon
pub const MAXIMIZE: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <rect x="4" y="4" width="16" height="16" rx="2"/>
</svg>"#;

/// Restore-down icon shown while the window is maximized
pub const RESTORE: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M9 8V5h10v10h-3"/>
    <rect x="5" y="8" width="11" height="11" rx="1"/>
</svg>"#;

/// Return the correct maximize control icon for the current window state.
pub fn maximize_restore(is_maximized: bool) -> &'static str {
    if is_maximized { RESTORE } else { MAXIMIZE }
}

/// Refresh icon (filled)
pub const REFRESH: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M17.65 6.35C16.2 4.9 14.21 4 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08c-.82 2.33-3.04 4-5.65 4-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"/>
</svg>"#;

/// Logout icon (filled)
pub const LOGOUT: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M17 7l-1.41 1.41L18.17 11H8v2h10.17l-2.58 2.58L17 17l5-5zM4 5h8V3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h8v-2H4V5z"/>
</svg>"#;

/// Equalizer icon (audio bars)
pub const EQUALIZER: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M10 20h4V4h-4v16zm-6 0h4v-8H4v8zM16 9v11h4V9h-4z"/>
</svg>"#;

/// Chevron down icon (filled)
pub const CHEVRON_DOWN: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6 1.41-1.41z"/>
</svg>"#;

/// Loading/spinner icon (circular dots)
pub const LOADING: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <circle cx="12" cy="4" r="2"/>
    <circle cx="12" cy="20" r="2" opacity="0.3"/>
    <circle cx="4" cy="12" r="2" opacity="0.5"/>
    <circle cx="20" cy="12" r="2" opacity="0.7"/>
    <circle cx="6.34" cy="6.34" r="2" opacity="0.9"/>
    <circle cx="17.66" cy="17.66" r="2" opacity="0.2"/>
    <circle cx="6.34" cy="17.66" r="2" opacity="0.4"/>
    <circle cx="17.66" cy="6.34" r="2" opacity="0.8"/>
</svg>"#;

/// Folder icon
pub const FOLDER: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/>
</svg>"#;

/// Disc/Album icon  
pub const DISC: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="2"/>
    <circle cx="12" cy="12" r="3" fill="currentColor"/>
    <circle cx="12" cy="12" r="1" fill="none" stroke="currentColor" stroke-width="1.5"/>
</svg>"#;

/// Hard drive icon (for local source badge)
pub const HARD_DRIVE: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 14H4V6h16v12z"/>
    <circle cx="8" cy="16" r="1.5"/>
    <circle cx="16" cy="16" r="1.5"/>
</svg>"#;

/// Cloud icon (for online source badge)
pub const CLOUD: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96z"/>
</svg>"#;

#[cfg(test)]
mod tests {
    use super::{MAXIMIZE, RESTORE, maximize_restore};

    #[test]
    fn maximize_control_icon_follows_window_state() {
        assert_eq!(maximize_restore(false), MAXIMIZE);
        assert_eq!(maximize_restore(true), RESTORE);
    }
}
