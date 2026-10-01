use std::collections::HashMap;
use std::ffi::c_void;
use std::mem;
use std::ptr::{NonNull, null};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use objc2_core_audio::{
    AudioDeviceID, AudioObjectAddPropertyListener, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectRemovePropertyListener, kAudioDevicePropertyDataSource,
    kAudioDevicePropertyDataSources, kAudioDevicePropertyDeviceIsAlive,
    kAudioDevicePropertyJackIsConnected, kAudioDevicePropertyNominalSampleRate,
    kAudioDevicePropertyPreferredChannelLayout, kAudioDevicePropertyPreferredChannelsForStereo,
    kAudioDevicePropertyScopeOutput, kAudioDevicePropertyStreamConfiguration,
    kAudioDevicePropertyStreams, kAudioHardwareNoError, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
};

use crate::player::{PlaybackError, PlaybackResult};

use super::{DeviceChangedCallback, DeviceTopologyChange, PlatformBackend};

enum WatchCommand {
    Changed,
    Stop,
}

#[derive(Clone)]
struct CallbackRegistration {
    commands: SyncSender<WatchCommand>,
    default_changed: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
struct PropertyRegistration {
    object_id: AudioObjectID,
    address: AudioObjectPropertyAddress,
}

static CALLBACKS: OnceLock<Mutex<HashMap<usize, CallbackRegistration>>> = OnceLock::new();

fn callbacks() -> &'static Mutex<HashMap<usize, CallbackRegistration>> {
    CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remove_callback(context_id: usize) {
    if let Ok(mut registered_callbacks) = callbacks().lock() {
        registered_callbacks.remove(&context_id);
    }
}

/// CoreAudio listener registered for output topology and device properties.
///
/// # Safety
///
/// CoreAudio must invoke this function with its `AudioObjectPropertyListenerProc`
/// ABI, an address array valid for `address_count` elements, and the retained
/// watcher context used during registration. Listener shutdown unregisters the
/// callback before removing that context from `CALLBACKS` and dropping its token.
unsafe extern "C-unwind" fn property_changed(
    _object_id: AudioObjectID,
    address_count: u32,
    addresses: NonNull<AudioObjectPropertyAddress>,
    context: *mut c_void,
) -> i32 {
    crate::ffi_guard::catch_unwind(
        "coreaudio_property_listener",
        Box::new(move || {
            // SAFETY: The outer callback receives the CoreAudio-owned address
            // slice and retained context under the contract documented below.
            unsafe { property_changed_inner(address_count, addresses, context) }
        }),
        Box::new(|| kAudioHardwareNoError),
    )
}

/// Process a CoreAudio property notification behind the no-unwind ABI guard.
///
/// # Safety
///
/// `addresses` must point to `address_count` initialized property addresses for
/// the duration of the callback. `context` must be the retained watcher token
/// registered in `CALLBACKS`; shutdown removes native listeners before dropping
/// that token or its callback registration.
unsafe fn property_changed_inner(
    address_count: u32,
    addresses: NonNull<AudioObjectPropertyAddress>,
    context: *mut c_void,
) -> i32 {
    let context_id = context as usize;
    let registration = callbacks()
        .lock()
        .ok()
        .and_then(|callbacks| callbacks.get(&context_id).cloned());
    if let Some(registration) = registration {
        // SAFETY: CoreAudio supplies `address_count` valid addresses for the
        // duration of this callback.
        let addresses =
            unsafe { std::slice::from_raw_parts(addresses.as_ptr(), address_count as usize) };
        let default_changed = addresses
            .iter()
            .any(|address| address.mSelector == kAudioHardwarePropertyDefaultOutputDevice);
        if default_changed {
            registration.default_changed.store(true, Ordering::Release);
        }
        let _ = registration.commands.try_send(WatchCommand::Changed);
    }
    kAudioHardwareNoError
}

fn context_pointer(context_id: usize) -> *mut c_void {
    context_id as *mut c_void
}

fn register_property(
    object_id: AudioObjectID,
    address: AudioObjectPropertyAddress,
    context_id: usize,
) -> PlaybackResult<PropertyRegistration> {
    // SAFETY: `address` lives for this call; the context points to a boxed
    // token retained by the watcher until all listeners are removed.
    let status = unsafe {
        AudioObjectAddPropertyListener(
            object_id,
            NonNull::from(&address),
            Some(property_changed),
            context_pointer(context_id),
        )
    };
    if status != kAudioHardwareNoError {
        return Err(PlaybackError::DeviceUnavailable(format!(
            "failed to register CoreAudio property listener: {status}"
        )));
    }
    Ok(PropertyRegistration { object_id, address })
}

fn unregister_property(registration: PropertyRegistration, context_id: usize) {
    // SAFETY: This matches a successful registration with the same object,
    // address, callback, and retained context.
    let status = unsafe {
        AudioObjectRemovePropertyListener(
            registration.object_id,
            NonNull::from(&registration.address),
            Some(property_changed),
            context_pointer(context_id),
        )
    };
    if status != kAudioHardwareNoError {
        tracing::warn!(status, "Failed to remove CoreAudio property listener");
    }
}

fn audio_device_ids() -> PlaybackResult<Vec<AudioDeviceID>> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDevices,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut data_size = 0;
    // SAFETY: All pointers refer to initialized local storage of the sizes
    // required by CoreAudio's property API.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut data_size),
        )
    };
    if status != kAudioHardwareNoError {
        return Err(PlaybackError::DeviceUnavailable(format!(
            "failed to read CoreAudio device-list size: {status}"
        )));
    }

    let device_count = data_size as usize / mem::size_of::<AudioDeviceID>();
    let mut devices = vec![0; device_count];
    let Some(data) = NonNull::new(devices.as_mut_ptr().cast()) else {
        return Ok(Vec::new());
    };
    // SAFETY: `devices` was allocated from the exact byte size CoreAudio
    // reported and remains live/mutable for the call.
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut data_size),
            data,
        )
    };
    if status != kAudioHardwareNoError {
        return Err(PlaybackError::DeviceUnavailable(format!(
            "failed to read CoreAudio device list: {status}"
        )));
    }
    devices.truncate(data_size as usize / mem::size_of::<AudioDeviceID>());
    Ok(devices)
}

fn supports_output(device_id: AudioDeviceID) -> bool {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyStreams,
        mScope: kAudioDevicePropertyScopeOutput,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut data_size = 0;
    // SAFETY: Output is written only to `data_size`; `address` is valid for
    // the duration of the call.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            device_id,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut data_size),
        )
    };
    status == kAudioHardwareNoError && data_size >= mem::size_of::<AudioObjectID>() as u32
}

fn register_device_properties(context_id: usize) -> Vec<PropertyRegistration> {
    let Ok(devices) = audio_device_ids() else {
        return Vec::new();
    };
    let properties = [
        (
            kAudioDevicePropertyDeviceIsAlive,
            kAudioObjectPropertyScopeGlobal,
        ),
        (
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
        ),
        (
            kAudioDevicePropertyStreamConfiguration,
            kAudioDevicePropertyScopeOutput,
        ),
        (
            kAudioDevicePropertyDataSource,
            kAudioDevicePropertyScopeOutput,
        ),
        (
            kAudioDevicePropertyDataSources,
            kAudioDevicePropertyScopeOutput,
        ),
        (
            kAudioDevicePropertyJackIsConnected,
            kAudioDevicePropertyScopeOutput,
        ),
        (
            kAudioDevicePropertyPreferredChannelsForStereo,
            kAudioDevicePropertyScopeOutput,
        ),
        (
            kAudioDevicePropertyPreferredChannelLayout,
            kAudioDevicePropertyScopeOutput,
        ),
    ];

    devices
        .into_iter()
        .filter(|device_id| supports_output(*device_id))
        .flat_map(|device_id| {
            properties.iter().filter_map(move |(selector, scope)| {
                register_property(
                    device_id,
                    AudioObjectPropertyAddress {
                        mSelector: *selector,
                        mScope: *scope,
                        mElement: kAudioObjectPropertyElementMain,
                    },
                    context_id,
                )
                .ok()
            })
        })
        .collect()
}

fn refresh_device_properties(registrations: &mut Vec<PropertyRegistration>, context_id: usize) {
    for registration in registrations.drain(..) {
        unregister_property(registration, context_id);
    }
    *registrations = register_device_properties(context_id);
}

pub(super) struct Backend {
    commands: SyncSender<WatchCommand>,
    thread: Option<JoinHandle<()>>,
}

impl PlatformBackend for Backend {
    const SUPPORTED: bool = true;

    fn new(callback: DeviceChangedCallback) -> PlaybackResult<Self> {
        let (command_tx, command_rx) = mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let default_changed = Arc::new(AtomicBool::new(false));
        let context = Box::new(0_u8);
        let context_id = std::ptr::from_ref(context.as_ref()) as usize;
        callbacks()
            .lock()
            .map_err(|_| {
                PlaybackError::InvariantViolation(
                    "CoreAudio callback registry is poisoned".to_string(),
                )
            })?
            .insert(
                context_id,
                CallbackRegistration {
                    commands: command_tx.clone(),
                    default_changed: Arc::clone(&default_changed),
                },
            );

        let thread = thread::Builder::new()
            .name("coreaudio-device-watcher".to_string())
            .spawn(move || {
                let _context = context;
                let system_properties = [
                    AudioObjectPropertyAddress {
                        mSelector: kAudioHardwarePropertyDefaultOutputDevice,
                        mScope: kAudioObjectPropertyScopeGlobal,
                        mElement: kAudioObjectPropertyElementMain,
                    },
                    AudioObjectPropertyAddress {
                        mSelector: kAudioHardwarePropertyDevices,
                        mScope: kAudioObjectPropertyScopeGlobal,
                        mElement: kAudioObjectPropertyElementMain,
                    },
                ];
                let mut system_registrations = Vec::with_capacity(system_properties.len());
                for address in system_properties {
                    match register_property(
                        kAudioObjectSystemObject as AudioObjectID,
                        address,
                        context_id,
                    ) {
                        Ok(registration) => system_registrations.push(registration),
                        Err(error) => {
                            for registration in system_registrations.drain(..) {
                                unregister_property(registration, context_id);
                            }
                            remove_callback(context_id);
                            let _ = ready_tx.send(Err(error.to_string()));
                            return;
                        }
                    }
                }
                let mut device_registrations = register_device_properties(context_id);
                if ready_tx.send(Ok(())).is_err() {
                    for registration in device_registrations.drain(..) {
                        unregister_property(registration, context_id);
                    }
                    for registration in system_registrations.drain(..) {
                        unregister_property(registration, context_id);
                    }
                    remove_callback(context_id);
                    return;
                }

                while let Ok(command) = command_rx.recv() {
                    match command {
                        WatchCommand::Changed => {
                            refresh_device_properties(&mut device_registrations, context_id);
                            callback(DeviceTopologyChange {
                                default_changed: default_changed.swap(false, Ordering::AcqRel),
                            });
                        }
                        WatchCommand::Stop => break,
                    }
                }

                for registration in device_registrations.drain(..) {
                    unregister_property(registration, context_id);
                }
                for registration in system_registrations.drain(..) {
                    unregister_property(registration, context_id);
                }
                remove_callback(context_id);
            })
            .map_err(|error| {
                remove_callback(context_id);
                PlaybackError::ControlUnavailable(format!(
                    "failed to start CoreAudio device watcher: {error}"
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
                    "CoreAudio device watcher exited during startup: {error}"
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
