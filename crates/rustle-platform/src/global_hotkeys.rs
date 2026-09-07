//! Operating-system global shortcut registration and event delivery.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tokio::sync::mpsc;

use rustle_domain::keybindings::{Action, KeyBinding, KeyBindings, KeyCode};

/// Owns the native manager and the subset of configured shortcuts that are
/// currently registered successfully with the operating system.
pub struct GlobalHotkeyService {
    manager: GlobalHotKeyManager,
    registered: HashMap<Action, HotKey>,
}

impl GlobalHotkeyService {
    /// Create the native manager and independently register each configured
    /// shortcut so one conflict does not disable the rest.
    pub fn new(bindings: &KeyBindings) -> Result<Self, GlobalHotkeyError> {
        ensure_supported_desktop_session()?;

        let manager = GlobalHotKeyManager::new().map_err(GlobalHotkeyError::backend_unavailable)?;
        let mut service = Self {
            manager,
            registered: HashMap::new(),
        };

        for (action, binding) in bindings.configured_global_bindings() {
            let Some(hotkey) = hotkey_for_binding(binding) else {
                tracing::warn!(?action, "Unsupported configured global hotkey");
                continue;
            };

            match service.manager.register(hotkey) {
                Ok(()) => {
                    service.registered.insert(action, hotkey);
                }
                Err(error) => {
                    tracing::warn!(
                        ?action,
                        binding = %crate::keybindings::display_binding(Some(binding)),
                        %error,
                        "Failed to register configured global hotkey"
                    );
                }
            }
        }

        tracing::info!(
            registered = service.registered.len(),
            thread = ?std::thread::current().id(),
            "Initialized global hotkeys"
        );

        Ok(service)
    }

    /// Transactionally replace one action's active native registration.
    ///
    /// The prior working registration is restored when the operating system
    /// rejects the replacement.
    pub fn replace(
        &mut self,
        action: Action,
        binding: Option<&KeyBinding>,
    ) -> Result<(), GlobalHotkeyError> {
        let new_hotkey = binding
            .map(|binding| {
                hotkey_for_binding(binding).ok_or_else(GlobalHotkeyError::unsupported_binding)
            })
            .transpose()?;

        replace_registration(&self.manager, &mut self.registered, action, new_hotkey)
    }

    pub fn is_registered(&self, action: Action) -> bool {
        self.registered.contains_key(&action)
    }

    /// Resolve an event against the registrations that are actually active,
    /// rather than merely configured in settings.
    pub fn action_for_id(&self, id: u32) -> Option<Action> {
        action_for_registered_id(&self.registered, id)
    }
}

fn action_for_registered_id(registered: &HashMap<Action, HotKey>, id: u32) -> Option<Action> {
    registered
        .iter()
        .find_map(|(action, hotkey)| (hotkey.id() == id).then_some(*action))
}

#[cfg(target_os = "linux")]
fn ensure_supported_desktop_session() -> Result<(), GlobalHotkeyError> {
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland_display_present =
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
    if session_is_wayland(&session_type, wayland_display_present) {
        return Err(GlobalHotkeyError::unsupported_session());
    }

    x11rb::connect(None)
        .map(|_| ())
        .map_err(GlobalHotkeyError::backend_unavailable)
}

#[cfg(not(target_os = "linux"))]
fn ensure_supported_desktop_session() -> Result<(), GlobalHotkeyError> {
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn session_is_wayland(session_type: &str, wayland_display_present: bool) -> bool {
    session_type.eq_ignore_ascii_case("wayland")
        || (session_type.is_empty() && wayland_display_present)
}

/// Convert Rustle's serialized shortcut representation to the native crate's
/// physical-key representation.
pub fn hotkey_for_binding(binding: &KeyBinding) -> Option<HotKey> {
    let mut modifiers = Modifiers::empty();
    if binding.modifiers.ctrl {
        modifiers |= Modifiers::CONTROL;
    }
    if binding.modifiers.cmd {
        modifiers |= Modifiers::SUPER;
    }
    if binding.modifiers.alt {
        modifiers |= Modifiers::ALT;
    }
    if binding.modifiers.shift {
        modifiers |= Modifiers::SHIFT;
    }

    Some(HotKey::new(Some(modifiers), code_for_key(&binding.key)))
}

fn code_for_key(key: &KeyCode) -> Code {
    match key {
        KeyCode::A => Code::KeyA,
        KeyCode::B => Code::KeyB,
        KeyCode::C => Code::KeyC,
        KeyCode::D => Code::KeyD,
        KeyCode::E => Code::KeyE,
        KeyCode::F => Code::KeyF,
        KeyCode::G => Code::KeyG,
        KeyCode::H => Code::KeyH,
        KeyCode::I => Code::KeyI,
        KeyCode::J => Code::KeyJ,
        KeyCode::K => Code::KeyK,
        KeyCode::L => Code::KeyL,
        KeyCode::M => Code::KeyM,
        KeyCode::N => Code::KeyN,
        KeyCode::O => Code::KeyO,
        KeyCode::P => Code::KeyP,
        KeyCode::Q => Code::KeyQ,
        KeyCode::R => Code::KeyR,
        KeyCode::S => Code::KeyS,
        KeyCode::T => Code::KeyT,
        KeyCode::U => Code::KeyU,
        KeyCode::V => Code::KeyV,
        KeyCode::W => Code::KeyW,
        KeyCode::X => Code::KeyX,
        KeyCode::Y => Code::KeyY,
        KeyCode::Z => Code::KeyZ,
        KeyCode::Key0 => Code::Digit0,
        KeyCode::Key1 => Code::Digit1,
        KeyCode::Key2 => Code::Digit2,
        KeyCode::Key3 => Code::Digit3,
        KeyCode::Key4 => Code::Digit4,
        KeyCode::Key5 => Code::Digit5,
        KeyCode::Key6 => Code::Digit6,
        KeyCode::Key7 => Code::Digit7,
        KeyCode::Key8 => Code::Digit8,
        KeyCode::Key9 => Code::Digit9,
        KeyCode::F1 => Code::F1,
        KeyCode::F2 => Code::F2,
        KeyCode::F3 => Code::F3,
        KeyCode::F4 => Code::F4,
        KeyCode::F5 => Code::F5,
        KeyCode::F6 => Code::F6,
        KeyCode::F7 => Code::F7,
        KeyCode::F8 => Code::F8,
        KeyCode::F9 => Code::F9,
        KeyCode::F10 => Code::F10,
        KeyCode::F11 => Code::F11,
        KeyCode::F12 => Code::F12,
        KeyCode::Up => Code::ArrowUp,
        KeyCode::Down => Code::ArrowDown,
        KeyCode::Left => Code::ArrowLeft,
        KeyCode::Right => Code::ArrowRight,
        KeyCode::Home => Code::Home,
        KeyCode::End => Code::End,
        KeyCode::PageUp => Code::PageUp,
        KeyCode::PageDown => Code::PageDown,
        KeyCode::Space => Code::Space,
        KeyCode::Enter => Code::Enter,
        KeyCode::Escape => Code::Escape,
        KeyCode::Tab => Code::Tab,
        KeyCode::Backspace => Code::Backspace,
        KeyCode::Delete => Code::Delete,
        KeyCode::MediaPlayPause => Code::MediaPlayPause,
        KeyCode::MediaNext => Code::MediaTrackNext,
        KeyCode::MediaPrev => Code::MediaTrackPrevious,
        KeyCode::VolumeUp => Code::AudioVolumeUp,
        KeyCode::VolumeDown => Code::AudioVolumeDown,
        KeyCode::VolumeMute => Code::AudioVolumeMute,
    }
}

/// Install event-driven delivery before native shortcuts are registered.
pub fn install_event_handler() -> mpsc::UnboundedReceiver<u32> {
    let (sender, receiver) = mpsc::unbounded_channel();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let _ = sender.send(event.id);
        }
    }));
    receiver
}

trait HotkeyRegistrar {
    fn register_hotkey(&self, hotkey: HotKey) -> Result<(), HotkeyRegistrationError>;
    fn unregister_hotkey(&self, hotkey: HotKey) -> Result<(), HotkeyRegistrationError>;
}

impl HotkeyRegistrar for GlobalHotKeyManager {
    fn register_hotkey(&self, hotkey: HotKey) -> Result<(), HotkeyRegistrationError> {
        self.register(hotkey).map_err(HotkeyRegistrationError::from)
    }

    fn unregister_hotkey(&self, hotkey: HotKey) -> Result<(), HotkeyRegistrationError> {
        self.unregister(hotkey)
            .map_err(HotkeyRegistrationError::from)
    }
}

fn replace_registration<R: HotkeyRegistrar>(
    registrar: &R,
    registered: &mut HashMap<Action, HotKey>,
    action: Action,
    new_hotkey: Option<HotKey>,
) -> Result<(), GlobalHotkeyError> {
    let old_hotkey = registered.get(&action).copied();
    if old_hotkey == new_hotkey {
        return Ok(());
    }

    if let Some(old_hotkey) = old_hotkey {
        registrar
            .unregister_hotkey(old_hotkey)
            .map_err(GlobalHotkeyError::lifecycle)?;
    }

    if let Some(new_hotkey) = new_hotkey {
        if let Err(registration_error) = registrar.register_hotkey(new_hotkey) {
            if let Some(old_hotkey) = old_hotkey {
                match registrar.register_hotkey(old_hotkey) {
                    Ok(()) => {
                        registered.insert(action, old_hotkey);
                    }
                    Err(rollback_error) => {
                        registered.remove(&action);
                        return Err(GlobalHotkeyError::rollback(
                            registration_error,
                            rollback_error,
                        ));
                    }
                }
            }

            return Err(GlobalHotkeyError::native_conflict(registration_error));
        }

        registered.insert(action, new_hotkey);
    } else {
        registered.remove(&action);
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalHotkeyErrorKind {
    UnsupportedSession,
    UnsupportedBinding,
    NativeConflict,
    BackendUnavailable,
    Lifecycle,
    Rollback,
}

type BoxError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug)]
pub struct GlobalHotkeyError {
    kind: GlobalHotkeyErrorKind,
    source: Option<BoxError>,
}

impl GlobalHotkeyError {
    pub fn unsupported_session() -> Self {
        Self {
            kind: GlobalHotkeyErrorKind::UnsupportedSession,
            source: None,
        }
    }

    fn unsupported_binding() -> Self {
        Self {
            kind: GlobalHotkeyErrorKind::UnsupportedBinding,
            source: None,
        }
    }

    fn native_conflict(source: HotkeyRegistrationError) -> Self {
        Self {
            kind: GlobalHotkeyErrorKind::NativeConflict,
            source: Some(Box::new(source)),
        }
    }

    fn backend_unavailable<E>(source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind: GlobalHotkeyErrorKind::BackendUnavailable,
            source: Some(Box::new(source)),
        }
    }

    fn lifecycle(source: HotkeyRegistrationError) -> Self {
        Self {
            kind: GlobalHotkeyErrorKind::Lifecycle,
            source: Some(Box::new(source)),
        }
    }

    fn rollback(registration: HotkeyRegistrationError, rollback: HotkeyRegistrationError) -> Self {
        Self {
            kind: GlobalHotkeyErrorKind::Rollback,
            source: Some(Box::new(HotkeyRegistrationError::Rollback {
                registration: Box::new(registration),
                rollback: Box::new(rollback),
            })),
        }
    }

    pub const fn kind(&self) -> GlobalHotkeyErrorKind {
        self.kind
    }

    pub const fn code(&self) -> rustle_domain::error::ErrorCode {
        match self.kind {
            GlobalHotkeyErrorKind::UnsupportedSession
            | GlobalHotkeyErrorKind::UnsupportedBinding => {
                rustle_domain::error::ErrorCode::PlatformUnsupported
            }
            GlobalHotkeyErrorKind::NativeConflict => {
                rustle_domain::error::ErrorCode::PlatformConflict
            }
            GlobalHotkeyErrorKind::BackendUnavailable
            | GlobalHotkeyErrorKind::Lifecycle
            | GlobalHotkeyErrorKind::Rollback => {
                rustle_domain::error::ErrorCode::PlatformUnavailable
            }
        }
    }
}

impl fmt::Display for GlobalHotkeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let summary = match self.kind {
            GlobalHotkeyErrorKind::UnsupportedSession => {
                "Global shortcuts are not supported in the current desktop session"
            }
            GlobalHotkeyErrorKind::UnsupportedBinding => {
                "This key combination cannot be registered as a global shortcut"
            }
            GlobalHotkeyErrorKind::NativeConflict => "This global shortcut is already in use",
            GlobalHotkeyErrorKind::BackendUnavailable => {
                "The global shortcut backend is unavailable"
            }
            GlobalHotkeyErrorKind::Lifecycle => "Rustle could not update the global shortcut",
            GlobalHotkeyErrorKind::Rollback => {
                "Rustle could not restore the previous global shortcut"
            }
        };
        formatter.write_str(summary)
    }
}

impl Error for GlobalHotkeyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl From<GlobalHotkeyError> for rustle_application::error::AppError {
    fn from(error: GlobalHotkeyError) -> Self {
        let code = error.code();
        let summary = error.to_string();
        rustle_application::error::AppError::with_source(code, summary, error)
    }
}

#[derive(Debug, thiserror::Error)]
enum HotkeyRegistrationError {
    #[error("native global hotkey operation failed")]
    Native(#[source] global_hotkey::Error),
    #[cfg(test)]
    #[error("simulated global hotkey operation failed")]
    Simulated(#[source] std::io::Error),
    #[error("global hotkey registration and rollback both failed")]
    Rollback {
        registration: Box<Self>,
        #[source]
        rollback: Box<Self>,
    },
}

impl From<global_hotkey::Error> for HotkeyRegistrationError {
    fn from(error: global_hotkey::Error) -> Self {
        Self::Native(error)
    }
}

#[cfg(test)]
impl HotkeyRegistrationError {
    fn simulated(message: impl Into<String>) -> Self {
        Self::Simulated(std::io::Error::other(message.into()))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use super::*;
    use rustle_domain::keybindings::ModifierSet;

    #[test]
    fn converts_keys_and_arbitrary_modifier_combinations() {
        let binding = KeyBinding {
            modifiers: ModifierSet {
                cmd: true,
                shift: true,
                ..ModifierSet::default()
            },
            key: KeyCode::Space,
        };

        let hotkey = hotkey_for_binding(&binding).unwrap();
        assert_eq!(hotkey.key, Code::Space);
        assert!(hotkey.mods.contains(Modifiers::SUPER));
        assert!(hotkey.mods.contains(Modifiers::SHIFT));
        assert!(!hotkey.mods.contains(Modifiers::CONTROL));

        let unmodified = hotkey_for_binding(&KeyBinding::new(KeyCode::F8)).unwrap();
        assert!(unmodified.mods.is_empty());
        assert_eq!(unmodified.key, Code::F8);
    }

    #[derive(Default)]
    struct FakeRegistrar {
        active: RefCell<HashSet<HotKey>>,
        fail_registration_for: RefCell<Option<HotKey>>,
    }

    impl HotkeyRegistrar for FakeRegistrar {
        fn register_hotkey(&self, hotkey: HotKey) -> Result<(), HotkeyRegistrationError> {
            if *self.fail_registration_for.borrow() == Some(hotkey) {
                return Err(HotkeyRegistrationError::simulated(
                    "reserved by another application",
                ));
            }
            self.active.borrow_mut().insert(hotkey);
            Ok(())
        }

        fn unregister_hotkey(&self, hotkey: HotKey) -> Result<(), HotkeyRegistrationError> {
            self.active.borrow_mut().remove(&hotkey);
            Ok(())
        }
    }

    #[test]
    fn failed_replacement_restores_previous_working_registration() {
        let registrar = FakeRegistrar::default();
        let old_hotkey = HotKey::new(Some(Modifiers::CONTROL), Code::Space);
        let rejected_hotkey = HotKey::new(None, Code::KeyQ);
        registrar.active.borrow_mut().insert(old_hotkey);
        *registrar.fail_registration_for.borrow_mut() = Some(rejected_hotkey);

        let mut registered = HashMap::from([(Action::PlayPause, old_hotkey)]);
        let error = replace_registration(
            &registrar,
            &mut registered,
            Action::PlayPause,
            Some(rejected_hotkey),
        )
        .unwrap_err();

        assert_eq!(error.kind(), GlobalHotkeyErrorKind::NativeConflict);
        assert_eq!(
            error.code(),
            rustle_domain::error::ErrorCode::PlatformConflict
        );
        assert_eq!(
            error
                .source()
                .and_then(Error::source)
                .expect("native source")
                .to_string(),
            "reserved by another application"
        );
        assert_eq!(registered.get(&Action::PlayPause), Some(&old_hotkey));
        assert!(registrar.active.borrow().contains(&old_hotkey));
        assert!(!registrar.active.borrow().contains(&rejected_hotkey));
    }

    #[test]
    fn successful_replacement_updates_the_active_registration() {
        let registrar = FakeRegistrar::default();
        let old_hotkey = HotKey::new(Some(Modifiers::CONTROL), Code::Space);
        let new_hotkey = HotKey::new(Some(Modifiers::ALT), Code::KeyQ);
        registrar.active.borrow_mut().insert(old_hotkey);

        let mut registered = HashMap::from([(Action::PlayPause, old_hotkey)]);
        replace_registration(
            &registrar,
            &mut registered,
            Action::PlayPause,
            Some(new_hotkey),
        )
        .unwrap();

        assert_eq!(registered.get(&Action::PlayPause), Some(&new_hotkey));
        assert!(!registrar.active.borrow().contains(&old_hotkey));
        assert!(registrar.active.borrow().contains(&new_hotkey));
    }

    #[test]
    fn clearing_removes_the_active_registration() {
        let registrar = FakeRegistrar::default();
        let old_hotkey = HotKey::new(Some(Modifiers::CONTROL), Code::Space);
        registrar.active.borrow_mut().insert(old_hotkey);

        let mut registered = HashMap::from([(Action::PlayPause, old_hotkey)]);
        replace_registration(&registrar, &mut registered, Action::PlayPause, None).unwrap();

        assert!(!registered.contains_key(&Action::PlayPause));
        assert!(!registrar.active.borrow().contains(&old_hotkey));
    }

    #[test]
    fn registered_hotkey_id_resolves_to_action() {
        let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
        let registered = HashMap::from([(Action::PlayPause, hotkey)]);

        assert_eq!(
            action_for_registered_id(&registered, hotkey.id()),
            Some(Action::PlayPause)
        );
        assert_eq!(action_for_registered_id(&registered, u32::MAX), None);
    }

    #[test]
    fn detects_wayland_without_rejecting_an_explicit_x11_session() {
        assert!(session_is_wayland("wayland", false));
        assert!(session_is_wayland("", true));
        assert!(!session_is_wayland("x11", true));
        assert!(!session_is_wayland("x11", false));
    }

    #[test]
    fn platform_error_kinds_map_to_stable_application_codes() {
        assert_eq!(
            GlobalHotkeyError::unsupported_session().code(),
            rustle_domain::error::ErrorCode::PlatformUnsupported
        );
        assert_eq!(
            GlobalHotkeyError::native_conflict(HotkeyRegistrationError::simulated("conflict"))
                .code(),
            rustle_domain::error::ErrorCode::PlatformConflict
        );
        assert_eq!(
            GlobalHotkeyError::backend_unavailable(std::io::Error::other("backend")).code(),
            rustle_domain::error::ErrorCode::PlatformUnavailable
        );
        assert_eq!(
            GlobalHotkeyError::lifecycle(HotkeyRegistrationError::simulated("lifecycle")).code(),
            rustle_domain::error::ErrorCode::PlatformUnavailable
        );
        assert_eq!(
            GlobalHotkeyError::rollback(
                HotkeyRegistrationError::simulated("register"),
                HotkeyRegistrationError::simulated("rollback"),
            )
            .code(),
            rustle_domain::error::ErrorCode::PlatformUnavailable
        );
    }
}
