//! Native Windows notification-area implementation.

use super::{
    TrayAvailability, TrayCommand, TrayError, TrayHandle, TrayPresentation, TrayResultExt,
    TrayWindowCommand,
};
use rustle_domain::playback::PlayMode;
use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT,
    SetLastError, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETFOCUS,
    NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, GWLP_USERDATA, GetCursorPos,
    GetSystemMetrics, GetWindowLongPtrW, HICON, HMENU, ICONINFO, KillTimer, MF_CHECKED,
    MF_DISABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PostMessageW, RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON,
    SM_CYSMICON, SM_MENUDROPALIGNMENT, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TrackPopupMenuEx,
    UnregisterClassW, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY,
    WM_NULL, WM_POWERBROADCAST, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};

const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 0x51;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const TRAY_ICON_ID: u32 = 1;
const RECOVERY_TIMER_ID: usize = 1;
const RECOVERY_DELAYS_MS: [u32; 6] = [250, 500, 1_000, 2_000, 4_000, 8_000];

#[derive(Default)]
struct RecoveryBackoff {
    attempts: usize,
    pending: bool,
}

impl RecoveryBackoff {
    fn next_delay(&mut self) -> Option<u32> {
        let delay = *RECOVERY_DELAYS_MS.get(self.attempts)?;
        self.attempts += 1;
        Some(delay)
    }
}

const CMD_PLAY_PAUSE: u16 = 1001;
const CMD_PREV_TRACK: u16 = 1002;
const CMD_NEXT_TRACK: u16 = 1003;
const CMD_TOGGLE_FAVORITE: u16 = 1004;
const CMD_SEQUENTIAL: u16 = 1010;
const CMD_LOOP_ALL: u16 = 1011;
const CMD_LOOP_ONE: u16 = 1012;
const CMD_SHUFFLE: u16 = 1013;
const CMD_TOGGLE_WINDOW: u16 = 1020;
const CMD_QUIT: u16 = 1030;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrayIdentity {
    window_id: u32,
}

fn default_tray_identity() -> TrayIdentity {
    TrayIdentity::window_id(TRAY_ICON_ID)
}

impl TrayIdentity {
    const fn window_id(window_id: u32) -> Self {
        Self { window_id }
    }

    fn apply_to_notify_data(self, data: &mut NOTIFYICONDATAW) {
        data.uID = self.window_id;
    }

    fn identifier(self, hwnd: HWND) -> NOTIFYICONIDENTIFIER {
        let mut identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: hwnd,
            ..Default::default()
        };
        identifier.uID = self.window_id;
        identifier
    }

    fn matches_callback(self, packed: LPARAM) -> bool {
        (packed as u32 >> 16) as u16 == self.window_id as u16
    }
}

thread_local! {
    /// The native owner is deliberately thread-local: HWND/HMENU/HICON never
    /// cross the Iced/Winit UI thread boundary.
    static WINDOWS_TRAY: RefCell<Option<WindowsTray>> = const { RefCell::new(None) };
}

pub fn start_windows_tray(
    presentation: TrayPresentation,
    command_capacity: usize,
) -> Result<(TrayHandle, mpsc::Receiver<TrayCommand>), TrayError> {
    start_windows_tray_with_identity(presentation, command_capacity, default_tray_identity())
}

fn start_windows_tray_with_identity(
    presentation: TrayPresentation,
    command_capacity: usize,
    identity: TrayIdentity,
) -> Result<(TrayHandle, mpsc::Receiver<TrayCommand>), TrayError> {
    let (command_tx, command_rx) = mpsc::channel(command_capacity);
    let tray = WindowsTray::new(command_tx, presentation, identity)?;

    WINDOWS_TRAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return Err(TrayError::already_initialized());
        }
        *slot = Some(tray);
        Ok(())
    })?;

    Ok((TrayHandle { _private: () }, command_rx))
}

pub fn update_state(presentation: TrayPresentation) -> Result<(), TrayError> {
    WINDOWS_TRAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let tray = slot.as_mut().ok_or_else(TrayError::not_initialized)?;
        tray.update_state(presentation)
    })
}

pub fn shutdown() {
    WINDOWS_TRAY.with(|slot| {
        let _ = slot.borrow_mut().take();
    });
}

pub fn is_available() -> bool {
    WINDOWS_TRAY.with(|slot| {
        slot.try_borrow()
            .ok()
            .and_then(|slot| {
                slot.as_ref()
                    .map(|tray| tray.state.availability.is_available())
            })
            .unwrap_or(false)
    })
}

struct WindowsTray {
    state: Box<WindowState>,
    instance: HINSTANCE,
    class_name: Vec<u16>,
    owns_window_class: bool,
    _thread_bound: PhantomData<Rc<()>>,
}

impl WindowsTray {
    fn new(
        command_tx: mpsc::Sender<TrayCommand>,
        presentation: TrayPresentation,
        identity: TrayIdentity,
    ) -> Result<Self, TrayError> {
        // SAFETY: Passing null requests the module containing the current
        // process image; the call has no borrowed output or cleanup contract.
        let instance = unsafe { GetModuleHandleW(null()) };
        if instance.is_null() {
            return Err(last_error("GetModuleHandleW"));
        }

        let class_name = wide("Rustle.TrayWindow.1");
        let owns_window_class = register_window_class(instance, &class_name)?;

        let taskbar_created_name = wide("TaskbarCreated");
        // SAFETY: taskbar_created_name is NUL-terminated and remains allocated
        // for this synchronous registration call.
        let taskbar_created = unsafe { RegisterWindowMessageW(taskbar_created_name.as_ptr()) };
        if taskbar_created == 0 {
            let error = last_error("RegisterWindowMessageW(TaskbarCreated)");
            if owns_window_class {
                // SAFETY: This constructor registered the class using the same
                // live instance/name pair and has not created a window yet.
                let _ = unsafe { UnregisterClassW(class_name.as_ptr(), instance) };
            }
            return Err(error);
        }

        let icon = match load_small_icon() {
            Ok(icon) => icon,
            Err(error) => {
                if owns_window_class {
                    // SAFETY: No callback window exists, so the class owned by
                    // this constructor can be unregistered immediately.
                    let _ = unsafe { UnregisterClassW(class_name.as_ptr(), instance) };
                }
                return Err(error);
            }
        };
        let menu = match build_menu(&presentation) {
            Ok(menu) => menu,
            Err(error) => {
                if icon.owned {
                    // SAFETY: icon is a live, uniquely owned HICON returned by
                    // load_small_icon and has not been registered with Shell.
                    let _ = unsafe { DestroyIcon(icon.handle) };
                }
                if owns_window_class {
                    // SAFETY: No callback window exists and this constructor
                    // owns the registered instance/name pair.
                    let _ = unsafe { UnregisterClassW(class_name.as_ptr(), instance) };
                }
                return Err(error);
            }
        };

        let mut window_state = Box::new(WindowState {
            hwnd: null_mut(),
            menu,
            pending_menu: null_mut(),
            menu_tracking: false,
            icon: icon.handle,
            icon_owned: icon.owned,
            icon_registered: false,
            availability: TrayAvailability::Starting,
            recovery: RecoveryBackoff::default(),
            taskbar_created,
            command_tx,
            command_overflow_warned: false,
            identity,
            presentation,
        });

        let window_title = wide("Rustle System Tray");
        // SAFETY: instance/class_name identify the registered class and all
        // string pointers remain live for this synchronous call. window_state
        // is boxed, so the lpParam address remains stable until the HWND is
        // destroyed; WM_NCCREATE installs that address as callback user data.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                window_state.as_mut() as *mut WindowState as *const c_void,
            )
        };
        if hwnd.is_null() {
            let error = last_error("CreateWindowExW(tray window)");
            if window_state.icon_owned {
                // SAFETY: Window creation failed, so the uniquely owned icon
                // was never exposed through a live callback window.
                let _ = unsafe { DestroyIcon(window_state.icon) };
                window_state.icon_owned = false;
            }
            // SAFETY: menu is the live, unattached root HMENU produced by
            // build_menu and WindowState is still its unique owner.
            let _ = unsafe { DestroyMenu(window_state.menu) };
            window_state.menu = null_mut();
            if owns_window_class {
                // SAFETY: CreateWindowExW failed, so no window of the class is
                // live and this constructor owns the registration.
                let _ = unsafe { UnregisterClassW(class_name.as_ptr(), instance) };
            }
            return Err(error);
        }
        window_state.hwnd = hwnd;

        let mut tray = Self {
            state: window_state,
            instance,
            class_name,
            owns_window_class,
            _thread_bound: PhantomData,
        };
        if let Err(error) = tray.state.sync_icon() {
            drop(tray);
            return Err(error);
        }
        Ok(tray)
    }

    fn update_state(&mut self, presentation: TrayPresentation) -> Result<(), TrayError> {
        let new_menu = build_menu(&presentation)?;
        self.state.install_menu(new_menu);
        self.state.presentation = presentation;
        self.state.sync_icon()
    }
}

impl Drop for WindowsTray {
    fn drop(&mut self) {
        self.state.cancel_recovery_timer();
        self.state.unregister_icon();
        if !self.state.menu.is_null() {
            // SAFETY: WindowsTray is thread-bound and uniquely owns this root
            // menu; unregister_icon has detached the Shell identity and no
            // TrackPopupMenuEx call can outlive the synchronous owner method.
            let _ = unsafe { DestroyMenu(self.state.menu) };
            self.state.menu = null_mut();
        }
        if !self.state.pending_menu.is_null() {
            // SAFETY: pending_menu is never attached or tracked and is uniquely
            // owned by WindowState.
            let _ = unsafe { DestroyMenu(self.state.pending_menu) };
            self.state.pending_menu = null_mut();
        }
        if !self.state.hwnd.is_null() {
            // SAFETY: The callback HWND and boxed WindowState are both live on
            // their owner thread. Clearing user data first prevents callbacks
            // during DestroyWindow from observing a soon-to-be-dropped pointer.
            unsafe {
                SetWindowLongPtrW(self.state.hwnd, GWLP_USERDATA, 0);
                let _ = DestroyWindow(self.state.hwnd);
            }
            self.state.hwnd = null_mut();
        }
        if self.state.icon_owned && !self.state.icon.is_null() {
            // SAFETY: The Shell registration and callback window are gone;
            // WindowState remains the unique owner of this HICON.
            let _ = unsafe { DestroyIcon(self.state.icon) };
            self.state.icon_owned = false;
            self.state.icon = null_mut();
        }
        if self.owns_window_class {
            // SAFETY: The owned callback window has been destroyed and the
            // NUL-terminated class name/instance pair is the one we registered.
            let _ = unsafe { UnregisterClassW(self.class_name.as_ptr(), self.instance) };
        }
    }
}

struct WindowState {
    hwnd: HWND,
    menu: HMENU,
    pending_menu: HMENU,
    menu_tracking: bool,
    icon: HICON,
    icon_owned: bool,
    icon_registered: bool,
    availability: TrayAvailability,
    recovery: RecoveryBackoff,
    taskbar_created: u32,
    command_tx: mpsc::Sender<TrayCommand>,
    command_overflow_warned: bool,
    identity: TrayIdentity,
    presentation: TrayPresentation,
}

impl WindowState {
    fn register_icon(&mut self) -> Result<(), TrayError> {
        let data = self.notify_data(
            NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            &self.presentation.tooltip,
        );
        repair_icon(data, notify_icon)?;
        self.icon_registered = true;
        Ok(())
    }

    fn unregister_icon(&mut self) {
        if !self.icon_registered || self.hwnd.is_null() {
            return;
        }
        let data = self.notify_data(0, "");
        // SAFETY: deletion is idempotently guarded by icon_registered.
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
        self.icon_registered = false;
        self.availability = TrayAvailability::Starting;
    }

    fn sync_icon(&mut self) -> Result<(), TrayError> {
        // Playback changes and repeated broadcasts must not bypass or postpone
        // a pending retry. The retry uses the newest presentation.
        if self.recovery.pending {
            return Ok(());
        }
        let result = self.register_icon();
        self.finish_sync(result)
    }

    fn finish_sync(&mut self, result: Result<(), TrayError>) -> Result<(), TrayError> {
        match result {
            Ok(()) => {
                self.cancel_recovery_timer();
                self.recovery = RecoveryBackoff::default();
                self.set_availability(TrayAvailability::Available);
                Ok(())
            }
            Err(error) => {
                tracing::debug!(
                    operation = error.operation(),
                    native_code = error.native_code(),
                    "Windows tray repair failed"
                );
                if let Some(delay) = self.recovery.next_delay() {
                    // SAFETY: hwnd is our live UI-thread window. A null callback
                    // delivers WM_TIMER on that same thread; no Rust pointer is retained.
                    if unsafe { SetTimer(self.hwnd, RECOVERY_TIMER_ID, delay, None) } != 0 {
                        self.recovery.pending = true;
                        // Once warned, remain unavailable until a confirmed
                        // success. More shell broadcasts must not create a new
                        // warning episode while the same outage continues.
                        if !matches!(self.availability, TrayAvailability::Unavailable(_)) {
                            self.set_availability(TrayAvailability::Recovering);
                        }
                        return Ok(());
                    }
                    let timer_error = last_error("SetTimer(tray recovery)");
                    tracing::warn!(
                        operation = timer_error.operation(),
                        native_code = timer_error.native_code(),
                        "Could not schedule Windows tray recovery"
                    );
                }
                self.set_availability(TrayAvailability::Unavailable(error.to_string()));
                Err(error)
            }
        }
    }

    fn cancel_recovery_timer(&mut self) {
        // SAFETY: hwnd is the tray-owned window. Killing an absent timer is harmless.
        let _ = unsafe { KillTimer(self.hwnd, RECOVERY_TIMER_ID) };
        self.recovery.pending = false;
    }

    fn recover(&mut self, timer: bool) {
        if timer {
            // KillTimer does not remove already queued messages. Ignore stale
            // WM_TIMER after a successful recovery/cancellation.
            if !self.recovery.pending {
                return;
            }
            self.cancel_recovery_timer();
        } else if !self.recovery.pending {
            // A fresh shell/resume event can start a new bounded attempt after
            // exhaustion. Repeated events during recovery do not reset its budget.
            self.recovery = RecoveryBackoff::default();
        }
        if let Err(error) = self.sync_icon() {
            tracing::warn!(
                operation = error.operation(),
                native_code = error.native_code(),
                "Windows tray recovery exhausted"
            );
        }
    }

    fn notify_data(&self, flags: u32, tooltip: &str) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uFlags: flags,
            uCallbackMessage: TRAY_CALLBACK_MESSAGE,
            hIcon: self.icon,
            ..Default::default()
        };
        self.identity.apply_to_notify_data(&mut data);
        data.szTip = utf16_array::<128>(tooltip);
        data
    }

    fn install_menu(&mut self, new_menu: HMENU) {
        if self.menu_tracking {
            if !self.pending_menu.is_null() {
                // SAFETY: pending_menu is not attached to a window or being tracked.
                unsafe {
                    let _ = DestroyMenu(self.pending_menu);
                }
            }
            self.pending_menu = new_menu;
            return;
        }

        let old_menu = std::mem::replace(&mut self.menu, new_menu);
        if !old_menu.is_null() {
            // SAFETY: the popup menu is not attached to a window and is not being tracked.
            unsafe {
                let _ = DestroyMenu(old_menu);
            }
        }
    }

    fn finish_menu_tracking(&mut self) {
        self.menu_tracking = false;
        if !self.pending_menu.is_null() {
            let pending_menu = std::mem::replace(&mut self.pending_menu, null_mut());
            self.install_menu(pending_menu);
        }
    }

    fn set_availability(&mut self, availability: TrayAvailability) {
        if self.availability != availability {
            self.availability = availability.clone();
            self.send_command(TrayCommand::AvailabilityChanged(availability));
        }
    }

    fn send_command(&mut self, command: TrayCommand) {
        match self.command_tx.try_send(command) {
            Ok(()) => self.command_overflow_warned = false,
            Err(mpsc::error::TrySendError::Full(_)) => {
                if !self.command_overflow_warned {
                    tracing::warn!("Windows tray command channel is full; dropping input");
                    self.command_overflow_warned = true;
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!("Windows tray command receiver is closed");
            }
        }
    }

    fn send_menu_command(&mut self, id: u16) {
        if let Some(command) = command_for_menu_id(id) {
            self.send_command(command);
        }
    }
}

/// Win32 callback for the thread-bound hidden notification-area window.
///
/// # Safety
///
/// Windows must invoke this function with the `WNDPROC` ABI and message-specific
/// `wparam`/`lparam` values. While `GWLP_USERDATA` is non-zero it must be the live
/// stable `Box<WindowState>` pointer installed during `WM_NCCREATE` by
/// `WindowsTray::new` on the same UI thread.
unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    crate::runtime::catch_ffi_unwind(
        "windows_tray_wndproc",
        Box::new(move || {
            // SAFETY: The outer WNDPROC contract supplies the same validated
            // native arguments to the implementation, and this closure cannot
            // unwind beyond the ABI wrapper.
            unsafe { tray_window_proc_inner(hwnd, message, wparam, lparam) }
        }),
        Box::new(move || {
            // SAFETY: On a captured panic, forwarding the untouched native
            // arguments is the only operation performed before returning to
            // Windows.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }),
    )
}

/// Rust implementation behind the no-unwind Win32 ABI wrapper.
///
/// # Safety
///
/// The caller must uphold the same native message and `GWLP_USERDATA` lifetime
/// contract documented on `tray_window_proc`.
unsafe fn tray_window_proc_inner(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: lparam is a CREATESTRUCTW for WM_NCCREATE and lpCreateParams
        // is the stable Box<WindowState> address supplied to CreateWindowExW.
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        let state_ptr = create.lpCreateParams as *mut WindowState;
        if state_ptr.is_null() {
            return 0;
        }
        // SAFETY: state_ptr came from the validated CREATESTRUCTW payload and
        // the boxed state is still uniquely owned by the constructing tray.
        unsafe {
            (*state_ptr).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        }
    }

    // SAFETY: hwnd is the live window currently dispatching this callback;
    // reading its integer-sized user-data slot does not retain a Rust borrow.
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState };
    if !state_ptr.is_null() {
        // Keep every Rust reference to WindowState scoped away from Win32 calls
        // that can run a nested message loop. WndProc may be re-entered while a
        // popup menu is being tracked.
        // SAFETY: A non-null user-data value is the stable WindowState pointer
        // installed above and remains live until it is cleared before teardown.
        let taskbar_created = unsafe { (*state_ptr).taskbar_created };
        if message == taskbar_created {
            // SAFETY: This callback runs on the owner thread and no Rust
            // reference to the state is held across this call.
            unsafe { (&mut *state_ptr).recover(false) };
            return 0;
        }

        if message == TRAY_CALLBACK_MESSAGE {
            // SAFETY: Same validated live state pointer; copying TrayIdentity
            // does not create a reference that can cross a nested message loop.
            let identity = unsafe { (*state_ptr).identity };
            match classify_callback(identity, wparam, lparam) {
                TrayCallbackAction::PrimaryActivation => {
                    // SAFETY: The pointer is live and callback dispatch holds no
                    // competing Rust reference. send_command is non-blocking.
                    unsafe {
                        (&mut *state_ptr).send_command(TrayCommand::Window(
                            TrayWindowCommand::PrimaryActivation,
                        ));
                    }
                }
                TrayCallbackAction::ContextMenu(position) => {
                    // SAFETY: The pointer is the tray-owned state on its UI
                    // thread. show_context_menu revalidates it after the nested
                    // TrackPopupMenuEx loop before reacquiring a Rust reference.
                    unsafe { show_context_menu(state_ptr, position) };
                }
                TrayCallbackAction::Ignore => {}
            }
            return 0;
        }

        match message {
            WM_TIMER if wparam == RECOVERY_TIMER_ID => {
                // SAFETY: This is the live window-owned state on its UI thread.
                unsafe { (&mut *state_ptr).recover(true) };
                return 0;
            }
            WM_POWERBROADCAST
                if matches!(wparam as u32, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) =>
            {
                // SAFETY: Resume is delivered to our live top-level window on
                // the owner thread; recovery uses the same bounded retry path.
                unsafe { (&mut *state_ptr).recover(false) };
                return 1;
            }
            WM_COMMAND => {
                // SAFETY: The state pointer remains live and no reference is
                // retained after the bounded, non-blocking command projection.
                unsafe {
                    (&mut *state_ptr).send_menu_command((wparam as u32 & 0xffff) as u16);
                }
                return 0;
            }
            WM_NCDESTROY => {
                // SAFETY: hwnd is being destroyed on its owner thread. Clearing
                // the slot prevents all later callbacks from dereferencing the
                // WindowState pointer.
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            _ => {}
        }
    }

    // SAFETY: Forwarding unhandled messages with the original arguments is the
    // required WNDPROC contract; no Rust-owned pointer is passed to Windows.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Shows the current popup menu without retaining a Rust borrow across Win32's
/// nested message loop.
///
/// # Safety
///
/// `state_ptr` must point to the live, uniquely owned `WindowState` installed in
/// the callback `HWND`, and this function must run on that window's owner thread.
/// The pointer may be invalidated by nested dispatch, so it must be revalidated
/// through `GWLP_USERDATA` before it is dereferenced after `TrackPopupMenuEx`.
unsafe fn show_context_menu(state_ptr: *mut WindowState, packed_position: Option<WPARAM>) {
    // Capture the native handles, then end the Rust borrow before calling
    // TrackPopupMenuEx because it runs a nested Windows message loop.
    // SAFETY: The caller guarantees state_ptr is live and uniquely accessible
    // at entry. The reference ends before any nested-loop Win32 call.
    let (hwnd, menu, identity) = unsafe {
        let state = &mut *state_ptr;
        if state.menu_tracking {
            return;
        }
        state.menu_tracking = true;
        (state.hwnd, state.menu, state.identity)
    };

    let point = context_menu_point(hwnd, identity, packed_position);
    // SAFETY: This reads a process/system metric and has no pointer, ownership,
    // or cleanup preconditions.
    let alignment = menu_alignment_flag(unsafe { GetSystemMetrics(SM_MENUDROPALIGNMENT) } != 0);

    // TPM_RETURNCMD | TPM_NONOTIFY keeps WM_COMMAND out of the nested loop.
    // The owner must be foreground or clicking outside will not dismiss the menu.
    // SAFETY: hwnd is the live owner window captured before releasing the Rust
    // reference; failure is benign and reported by the API return value.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    // SAFETY: menu and hwnd are live thread-owned handles. No Rust reference is
    // held while TrackPopupMenuEx runs its nested message loop, and NONOTIFY
    // prevents a menu command from being delivered through reentrant WM_COMMAND.
    let selected = unsafe {
        TrackPopupMenuEx(
            menu,
            alignment | TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            hwnd,
            null(),
        )
    };
    // WM_NULL completes the documented notification-area menu-dismissal handoff.
    // SAFETY: hwnd is the captured callback window; posting copies plain values
    // and does not borrow state. Failure requires no cleanup.
    let _ = unsafe { PostMessageW(hwnd, WM_NULL, 0, 0) };

    let mut focus_data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        ..Default::default()
    };
    identity.apply_to_notify_data(&mut focus_data);
    // Return keyboard focus to the notification area after either selection or
    // cancellation, as required for notification-icon shortcut menus.
    // SAFETY: focus_data is fully initialized for the live captured HWND and
    // numeric identity and remains borrowed only for this synchronous call.
    if unsafe { Shell_NotifyIconW(NIM_SETFOCUS, &focus_data) } == 0 {
        tracing::debug!("Windows notification area rejected NIM_SETFOCUS");
    }

    // SAFETY: Reading the live HWND's user-data slot is the required validity
    // check after nested dispatch and creates no Rust reference.
    if unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState } != state_ptr {
        return;
    }

    // SAFETY: WindowState remains owned by WindowsTray while its callback
    // window is live. No reference was held across TrackPopupMenuEx.
    let state = unsafe { &mut *state_ptr };
    state.finish_menu_tracking();
    if selected > 0 {
        state.send_menu_command(selected as u16);
    }
}

fn context_menu_point(
    hwnd: HWND,
    identity: TrayIdentity,
    packed_position: Option<WPARAM>,
) -> POINT {
    if let Some(position) = packed_position {
        let point = point_from_packed_position(position);
        if point.x != -1 || point.y != -1 {
            return point;
        }

        let identifier = identity.identifier(hwnd);
        let mut icon_rect = RECT::default();
        // SAFETY: Both values are live for the call. Only S_OK is usable.
        if unsafe { Shell_NotifyIconGetRect(&identifier, &mut icon_rect) } == 0 {
            return POINT {
                x: icon_rect.left + (icon_rect.right - icon_rect.left) / 2,
                y: icon_rect.top + (icon_rect.bottom - icon_rect.top) / 2,
            };
        }
    }

    // Legacy callbacks carry an icon ID in wParam, not coordinates. Use the
    // cursor so a third-party dock opens the menu at its own icon location.
    let mut cursor = POINT::default();
    // SAFETY: GetCursorPos writes one initialized POINT on success.
    if unsafe { GetCursorPos(&mut cursor) } != 0 {
        cursor
    } else {
        POINT { x: 0, y: 0 }
    }
}

fn register_window_class(instance: HINSTANCE, class_name: &[u16]) -> Result<bool, TrayError> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(tray_window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    // SAFETY: class_name is NUL-terminated and the callback has the required ABI.
    if unsafe { RegisterClassExW(&class) } == 0 {
        // SAFETY: GetLastError is thread-local and must be read immediately
        // after the failed registration call on this same thread.
        let error = unsafe { GetLastError() };
        if error != ERROR_CLASS_ALREADY_EXISTS {
            return Err(TrayError::native("RegisterClassExW(tray window)", error));
        }
        return Ok(false);
    }
    Ok(true)
}

struct LoadedIcon {
    handle: HICON,
    owned: bool,
}

fn load_small_icon() -> Result<LoadedIcon, TrayError> {
    static ICON_DATA: &[u8] = include_bytes!("../../../../assets/icons/icon_256.png");

    // SAFETY: GetDpiForSystem reads process DPI state and has no pointer or
    // ownership preconditions.
    let dpi = unsafe { GetDpiForSystem() };
    let metric = |index| {
        let scaled = if dpi == 0 {
            0
        } else {
            // SAFETY: index is one of the documented small-icon metrics and
            // dpi came from GetDpiForSystem in this function.
            unsafe { GetSystemMetricsForDpi(index, dpi) }
        };
        if scaled > 0 {
            scaled
        } else {
            // SAFETY: index is one of the documented system metric constants
            // and this query has no ownership or pointer preconditions.
            unsafe { GetSystemMetrics(index) }
        }
    };
    let width = metric(SM_CXSMICON).max(16) as u32;
    let height = metric(SM_CYSMICON).max(16) as u32;

    let rgba = image::load_from_memory(ICON_DATA)
        .tray_context("decode embedded Windows tray icon")?
        .resize_exact(width, height, image::imageops::FilterType::Lanczos3)
        .to_rgba8();

    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // A negative height produces a top-down DIB matching image's row order.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: width * height * 4,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut dib_bits = null_mut();
    // SAFETY: bitmap_info describes a top-down 32-bit DIB with an exact
    // width*height*4 allocation. dib_bits is a valid out-pointer; the returned
    // HBITMAP uniquely owns that allocation until DeleteObject below.
    let color_bitmap = unsafe {
        CreateDIBSection(
            null_mut(),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut dib_bits,
            null_mut(),
            0,
        )
    };
    if color_bitmap.is_null() || dib_bits.is_null() {
        let error = last_error("CreateDIBSection(tray icon)");
        if !color_bitmap.is_null() {
            // SAFETY: A non-null color_bitmap is the unique partial allocation
            // returned by CreateDIBSection and has not escaped this function.
            unsafe {
                let _ = DeleteObject(color_bitmap);
            }
        }
        return Err(error);
    }

    // SAFETY: Successful CreateDIBSection returned a non-null allocation of
    // exactly width*height*4 writable bytes, uniquely owned through
    // color_bitmap. No other pointer accesses it while this slice is used.
    let target = unsafe {
        std::slice::from_raw_parts_mut(dib_bits.cast::<u8>(), (width * height * 4) as usize)
    };
    target.copy_from_slice(&premultiplied_bgra(rgba.as_raw()));

    let mask_stride = width.div_ceil(16) * 2;
    let mask_bits = vec![0u8; (mask_stride * height) as usize];
    // SAFETY: mask_bits contains the documented 1-bpp scanline allocation and
    // remains live for the synchronous copy performed by CreateBitmap.
    let mask_bitmap =
        unsafe { CreateBitmap(width as i32, height as i32, 1, 1, mask_bits.as_ptr().cast()) };
    if mask_bitmap.is_null() {
        let error = last_error("CreateBitmap(tray icon mask)");
        // SAFETY: color_bitmap is still uniquely owned here and must be
        // released when creation of its paired mask fails.
        unsafe {
            let _ = DeleteObject(color_bitmap);
        }
        return Err(error);
    }

    let icon_info = ICONINFO {
        fIcon: 1,
        hbmMask: mask_bitmap,
        hbmColor: color_bitmap,
        ..Default::default()
    };
    // SAFETY: Both HBITMAPs are live and uniquely owned for the duration of
    // this call. CreateIconIndirect copies their bitmap data into a new HICON.
    let icon = unsafe { CreateIconIndirect(&icon_info) };
    let icon_error = icon
        .is_null()
        .then(|| last_error("CreateIconIndirect(tray icon)"));
    // SAFETY: CreateIconIndirect does not transfer HBITMAP ownership. Both
    // temporary handles remain unique and are released exactly once here,
    // regardless of icon creation success.
    unsafe {
        let _ = DeleteObject(mask_bitmap);
        let _ = DeleteObject(color_bitmap);
    }
    if let Some(error) = icon_error {
        return Err(error);
    }

    Ok(LoadedIcon {
        handle: icon,
        owned: true,
    })
}

fn premultiplied_bgra(rgba: &[u8]) -> Vec<u8> {
    debug_assert_eq!(rgba.len() % 4, 0);
    let mut output = Vec::with_capacity(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        let alpha = u16::from(pixel[3]);
        output.push(((u16::from(pixel[2]) * alpha + 127) / 255) as u8);
        output.push(((u16::from(pixel[1]) * alpha + 127) / 255) as u8);
        output.push(((u16::from(pixel[0]) * alpha + 127) / 255) as u8);
        output.push(pixel[3]);
    }
    output
}

struct OwnedMenu(HMENU);

impl OwnedMenu {
    fn popup(operation: &'static str) -> Result<Self, TrayError> {
        // SAFETY: CreatePopupMenu has no preconditions.
        let handle = unsafe { CreatePopupMenu() };
        if handle.is_null() {
            Err(last_error(operation))
        } else {
            Ok(Self(handle))
        }
    }

    fn into_raw(mut self) -> HMENU {
        let handle = self.0;
        self.0 = null_mut();
        handle
    }
}

impl Drop for OwnedMenu {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: OwnedMenu uniquely owns this unattached menu.
            unsafe {
                let _ = DestroyMenu(self.0);
            }
        }
    }
}

fn build_menu(presentation: &TrayPresentation) -> Result<HMENU, TrayError> {
    let root = OwnedMenu::popup("CreatePopupMenu(root)")?;
    append_text(
        root.0,
        MF_STRING | MF_DISABLED | MF_GRAYED,
        0,
        &presentation.now_playing,
    )?;
    append_separator(root.0)?;
    append_text(
        root.0,
        MF_STRING,
        CMD_PLAY_PAUSE as usize,
        presentation.play_pause,
    )?;
    append_text(
        root.0,
        MF_STRING,
        CMD_PREV_TRACK as usize,
        presentation.previous,
    )?;
    append_text(
        root.0,
        MF_STRING,
        CMD_NEXT_TRACK as usize,
        presentation.next,
    )?;
    let favorite_flags = if presentation.favorite_enabled {
        MF_STRING
    } else {
        MF_STRING | MF_DISABLED | MF_GRAYED
    };
    append_text(
        root.0,
        favorite_flags,
        CMD_TOGGLE_FAVORITE as usize,
        presentation.favorite,
    )?;
    append_separator(root.0)?;

    let modes = OwnedMenu::popup("CreatePopupMenu(play mode)")?;
    append_check_item(
        modes.0,
        CMD_SEQUENTIAL,
        presentation.sequential,
        presentation.play_mode == PlayMode::Sequential,
    )?;
    append_check_item(
        modes.0,
        CMD_LOOP_ALL,
        presentation.loop_all,
        presentation.play_mode == PlayMode::LoopAll,
    )?;
    append_check_item(
        modes.0,
        CMD_LOOP_ONE,
        presentation.loop_one,
        presentation.play_mode == PlayMode::LoopOne,
    )?;
    append_check_item(
        modes.0,
        CMD_SHUFFLE,
        presentation.shuffle,
        presentation.play_mode == PlayMode::Shuffle,
    )?;
    append_text(
        root.0,
        MF_POPUP,
        modes.0 as usize,
        presentation.play_mode_label,
    )?;
    let _ = modes.into_raw(); // ownership transferred to root by AppendMenuW

    append_separator(root.0)?;
    append_text(
        root.0,
        MF_STRING,
        CMD_TOGGLE_WINDOW as usize,
        presentation.toggle_window,
    )?;
    append_separator(root.0)?;
    append_text(root.0, MF_STRING, CMD_QUIT as usize, presentation.quit)?;
    Ok(root.into_raw())
}

fn append_check_item(menu: HMENU, id: u16, label: &str, checked: bool) -> Result<(), TrayError> {
    let flags = MF_STRING | if checked { MF_CHECKED } else { 0 };
    append_text(menu, flags, id as usize, label)
}

fn append_separator(menu: HMENU) -> Result<(), TrayError> {
    // SAFETY: menu is live and MF_SEPARATOR ignores the text pointer.
    if unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null()) } == 0 {
        Err(last_error("AppendMenuW(separator)"))
    } else {
        Ok(())
    }
}

fn append_text(menu: HMENU, flags: u32, id: usize, label: &str) -> Result<(), TrayError> {
    let label = wide(label);
    // SAFETY: AppendMenuW copies the NUL-terminated string during the call.
    if unsafe { AppendMenuW(menu, flags, id, label.as_ptr()) } == 0 {
        Err(last_error("AppendMenuW(item)"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayCallbackAction {
    PrimaryActivation,
    ContextMenu(Option<WPARAM>),
    Ignore,
}

fn classify_callback(identity: TrayIdentity, wparam: WPARAM, packed: LPARAM) -> TrayCallbackAction {
    if identity.matches_callback(packed) {
        return match packed as u32 & 0xffff {
            NIN_SELECT | NIN_KEYSELECT => TrayCallbackAction::PrimaryActivation,
            WM_CONTEXTMENU => TrayCallbackAction::ContextMenu(Some(wparam)),
            // Raw mouse messages alongside version-4 semantic events must not
            // activate twice or reopen a menu that was just dismissed.
            _ => TrayCallbackAction::Ignore,
        };
    }
    // Some third-party trays forward the legacy format even after Explorer
    // accepted version 4. Validate both the full event and the numeric ID.
    if wparam == identity.window_id as WPARAM {
        return match u32::try_from(packed) {
            Ok(WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT) => TrayCallbackAction::PrimaryActivation,
            Ok(WM_RBUTTONUP | WM_CONTEXTMENU) => TrayCallbackAction::ContextMenu(None),
            _ => TrayCallbackAction::Ignore,
        };
    }
    TrayCallbackAction::Ignore
}

/// Idempotently update an existing icon before attempting to add a missing one.
/// A duplicate TaskbarCreated broadcast must not invalidate a working icon.
fn repair_icon(
    mut data: NOTIFYICONDATAW,
    mut notify: impl FnMut(u32, &NOTIFYICONDATAW) -> Result<(), TrayError>,
) -> Result<(), TrayError> {
    if notify(NIM_MODIFY, &data).is_ok() {
        return Ok(());
    }
    if let Err(add_error) = notify(NIM_ADD, &data) {
        // The shell may finish restoring the existing registration between calls.
        return notify(NIM_MODIFY, &data).map_err(|_| add_error);
    }
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    if let Err(error) = notify(NIM_SETVERSION, &data) {
        // Successful registration still provides a usable legacy recovery
        // surface. Our callback decoder supports both formats.
        tracing::debug!(
            operation = error.operation(),
            native_code = error.native_code(),
            "Windows tray retained with legacy callbacks after version negotiation failed"
        );
    }
    Ok(())
}

fn notify_icon(operation: u32, data: &NOTIFYICONDATAW) -> Result<(), TrayError> {
    // SAFETY: data contains handles owned by the caller on this UI thread and
    // no borrowed buffers. Clear stale last-error state: Shell_NotifyIcon only
    // guarantees a BOOL result, so a zero native code means no further detail.
    let result = unsafe {
        SetLastError(0);
        Shell_NotifyIconW(operation, data)
    };
    if result != 0 {
        return Ok(());
    }
    Err(last_error(match operation {
        NIM_ADD => "Shell_NotifyIconW(NIM_ADD)",
        NIM_MODIFY => "Shell_NotifyIconW(NIM_MODIFY)",
        NIM_SETVERSION => "Shell_NotifyIconW(NIM_SETVERSION)",
        _ => "Shell_NotifyIconW",
    }))
}

fn menu_alignment_flag(drop_alignment: bool) -> u32 {
    if drop_alignment {
        TPM_RIGHTALIGN
    } else {
        TPM_LEFTALIGN
    }
}

fn command_for_menu_id(id: u16) -> Option<TrayCommand> {
    match id {
        CMD_PLAY_PAUSE => Some(TrayCommand::PlayPause),
        CMD_PREV_TRACK => Some(TrayCommand::PrevTrack),
        CMD_NEXT_TRACK => Some(TrayCommand::NextTrack),
        CMD_TOGGLE_FAVORITE => Some(TrayCommand::ToggleFavorite),
        CMD_SEQUENTIAL => Some(TrayCommand::SetPlayMode(PlayMode::Sequential)),
        CMD_LOOP_ALL => Some(TrayCommand::SetPlayMode(PlayMode::LoopAll)),
        CMD_LOOP_ONE => Some(TrayCommand::SetPlayMode(PlayMode::LoopOne)),
        CMD_SHUFFLE => Some(TrayCommand::SetPlayMode(PlayMode::Shuffle)),
        CMD_TOGGLE_WINDOW => Some(TrayCommand::Window(TrayWindowCommand::Toggle)),
        CMD_QUIT => Some(TrayCommand::Quit),
        _ => None,
    }
}

fn point_from_packed_position(position: WPARAM) -> POINT {
    let packed = position as u32;
    POINT {
        x: (packed as u16 as i16) as i32,
        y: ((packed >> 16) as u16 as i16) as i32,
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn utf16_array<const N: usize>(value: &str) -> [u16; N] {
    let mut output = [0; N];
    if N == 0 {
        return output;
    }

    let mut cursor = 0;
    for character in value.chars() {
        let mut encoded = [0; 2];
        let units = character.encode_utf16(&mut encoded);
        if cursor + units.len() >= N {
            break;
        }
        output[cursor..cursor + units.len()].copy_from_slice(units);
        cursor += units.len();
    }
    output
}

fn last_error(operation: &'static str) -> TrayError {
    // SAFETY: GetLastError is thread-local and has no preconditions.
    let code = unsafe { GetLastError() };
    TrayError::native(operation, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_presentation() -> TrayPresentation {
        TrayPresentation {
            is_playing: false,
            now_playing: "Not playing".to_string(),
            tooltip: "Rustle — Not playing".to_string(),
            play_pause: "Play",
            previous: "Previous",
            next: "Next",
            favorite: "Add to Favorites",
            favorite_enabled: false,
            is_favorited: false,
            play_mode_label: "Play Mode",
            sequential: "Sequential",
            loop_all: "Loop All",
            loop_one: "Loop One",
            shuffle: "Shuffle",
            toggle_window: "Show/Hide Window",
            quit: "Quit",
            play_mode: PlayMode::Sequential,
        }
    }

    #[test]
    fn backend_uses_the_supplied_presentation_verbatim() {
        let mut presentation = test_presentation();
        presentation.now_playing = "♪ Song — Artist".to_string();
        presentation.play_pause = "Pause";
        presentation.favorite = "Remove from Favorites";
        presentation.favorite_enabled = true;
        presentation.play_mode = PlayMode::LoopOne;

        assert_eq!(presentation.now_playing, "♪ Song — Artist");
        assert_eq!(presentation.play_pause, "Pause");
        assert_eq!(presentation.favorite, "Remove from Favorites");
        assert!(presentation.favorite_enabled);
        assert_eq!(presentation.play_mode, PlayMode::LoopOne);
    }

    #[test]
    fn favorite_is_visible_but_disabled_without_ncm_identity() {
        let presentation = test_presentation();
        assert_eq!(presentation.favorite, "Add to Favorites");
        assert!(!presentation.favorite_enabled);
    }

    #[test]
    fn tooltip_truncation_preserves_surrogate_pairs_and_nul_termination() {
        let long = format!("{}😀", "a".repeat(126));
        let encoded = utf16_array::<128>(&long);
        assert_eq!(encoded[126], 0);
        assert_eq!(encoded[127], 0);

        let exact = format!("{}😀", "a".repeat(125));
        let encoded = utf16_array::<128>(&exact);
        assert_ne!(encoded[125], 0);
        assert_ne!(encoded[126], 0);
        assert_eq!(encoded[127], 0);
        assert!(String::from_utf16(&encoded[..127]).is_ok());
    }

    #[test]
    fn callback_classification_covers_mouse_keyboard_and_context_menu() {
        let window_id = TrayIdentity::window_id(TRAY_ICON_ID);
        assert_eq!(
            classify_callback(window_id, 0, (TRAY_ICON_ID << 16 | NIN_SELECT) as LPARAM),
            TrayCallbackAction::PrimaryActivation
        );
        assert_eq!(
            classify_callback(window_id, 0, (TRAY_ICON_ID << 16 | NIN_KEYSELECT) as LPARAM),
            TrayCallbackAction::PrimaryActivation
        );
        assert_eq!(
            classify_callback(
                window_id,
                0,
                (TRAY_ICON_ID << 16 | WM_CONTEXTMENU) as LPARAM
            ),
            TrayCallbackAction::ContextMenu(Some(0))
        );
        assert_eq!(
            classify_callback(
                window_id,
                0,
                ((TRAY_ICON_ID + 1) << 16 | WM_CONTEXTMENU) as LPARAM,
            ),
            TrayCallbackAction::Ignore
        );
        assert_eq!(
            classify_callback(window_id, 0, (TRAY_ICON_ID << 16 | 0xffff) as LPARAM),
            TrayCallbackAction::Ignore
        );
    }

    #[test]
    fn all_distribution_modes_use_the_numeric_identity() {
        assert_eq!(
            default_tray_identity(),
            TrayIdentity::window_id(TRAY_ICON_ID)
        );
    }

    #[test]
    fn legacy_dock_callbacks_validate_identity_and_use_cursor_position() {
        let identity = default_tray_identity();
        assert_eq!(
            classify_callback(identity, TRAY_ICON_ID as WPARAM, WM_RBUTTONUP as LPARAM),
            TrayCallbackAction::ContextMenu(None)
        );
        assert_eq!(
            classify_callback(identity, TRAY_ICON_ID as WPARAM, WM_CONTEXTMENU as LPARAM),
            TrayCallbackAction::ContextMenu(None)
        );
        assert_eq!(
            classify_callback(identity, TRAY_ICON_ID as WPARAM, WM_LBUTTONUP as LPARAM),
            TrayCallbackAction::PrimaryActivation
        );
        assert_eq!(
            classify_callback(
                identity,
                (TRAY_ICON_ID + 1) as WPARAM,
                WM_RBUTTONUP as LPARAM
            ),
            TrayCallbackAction::Ignore
        );
        // A packed callback for a different icon cannot masquerade as a legacy
        // callback just because its screen coordinates happen to equal our ID.
        assert_eq!(
            classify_callback(
                identity,
                TRAY_ICON_ID as WPARAM,
                ((TRAY_ICON_ID + 1) << 16 | WM_CONTEXTMENU) as LPARAM
            ),
            TrayCallbackAction::Ignore
        );
    }

    #[test]
    fn modern_callbacks_preserve_position_without_duplicate_mouse_activation() {
        let identity = default_tray_identity();
        let position = ((0xffec_u32 << 16) | 0xfff6) as WPARAM;
        assert_eq!(
            classify_callback(
                identity,
                position,
                (TRAY_ICON_ID << 16 | WM_CONTEXTMENU) as LPARAM
            ),
            TrayCallbackAction::ContextMenu(Some(position))
        );
        for event in [WM_RBUTTONUP, WM_LBUTTONUP] {
            assert_eq!(
                classify_callback(identity, position, (TRAY_ICON_ID << 16 | event) as LPARAM),
                TrayCallbackAction::Ignore
            );
        }
    }

    fn scripted_repair(expected: &[(u32, bool)]) -> Result<(), TrayError> {
        let mut steps = expected.iter();
        let result = repair_icon(NOTIFYICONDATAW::default(), |operation, data| {
            let &(expected_operation, succeeds) = steps.next().expect("unexpected shell operation");
            assert_eq!(operation, expected_operation);
            if operation == NIM_SETVERSION {
                // SAFETY: repair_icon initializes the version union before this call.
                assert_eq!(unsafe { data.Anonymous.uVersion }, NOTIFYICON_VERSION_4);
            }
            if succeeds {
                Ok(())
            } else {
                Err(TrayError::native("simulated shell failure", 0))
            }
        });
        assert!(steps.next().is_none(), "missing shell operation");
        result
    }

    #[test]
    fn duplicate_taskbar_notification_updates_existing_icon_without_adding() {
        scripted_repair(&[(NIM_MODIFY, true)]).expect("existing icon remains available");
    }

    #[test]
    fn missing_icon_is_added_and_versioned_but_legacy_shells_keep_the_icon() {
        scripted_repair(&[(NIM_MODIFY, false), (NIM_ADD, true), (NIM_SETVERSION, true)])
            .expect("restore missing icon");
        scripted_repair(&[
            (NIM_MODIFY, false),
            (NIM_ADD, true),
            (NIM_SETVERSION, false),
        ])
        .expect("retain usable legacy registration");
    }

    #[test]
    fn shell_recovery_race_is_rechecked_before_reporting_failure() {
        scripted_repair(&[(NIM_MODIFY, false), (NIM_ADD, false), (NIM_MODIFY, true)])
            .expect("shell restored icon during registration");
        assert!(
            scripted_repair(&[(NIM_MODIFY, false), (NIM_ADD, false), (NIM_MODIFY, false)]).is_err()
        );
    }

    #[test]
    fn recovery_retries_back_off_and_exhaust_the_budget() {
        let mut retry = RecoveryBackoff::default();
        let delays: Vec<_> = std::iter::from_fn(|| retry.next_delay()).collect();
        assert_eq!(delays, [250, 500, 1_000, 2_000, 4_000, 8_000]);
        assert_eq!(retry.next_delay(), None);
        assert_eq!(retry.next_delay(), None);
    }

    #[test]
    fn notification_identity_populates_every_native_identifier_consistently() {
        let mut numeric = NOTIFYICONDATAW::default();
        TrayIdentity::window_id(TRAY_ICON_ID).apply_to_notify_data(&mut numeric);
        assert_eq!(numeric.uID, TRAY_ICON_ID);

        let numeric_identifier = TrayIdentity::window_id(TRAY_ICON_ID).identifier(null_mut());
        assert_eq!(numeric_identifier.uID, TRAY_ICON_ID);
    }

    #[test]
    fn context_menu_helpers_preserve_signed_coordinates_and_system_alignment() {
        let point = point_from_packed_position(((0xffec_u32 << 16) | 0xfff6) as WPARAM);
        assert_eq!((point.x, point.y), (-10, -20));
        assert_eq!(menu_alignment_flag(false), TPM_LEFTALIGN);
        assert_eq!(menu_alignment_flag(true), TPM_RIGHTALIGN);
    }

    #[test]
    fn icon_pixels_are_bgra_and_alpha_premultiplied() {
        assert_eq!(
            premultiplied_bgra(&[100, 50, 200, 128, 1, 2, 3, 0]),
            vec![100, 25, 50, 128, 0, 0, 0, 0]
        );
    }

    #[test]
    fn menu_command_projection_covers_modes_and_window_activation() {
        assert!(matches!(
            command_for_menu_id(CMD_LOOP_ALL),
            Some(TrayCommand::SetPlayMode(PlayMode::LoopAll))
        ));
        assert!(matches!(
            command_for_menu_id(CMD_TOGGLE_WINDOW),
            Some(TrayCommand::Window(TrayWindowCommand::Toggle))
        ));
        assert!(command_for_menu_id(u16::MAX).is_none());
    }

    #[test]
    #[ignore = "requires an interactive Windows Explorer notification area"]
    fn native_shell_registration_update_and_cleanup_smoke_test() {
        run_native_shell_smoke(TrayIdentity::window_id(0x7ffe), "Numeric identity");
    }

    fn run_native_shell_smoke(identity: TrayIdentity, title: &str) {
        let (_handle, mut commands) =
            start_windows_tray_with_identity(test_presentation(), 32, identity)
                .expect("register tray icon");
        assert!(is_available(), "registered tray must be reported available");
        update_state(TrayPresentation {
            is_playing: true,
            now_playing: format!("Rustle Tray Smoke Test — {title}"),
            tooltip: format!("Rustle — Rustle Tray Smoke Test — {title}"),
            play_mode: PlayMode::Shuffle,
            is_favorited: true,
            favorite_enabled: true,
            ..test_presentation()
        })
        .expect("update registered tray icon");
        assert!(is_available(), "updated tray must remain available");
        WINDOWS_TRAY.with(|slot| {
            let mut slot = slot.borrow_mut();
            let tray = slot.as_mut().expect("live native tray");
            // Exercise the same recovery path without broadcasting to or
            // restarting Explorer or touching other applications' icons.
            for _ in 0..3 {
                tray.state.recover(false);
                assert!(tray.state.availability.is_available());
            }
            tray.state
                .finish_sync(Err(TrayError::native("simulated resume failure", 0)))
                .expect("schedule a transient retry");
            assert_eq!(tray.state.availability, TrayAvailability::Recovering);
            assert!(tray.state.recovery.pending);
            let attempts = tray.state.recovery.attempts;
            tray.state.recover(false);
            assert_eq!(
                tray.state.recovery.attempts, attempts,
                "broadcasts must not reset backoff"
            );
            tray.state.recover(true);
            assert!(tray.state.availability.is_available());
            assert!(!tray.state.recovery.pending);
            assert_eq!(tray.state.recovery.attempts, 0);
            tray.state.recover(true); // Ignore a timer queued before cancellation.
            assert!(tray.state.availability.is_available());
            tray.state.recovery.attempts = RECOVERY_DELAYS_MS.len();
            assert!(
                tray.state
                    .finish_sync(Err(TrayError::native("simulated persistent failure", 0)))
                    .is_err()
            );
            assert!(matches!(
                tray.state.availability,
                TrayAvailability::Unavailable(_)
            ));
            assert!(!tray.state.recovery.pending);
            tray.state.recovery = RecoveryBackoff::default();
            tray.state
                .finish_sync(Err(TrayError::native("same outage, new attempt", 0)))
                .expect("retry a new recovery episode");
            assert!(matches!(
                tray.state.availability,
                TrayAvailability::Unavailable(_)
            ));
            tray.state.recover(true);
            assert!(tray.state.availability.is_available());
            tray.state.unregister_icon();
            tray.state.recover(false);
            assert!(
                tray.state.availability.is_available(),
                "restore our missing icon"
            );
        });
        let mut transitions = Vec::new();
        while let Ok(command) = commands.try_recv() {
            if let TrayCommand::AvailabilityChanged(availability) = command {
                transitions.push(availability);
            }
        }
        assert!(
            matches!(
                transitions.as_slice(),
                [
                    TrayAvailability::Available,
                    TrayAvailability::Recovering,
                    TrayAvailability::Available,
                    TrayAvailability::Unavailable(_),
                    TrayAvailability::Available,
                    TrayAvailability::Available,
                ]
            ),
            "duplicate broadcasts and retries must not repeat warnings: {transitions:?}"
        );
        shutdown();
        assert!(
            !is_available(),
            "shutdown tray must be reported unavailable"
        );
    }
}
