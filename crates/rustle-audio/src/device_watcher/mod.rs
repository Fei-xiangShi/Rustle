//! Native output-device topology monitoring.

use crate::player::PlaybackResult;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux::Backend as SelectedBackend;
#[cfg(target_os = "macos")]
use macos::Backend as SelectedBackend;
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
use unsupported::Backend as SelectedBackend;
#[cfg(target_os = "windows")]
use windows::Backend as SelectedBackend;

/// One coalesced native topology signal. Device-list refresh is always
/// implied; `default_changed` additionally indicates the platform reported a
/// default output change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeviceTopologyChange {
    pub default_changed: bool,
}

pub(super) type DeviceChangedCallback = Box<dyn Fn(DeviceTopologyChange) + Send + 'static>;

fn enqueue_topology_change(
    default_changed: &AtomicBool,
    signal_tx: &SyncSender<()>,
    change: DeviceTopologyChange,
) {
    if change.default_changed {
        default_changed.store(true, Ordering::Release);
    }
    let _ = signal_tx.try_send(());
}

trait PlatformBackend: Sized {
    const SUPPORTED: bool;

    fn new(callback: DeviceChangedCallback) -> PlaybackResult<Self>;
    fn stop(&mut self);
}

pub(crate) struct DeviceWatcher {
    backend: Option<SelectedBackend>,
    debounce_thread: Option<JoinHandle<()>>,
}

impl DeviceWatcher {
    pub(crate) fn new(callback: DeviceChangedCallback) -> PlaybackResult<Self> {
        let (signal_tx, signal_rx) = std::sync::mpsc::sync_channel(1);
        let default_changed = Arc::new(AtomicBool::new(false));
        let backend_default_changed = Arc::clone(&default_changed);
        let mut backend = SelectedBackend::new(Box::new(move |change| {
            enqueue_topology_change(&backend_default_changed, &signal_tx, change);
        }))?;
        let debounce_thread = match std::thread::Builder::new()
            .name("audio-device-debounce".to_string())
            .spawn(move || {
                while signal_rx.recv().is_ok() {
                    loop {
                        match signal_rx.recv_timeout(Duration::from_millis(200)) {
                            Ok(()) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    callback(DeviceTopologyChange {
                        default_changed: default_changed.swap(false, Ordering::AcqRel),
                    });
                }
            }) {
            Ok(thread) => thread,
            Err(error) => {
                backend.stop();
                return Err(crate::player::PlaybackError::ControlUnavailable(format!(
                    "failed to start audio device debounce worker: {error}"
                )));
            }
        };
        Ok(Self {
            backend: Some(backend),
            debounce_thread: Some(debounce_thread),
        })
    }

    pub(crate) fn stop(&mut self) {
        if let Some(mut backend) = self.backend.take() {
            backend.stop();
        }
        if let Some(thread) = self.debounce_thread.take() {
            let _ = thread.join();
        }
    }

    pub(crate) fn is_supported() -> bool {
        SelectedBackend::SUPPORTED
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_native_backend_support() {
        assert_eq!(
            DeviceWatcher::is_supported(),
            cfg!(any(
                target_os = "windows",
                target_os = "linux",
                target_os = "macos"
            ))
        );
    }

    #[test]
    fn coalescing_preserves_default_change_when_wake_is_already_full() {
        let (signal_tx, signal_rx) = std::sync::mpsc::sync_channel(1);
        let default_changed = AtomicBool::new(false);
        enqueue_topology_change(
            &default_changed,
            &signal_tx,
            DeviceTopologyChange {
                default_changed: false,
            },
        );
        enqueue_topology_change(
            &default_changed,
            &signal_tx,
            DeviceTopologyChange {
                default_changed: true,
            },
        );

        assert!(default_changed.load(Ordering::Acquire));
        assert_eq!(signal_rx.try_iter().count(), 1);
    }
}
