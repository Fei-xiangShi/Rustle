use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};

use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
            IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        },
    },
    core::{PCWSTR, Result as WindowsResult, implement},
};

use crate::player::{PlaybackError, PlaybackResult};

use super::{DeviceChangedCallback, DeviceTopologyChange, PlatformBackend};

enum WatchCommand {
    Changed,
    Stop,
}

#[implement(IMMNotificationClient)]
struct DeviceNotificationClient {
    commands: SyncSender<WatchCommand>,
    default_changed: Arc<AtomicBool>,
}

impl DeviceNotificationClient {
    fn notify(&self, default_changed: bool) {
        if default_changed {
            self.default_changed.store(true, Ordering::Release);
        }
        let _ = self.commands.try_send(WatchCommand::Changed);
    }
}

fn is_output_default_change(flow: EDataFlow, role: ERole) -> bool {
    flow == eRender && role == eConsole
}

impl IMMNotificationClient_Impl for DeviceNotificationClient_Impl {
    fn OnDeviceStateChanged(
        &self,
        _device_id: &PCWSTR,
        _new_state: windows::Win32::Media::Audio::DEVICE_STATE,
    ) -> WindowsResult<()> {
        self.notify(false);
        Ok(())
    }

    fn OnDeviceAdded(&self, _device_id: &PCWSTR) -> WindowsResult<()> {
        self.notify(false);
        Ok(())
    }

    fn OnDeviceRemoved(&self, _device_id: &PCWSTR) -> WindowsResult<()> {
        self.notify(false);
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _default_device_id: &PCWSTR,
    ) -> WindowsResult<()> {
        if is_output_default_change(flow, role) {
            self.notify(true);
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _device_id: &PCWSTR, _key: &PROPERTYKEY) -> WindowsResult<()> {
        Ok(())
    }
}

struct ComApartmentGuard;

impl ComApartmentGuard {
    fn init() -> PlaybackResult<Self> {
        // SAFETY: This dedicated watcher thread owns the matching
        // `CoUninitialize` guard and performs all COM calls from this MTA.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| {
                PlaybackError::DeviceUnavailable(format!(
                    "failed to initialize audio watcher COM apartment: {error}"
                ))
            })?;
        Ok(Self)
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        // SAFETY: `ComApartmentGuard::init` initialized COM on this same
        // dedicated thread and the guard cannot move out of it.
        unsafe { CoUninitialize() };
    }
}

pub(super) struct Backend {
    commands: SyncSender<WatchCommand>,
    thread: Option<JoinHandle<()>>,
}

impl PlatformBackend for Backend {
    const SUPPORTED: bool = true;

    fn new(callback: DeviceChangedCallback) -> PlaybackResult<Self> {
        let (command_tx, command_rx) = mpsc::sync_channel(1);
        let notification_tx = command_tx.clone();
        let default_changed = Arc::new(AtomicBool::new(false));
        let callback_default_changed = Arc::clone(&default_changed);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);

        let thread = thread::Builder::new()
            .name("audio-device-watcher".to_string())
            .spawn(move || {
                let _com_guard = match ComApartmentGuard::init() {
                    Ok(guard) => guard,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };

                // SAFETY: COM is initialized for this thread and the returned
                // interface is retained until after callback unregistration.
                let enumerator: IMMDeviceEnumerator =
                    match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                        Ok(enumerator) => enumerator,
                        Err(error) => {
                            let _ = ready_tx.send(Err(format!(
                                "failed to create audio device enumerator: {error}"
                            )));
                            return;
                        }
                    };
                let client: IMMNotificationClient = DeviceNotificationClient {
                    commands: notification_tx,
                    default_changed: callback_default_changed,
                }
                .into();

                // SAFETY: `client` remains alive and registered only on this
                // watcher thread, then is explicitly unregistered below.
                let registration =
                    unsafe { enumerator.RegisterEndpointNotificationCallback(&client) };
                if let Err(error) = registration {
                    let _ = ready_tx.send(Err(format!(
                        "failed to register audio device notification: {error}"
                    )));
                    return;
                }
                if ready_tx.send(Ok(())).is_err() {
                    // SAFETY: Matches the successful registration above.
                    let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&client) };
                    return;
                }

                while let Ok(command) = command_rx.recv() {
                    match command {
                        WatchCommand::Changed => {
                            callback(DeviceTopologyChange {
                                default_changed: default_changed.swap(false, Ordering::AcqRel),
                            });
                        }
                        WatchCommand::Stop => break,
                    }
                }

                // SAFETY: Matches the successful registration above and runs
                // before `client` or the COM apartment is dropped.
                let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&client) };
            })
            .map_err(|error| {
                PlaybackError::ControlUnavailable(format!(
                    "failed to start audio device watcher: {error}"
                ))
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands: command_tx,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(PlaybackError::DeviceUnavailable(error))
            }
            Err(error) => {
                let _ = thread.join();
                Err(PlaybackError::ControlUnavailable(format!(
                    "audio device watcher exited during startup: {error}"
                )))
            }
        }
    }

    fn stop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        let _ = self.commands.send(WatchCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::Audio::{DEVICE_STATE_ACTIVE, eCapture, eMultimedia};

    #[test]
    fn matches_console_render_default_only() {
        assert!(is_output_default_change(eRender, eConsole));
        assert!(!is_output_default_change(eCapture, eConsole));
        assert!(!is_output_default_change(eRender, eMultimedia));
    }

    #[test]
    fn emits_list_notifications_for_endpoint_events() {
        let (commands, receiver) = mpsc::sync_channel(1);
        let default_changed = Arc::new(AtomicBool::new(false));
        let client: IMMNotificationClient = DeviceNotificationClient {
            commands,
            default_changed: Arc::clone(&default_changed),
        }
        .into();

        // SAFETY: The generated COM shim is invoked directly with inert test
        // values and does not dereference the null endpoint identifier.
        unsafe {
            client
                .OnDeviceStateChanged(PCWSTR::null(), DEVICE_STATE_ACTIVE)
                .unwrap();
        }
        assert!(matches!(receiver.try_recv(), Ok(WatchCommand::Changed)));

        // SAFETY: Same direct shim test as above.
        unsafe { client.OnDeviceAdded(PCWSTR::null()).unwrap() };
        assert!(matches!(receiver.try_recv(), Ok(WatchCommand::Changed)));

        // SAFETY: Same direct shim test as above.
        unsafe { client.OnDeviceRemoved(PCWSTR::null()).unwrap() };
        assert!(matches!(receiver.try_recv(), Ok(WatchCommand::Changed)));
        assert!(!default_changed.load(Ordering::Acquire));
    }

    #[test]
    fn emits_default_notifications_for_console_render_only() {
        let (commands, receiver) = mpsc::sync_channel(1);
        let default_changed = Arc::new(AtomicBool::new(false));
        let client: IMMNotificationClient = DeviceNotificationClient {
            commands,
            default_changed: Arc::clone(&default_changed),
        }
        .into();

        // SAFETY: Direct generated-shim test with inert identifiers.
        unsafe {
            client
                .OnDefaultDeviceChanged(eCapture, eConsole, PCWSTR::null())
                .unwrap();
        }
        assert!(receiver.try_recv().is_err());

        // SAFETY: Direct generated-shim test with inert identifiers.
        unsafe {
            client
                .OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null())
                .unwrap();
        }
        assert!(matches!(receiver.try_recv(), Ok(WatchCommand::Changed)));
        assert!(default_changed.load(Ordering::Acquire));
    }
}
