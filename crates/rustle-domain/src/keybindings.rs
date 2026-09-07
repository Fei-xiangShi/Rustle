//! Persisted, framework-free keyboard shortcut domain model.
//!
//! This module provides a flexible keybinding system that allows users
//! to customize all keyboard shortcuts in the application.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// All bindable actions in the application
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    // Playback controls
    PlayPause,
    NextTrack,
    PrevTrack,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    SeekForward,
    SeekBackward,

    // Navigation
    GoHome,
    FocusSearch,

    // UI controls
    ToggleQueue,
    ToggleFullscreen,
}

impl Action {
    pub const ALL: [Self; 12] = [
        Self::PlayPause,
        Self::NextTrack,
        Self::PrevTrack,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::VolumeMute,
        Self::SeekForward,
        Self::SeekBackward,
        Self::GoHome,
        Self::FocusSearch,
        Self::ToggleQueue,
        Self::ToggleFullscreen,
    ];
}

/// Identifies whether the local or operating-system global shortcut is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutScope {
    Local,
    Global,
}

/// A keyboard shortcut consisting of modifiers and a key
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyBinding {
    /// Modifier keys (Ctrl, Alt, Shift, etc.)
    pub modifiers: ModifierSet,
    /// The main key
    pub key: KeyCode,
}

impl KeyBinding {
    /// Create a new keybinding
    pub fn new(key: KeyCode) -> Self {
        Self {
            modifiers: ModifierSet::default(),
            key,
        }
    }

    /// Apply the legacy platform-primary default without importing a platform
    /// adapter. Runtime event matching and formatting remain adapter-owned.
    pub fn primary(mut self) -> Self {
        #[cfg(target_os = "macos")]
        {
            self.modifiers.cmd = true;
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.modifiers.ctrl = true;
        }
        self
    }
}

/// Set of modifier keys
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ModifierSet {
    pub ctrl: bool,
    #[serde(default)]
    pub cmd: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Supported key codes for binding
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyCode {
    // Letters
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,

    // Numbers
    Key0,
    Key1,
    Key2,
    Key3,
    Key4,
    Key5,
    Key6,
    Key7,
    Key8,
    Key9,

    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    // Navigation
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,

    // Special
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,

    // Media keys
    MediaPlayPause,
    MediaNext,
    MediaPrev,
    VolumeUp,
    VolumeDown,
    VolumeMute,
}

impl KeyCode {
    /// Get display name for the key
    pub fn display(&self) -> &'static str {
        match self {
            KeyCode::A => "A",
            KeyCode::B => "B",
            KeyCode::C => "C",
            KeyCode::D => "D",
            KeyCode::E => "E",
            KeyCode::F => "F",
            KeyCode::G => "G",
            KeyCode::H => "H",
            KeyCode::I => "I",
            KeyCode::J => "J",
            KeyCode::K => "K",
            KeyCode::L => "L",
            KeyCode::M => "M",
            KeyCode::N => "N",
            KeyCode::O => "O",
            KeyCode::P => "P",
            KeyCode::Q => "Q",
            KeyCode::R => "R",
            KeyCode::S => "S",
            KeyCode::T => "T",
            KeyCode::U => "U",
            KeyCode::V => "V",
            KeyCode::W => "W",
            KeyCode::X => "X",
            KeyCode::Y => "Y",
            KeyCode::Z => "Z",
            KeyCode::Key0 => "0",
            KeyCode::Key1 => "1",
            KeyCode::Key2 => "2",
            KeyCode::Key3 => "3",
            KeyCode::Key4 => "4",
            KeyCode::Key5 => "5",
            KeyCode::Key6 => "6",
            KeyCode::Key7 => "7",
            KeyCode::Key8 => "8",
            KeyCode::Key9 => "9",
            KeyCode::F1 => "F1",
            KeyCode::F2 => "F2",
            KeyCode::F3 => "F3",
            KeyCode::F4 => "F4",
            KeyCode::F5 => "F5",
            KeyCode::F6 => "F6",
            KeyCode::F7 => "F7",
            KeyCode::F8 => "F8",
            KeyCode::F9 => "F9",
            KeyCode::F10 => "F10",
            KeyCode::F11 => "F11",
            KeyCode::F12 => "F12",
            KeyCode::Up => "↑",
            KeyCode::Down => "↓",
            KeyCode::Left => "←",
            KeyCode::Right => "→",
            KeyCode::Home => "Home",
            KeyCode::End => "End",
            KeyCode::PageUp => "PageUp",
            KeyCode::PageDown => "PageDown",
            KeyCode::Space => "Space",
            KeyCode::Enter => "Enter",
            KeyCode::Escape => "Esc",
            KeyCode::Tab => "Tab",
            KeyCode::Backspace => "Backspace",
            KeyCode::Delete => "Delete",
            KeyCode::MediaPlayPause => "Media Play",
            KeyCode::MediaNext => "Media Next",
            KeyCode::MediaPrev => "Media Prev",
            KeyCode::VolumeUp => "Vol+",
            KeyCode::VolumeDown => "Vol-",
            KeyCode::VolumeMute => "Mute",
        }
    }
}

/// The keybindings configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindings {
    /// Map from action to application-local keybindings.
    bindings: HashMap<Action, Vec<KeyBinding>>,
    /// Map from action to operating-system global keybindings.
    #[serde(default = "default_global_bindings")]
    global_bindings: HashMap<Action, Vec<KeyBinding>>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            bindings: default_local_bindings(),
            global_bindings: default_global_bindings(),
        }
    }
}

fn default_local_bindings() -> HashMap<Action, Vec<KeyBinding>> {
    let mut bindings = HashMap::new();

    // Default keybindings
    // Playback
    bindings.insert(
        Action::PlayPause,
        vec![
            KeyBinding::new(KeyCode::Space),
            KeyBinding::new(KeyCode::MediaPlayPause),
        ],
    );
    bindings.insert(
        Action::NextTrack,
        vec![
            KeyBinding::new(KeyCode::N).primary(),
            KeyBinding::new(KeyCode::MediaNext),
        ],
    );
    bindings.insert(
        Action::PrevTrack,
        vec![
            KeyBinding::new(KeyCode::P).primary(),
            KeyBinding::new(KeyCode::MediaPrev),
        ],
    );
    bindings.insert(
        Action::VolumeUp,
        vec![
            KeyBinding::new(KeyCode::Up).primary(),
            KeyBinding::new(KeyCode::VolumeUp),
        ],
    );
    bindings.insert(
        Action::VolumeDown,
        vec![
            KeyBinding::new(KeyCode::Down).primary(),
            KeyBinding::new(KeyCode::VolumeDown),
        ],
    );
    bindings.insert(
        Action::VolumeMute,
        vec![
            KeyBinding::new(KeyCode::M).primary(),
            KeyBinding::new(KeyCode::VolumeMute),
        ],
    );
    bindings.insert(
        Action::SeekForward,
        vec![KeyBinding::new(KeyCode::Right).primary()],
    );
    bindings.insert(
        Action::SeekBackward,
        vec![KeyBinding::new(KeyCode::Left).primary()],
    );

    // Navigation
    bindings.insert(Action::GoHome, vec![KeyBinding::new(KeyCode::H).primary()]);
    bindings.insert(
        Action::FocusSearch,
        vec![KeyBinding::new(KeyCode::K).primary()],
    );

    // UI
    bindings.insert(Action::ToggleQueue, vec![KeyBinding::new(KeyCode::Q)]);
    bindings.insert(
        Action::ToggleFullscreen,
        vec![KeyBinding::new(KeyCode::F11)],
    );

    bindings
}

fn default_global_bindings() -> HashMap<Action, Vec<KeyBinding>> {
    let local_bindings = default_local_bindings();

    Action::ALL
        .into_iter()
        .filter_map(|action| {
            local_bindings
                .get(&action)
                .and_then(|bindings| bindings.first())
                .map(|binding| {
                    (
                        action,
                        vec![KeyBinding {
                            modifiers: ModifierSet {
                                ctrl: true,
                                alt: true,
                                ..ModifierSet::default()
                            },
                            key: binding.key.clone(),
                        }],
                    )
                })
        })
        .collect()
}

impl KeyBindings {
    /// Set application-local keybindings for an action.
    pub fn set(&mut self, action: Action, bindings: Vec<KeyBinding>) {
        self.bindings.insert(action, bindings);
    }

    /// Set operating-system global keybindings for an action.
    pub fn set_global(&mut self, action: Action, bindings: Vec<KeyBinding>) {
        self.global_bindings.insert(action, bindings);
    }

    /// Get the first application-local binding for an action.
    pub fn local_binding(&self, action: &Action) -> Option<&KeyBinding> {
        self.local_bindings(action).first()
    }

    /// Get every application-local binding for an action.
    pub fn local_bindings(&self, action: &Action) -> &[KeyBinding] {
        self.bindings.get(action).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Get the first global binding for an action.
    pub fn global_binding(&self, action: &Action) -> Option<&KeyBinding> {
        self.global_bindings(action).first()
    }

    /// Get every operating-system global binding for an action.
    pub fn global_bindings(&self, action: &Action) -> &[KeyBinding] {
        self.global_bindings
            .get(action)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Iterate configured global bindings in a deterministic action order.
    pub fn configured_global_bindings(&self) -> impl Iterator<Item = (Action, &KeyBinding)> {
        Action::ALL.into_iter().filter_map(|action| {
            self.global_binding(&action)
                .map(|binding| (action, binding))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_local_and_global_key_identity() {
        let bindings = KeyBindings::default();

        for action in Action::ALL {
            let local = bindings.local_binding(&action).unwrap();
            let global = bindings.global_binding(&action).unwrap();
            assert_eq!(global.key, local.key);
            assert_eq!(
                global.modifiers,
                ModifierSet {
                    ctrl: true,
                    alt: true,
                    ..ModifierSet::default()
                }
            );
        }
    }

    #[test]
    fn missing_global_bindings_migrate_to_defaults() {
        let mut serialized = serde_json::to_value(KeyBindings::default()).unwrap();
        serialized
            .as_object_mut()
            .unwrap()
            .remove("global_bindings");

        let migrated: KeyBindings = serde_json::from_value(serialized).unwrap();

        assert_eq!(
            migrated.global_binding(&Action::PlayPause).unwrap().key,
            KeyCode::Space
        );
        assert_eq!(
            migrated.global_binding(&Action::NextTrack).unwrap().key,
            KeyCode::N
        );
    }

    #[test]
    fn persisted_keybinding_shape_remains_compatible() {
        let binding = KeyBinding {
            modifiers: ModifierSet {
                ctrl: true,
                cmd: false,
                alt: true,
                shift: false,
            },
            key: KeyCode::Space,
        };
        let value = serde_json::to_value(&binding).unwrap();

        assert_eq!(value["key"], "space");
        assert_eq!(value["modifiers"]["ctrl"], true);
        assert_eq!(value["modifiers"]["cmd"], false);
        assert_eq!(
            serde_json::from_value::<KeyBinding>(value).unwrap(),
            binding
        );
    }
}
