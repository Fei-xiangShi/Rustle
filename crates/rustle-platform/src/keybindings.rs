//! Platform-specific keybinding display
//!
//! Provides modifier key symbols and display functions that vary by platform.

use iced::keyboard::{Key, Modifiers};

use rustle_domain::keybindings::{Action, KeyBinding, KeyBindings, KeyCode, ModifierSet};

/// Modifier key symbols for display
pub struct ModifierSymbols {
    pub ctrl: &'static str,
    pub alt: &'static str,
    pub shift: &'static str,
    pub cmd: &'static str,
}

/// Platform-specific modifier symbols
#[cfg(target_os = "macos")]
pub const MODIFIER_SYMBOLS: ModifierSymbols = ModifierSymbols {
    ctrl: "⌃",
    alt: "⌥",
    shift: "⇧",
    cmd: "⌘",
};

#[cfg(not(target_os = "macos"))]
pub const MODIFIER_SYMBOLS: ModifierSymbols = ModifierSymbols {
    ctrl: "Ctrl",
    alt: "Alt",
    shift: "Shift",
    cmd: "Win", // Windows key on non-macOS
};

/// Check if the cmd modifier matches
/// - macOS: Checks if cmd flag matches logo key state
/// - Others: cmd should not be set (returns true only if cmd is false)
#[cfg(target_os = "macos")]
pub fn matches_cmd_modifier(cmd: bool, modifiers: &Modifiers) -> bool {
    cmd == modifiers.logo()
}

#[cfg(not(target_os = "macos"))]
pub fn matches_cmd_modifier(cmd: bool, _modifiers: &Modifiers) -> bool {
    // On non-macOS, cmd should not be set
    !cmd
}

pub fn binding_matches(binding: &KeyBinding, key: &Key, modifiers: &Modifiers) -> bool {
    key_to_keycode(key).as_ref() == Some(&binding.key)
        && local_modifiers_match(&binding.modifiers, modifiers)
}

pub fn find_action(bindings: &KeyBindings, key: &Key, modifiers: &Modifiers) -> Option<Action> {
    Action::ALL.into_iter().find(|action| {
        bindings
            .local_bindings(action)
            .iter()
            .any(|binding| binding_matches(binding, key, modifiers))
    })
}

pub fn find_global_action(
    bindings: &KeyBindings,
    key: &Key,
    modifiers: &Modifiers,
) -> Option<Action> {
    Action::ALL.into_iter().find(|action| {
        bindings.global_bindings(action).iter().any(|binding| {
            key_to_keycode(key).as_ref() == Some(&binding.key)
                && global_modifiers_match(&binding.modifiers, modifiers)
        })
    })
}

pub fn display_binding(binding: Option<&KeyBinding>) -> String {
    let Some(binding) = binding else {
        return "None".to_string();
    };
    let mut parts = Vec::new();
    if binding.modifiers.cmd {
        parts.push(MODIFIER_SYMBOLS.cmd);
    }
    if binding.modifiers.ctrl {
        parts.push(MODIFIER_SYMBOLS.ctrl);
    }
    if binding.modifiers.alt {
        parts.push(MODIFIER_SYMBOLS.alt);
    }
    if binding.modifiers.shift {
        parts.push(MODIFIER_SYMBOLS.shift);
    }
    parts.push(binding.key.display());
    parts.join("+")
}

pub fn display_for_action(bindings: &KeyBindings, action: &Action) -> String {
    display_binding(bindings.local_binding(action))
}

pub fn display_global_for_action(bindings: &KeyBindings, action: &Action) -> String {
    display_binding(bindings.global_binding(action))
}

fn local_modifiers_match(expected: &ModifierSet, modifiers: &Modifiers) -> bool {
    expected.ctrl == modifiers.control()
        && matches_cmd_modifier(expected.cmd, modifiers)
        && expected.alt == modifiers.alt()
        && expected.shift == modifiers.shift()
}

fn global_modifiers_match(expected: &ModifierSet, modifiers: &Modifiers) -> bool {
    expected.ctrl == modifiers.control()
        && expected.cmd == modifiers.logo()
        && expected.alt == modifiers.alt()
        && expected.shift == modifiers.shift()
}

/// Convert iced Key to our KeyCode
pub fn key_to_keycode(key: &Key) -> Option<KeyCode> {
    match key {
        Key::Character(c) => {
            let c = c.to_lowercase();
            match c.as_str() {
                "a" => Some(KeyCode::A),
                "b" => Some(KeyCode::B),
                "c" => Some(KeyCode::C),
                "d" => Some(KeyCode::D),
                "e" => Some(KeyCode::E),
                "f" => Some(KeyCode::F),
                "g" => Some(KeyCode::G),
                "h" => Some(KeyCode::H),
                "i" => Some(KeyCode::I),
                "j" => Some(KeyCode::J),
                "k" => Some(KeyCode::K),
                "l" => Some(KeyCode::L),
                "m" => Some(KeyCode::M),
                "n" => Some(KeyCode::N),
                "o" => Some(KeyCode::O),
                "p" => Some(KeyCode::P),
                "q" => Some(KeyCode::Q),
                "r" => Some(KeyCode::R),
                "s" => Some(KeyCode::S),
                "t" => Some(KeyCode::T),
                "u" => Some(KeyCode::U),
                "v" => Some(KeyCode::V),
                "w" => Some(KeyCode::W),
                "x" => Some(KeyCode::X),
                "y" => Some(KeyCode::Y),
                "z" => Some(KeyCode::Z),
                "0" => Some(KeyCode::Key0),
                "1" => Some(KeyCode::Key1),
                "2" => Some(KeyCode::Key2),
                "3" => Some(KeyCode::Key3),
                "4" => Some(KeyCode::Key4),
                "5" => Some(KeyCode::Key5),
                "6" => Some(KeyCode::Key6),
                "7" => Some(KeyCode::Key7),
                "8" => Some(KeyCode::Key8),
                "9" => Some(KeyCode::Key9),
                " " => Some(KeyCode::Space),
                _ => None,
            }
        }
        Key::Named(named) => {
            use iced::keyboard::key::Named;
            match named {
                Named::Space => Some(KeyCode::Space),
                Named::Enter => Some(KeyCode::Enter),
                Named::Escape => Some(KeyCode::Escape),
                Named::Tab => Some(KeyCode::Tab),
                Named::Backspace => Some(KeyCode::Backspace),
                Named::Delete => Some(KeyCode::Delete),
                Named::ArrowUp => Some(KeyCode::Up),
                Named::ArrowDown => Some(KeyCode::Down),
                Named::ArrowLeft => Some(KeyCode::Left),
                Named::ArrowRight => Some(KeyCode::Right),
                Named::Home => Some(KeyCode::Home),
                Named::End => Some(KeyCode::End),
                Named::PageUp => Some(KeyCode::PageUp),
                Named::PageDown => Some(KeyCode::PageDown),
                Named::F1 => Some(KeyCode::F1),
                Named::F2 => Some(KeyCode::F2),
                Named::F3 => Some(KeyCode::F3),
                Named::F4 => Some(KeyCode::F4),
                Named::F5 => Some(KeyCode::F5),
                Named::F6 => Some(KeyCode::F6),
                Named::F7 => Some(KeyCode::F7),
                Named::F8 => Some(KeyCode::F8),
                Named::F9 => Some(KeyCode::F9),
                Named::F10 => Some(KeyCode::F10),
                Named::F11 => Some(KeyCode::F11),
                Named::F12 => Some(KeyCode::F12),
                _ => None,
            }
        }
        Key::Unidentified => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_space_binding_matches_character_and_named_space() {
        let bindings = KeyBindings::default();
        let modifiers = Modifiers::default();

        assert_eq!(
            find_action(&bindings, &Key::Character(" ".into()), &modifiers),
            Some(Action::PlayPause)
        );
        assert_eq!(
            find_action(
                &bindings,
                &Key::Named(iced::keyboard::key::Named::Space),
                &modifiers,
            ),
            Some(Action::PlayPause)
        );
        assert_eq!(
            key_to_keycode(&Key::Character(" ".into())),
            Some(KeyCode::Space)
        );
    }

    #[test]
    fn display_uses_platform_modifier_symbols() {
        let binding = KeyBinding {
            modifiers: ModifierSet {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
            key: KeyCode::P,
        };

        #[cfg(target_os = "macos")]
        assert_eq!(display_binding(Some(&binding)), "⌃+⇧+P");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(display_binding(Some(&binding)), "Ctrl+Shift+P");
    }

    #[test]
    fn global_matching_compares_logo_exactly() {
        let mut bindings = KeyBindings::default();
        bindings.set_global(
            Action::ToggleQueue,
            vec![KeyBinding {
                modifiers: ModifierSet {
                    cmd: true,
                    ..Default::default()
                },
                key: KeyCode::Q,
            }],
        );

        assert_eq!(
            find_global_action(&bindings, &Key::Character("q".into()), &Modifiers::LOGO),
            Some(Action::ToggleQueue)
        );
        assert_eq!(
            find_global_action(
                &bindings,
                &Key::Character("q".into()),
                &Modifiers::default(),
            ),
            None
        );
    }
}
