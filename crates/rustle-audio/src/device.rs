//! Stable output-device identity, discovery, and Rodio output construction.

use std::sync::Arc;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{DeviceSinkBuilder, MixerDeviceSink};

use crate::player::{PlaybackError, PlaybackResult};

/// User-facing output device projection.
///
/// `id` is the CPAL stable device identifier and is the only value suitable
/// for persistence. `name` is display-only and may be duplicated or renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRecoveryReason {
    ManualSelection,
    DefaultDeviceChanged,
    StreamFailed,
    OutputStalled,
    ExplicitRetry,
}

pub(crate) type OutputFailureCallback = Arc<dyn Fn(PlaybackError) + Send + Sync + 'static>;

pub(crate) struct OpenedOutput {
    pub stream: MixerDeviceSink,
    pub active_device: AudioDevice,
    /// The persisted selector after legacy display-name migration. `None`
    /// deliberately means follow the system default.
    pub resolved_selection: Option<String>,
}

fn device_name(device: &rodio::cpal::Device) -> Option<String> {
    device
        .description()
        .ok()
        .map(|description| description.name().to_owned())
}

fn device_id(device: &rodio::cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

fn is_synthetic_default_device(name: &str) -> bool {
    cfg!(target_os = "linux") && matches!(name, "default_output" | "default_sink")
}

fn find_device(host: &rodio::cpal::Host, selector: &str) -> Option<rodio::cpal::Device> {
    if let Ok(id) = selector.parse::<rodio::cpal::DeviceId>() {
        return host.device_by_id(&id);
    }

    host.output_devices()
        .ok()?
        .find(|device| device_name(device).as_deref() == Some(selector))
}

fn list_output_devices_inner() -> Vec<AudioDevice> {
    let host = rodio::cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|device| device_id(&device));

    let mut devices: Vec<_> = host
        .output_devices()
        .map(|devices| {
            devices
                .filter_map(|device| {
                    let name = device_name(&device)?;
                    if is_synthetic_default_device(&name) {
                        return None;
                    }
                    let id = device_id(&device)?;
                    if device.default_output_config().is_err() {
                        return None;
                    }
                    Some(AudioDevice {
                        is_default: default_id.as_deref() == Some(id.as_str()),
                        id,
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    devices.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    devices
}

/// Enumerate the current output topology. No process-lifetime cache is used.
pub fn get_audio_devices() -> Vec<AudioDevice> {
    run_on_device_thread(|| Ok(list_output_devices_inner())).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "Failed to enumerate audio output devices");
        Vec::new()
    })
}

pub(crate) fn open_output(
    selector: Option<&str>,
    on_failure: OutputFailureCallback,
) -> PlaybackResult<OpenedOutput> {
    let selector = selector.map(str::to_owned);
    run_on_device_thread(move || open_output_inner(selector, on_failure))
}

fn open_output_inner(
    selector: Option<String>,
    on_failure: OutputFailureCallback,
) -> PlaybackResult<OpenedOutput> {
    let host = rodio::cpal::default_host();
    let device = match selector.as_deref() {
        Some(selector) => find_device(&host, selector)
            .ok_or_else(|| PlaybackError::DeviceUnavailable(selector.to_owned()))?,
        None => host.default_output_device().ok_or_else(|| {
            PlaybackError::DeviceUnavailable("no default output device is available".to_string())
        })?,
    };
    let id = device_id(&device).ok_or_else(|| {
        PlaybackError::DeviceUnavailable("output device has no stable identifier".to_string())
    })?;
    let name = device_name(&device).unwrap_or_else(|| id.clone());
    let default_id = host
        .default_output_device()
        .and_then(|device| device_id(&device));
    let config = device
        .default_output_config()
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?;
    let sample_rate = rodio::SampleRate::new(config.sample_rate()).ok_or_else(|| {
        PlaybackError::DeviceUnavailable("output device reported a zero sample rate".to_string())
    })?;
    let callback = move |error: rodio::cpal::StreamError| {
        on_failure(PlaybackError::DeviceUnavailable(error.to_string()));
    };
    let stream = DeviceSinkBuilder::from_device(device)
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?
        .with_sample_rate(sample_rate)
        .with_error_callback(callback)
        .open_stream()
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?;

    Ok(OpenedOutput {
        stream,
        active_device: AudioDevice {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id: id.clone(),
            name,
        },
        resolved_selection: selector.map(|_| id),
    })
}

#[cfg(target_os = "windows")]
fn run_on_device_thread<T: Send + 'static>(
    operation: impl FnOnce() -> PlaybackResult<T> + Send + 'static,
) -> PlaybackResult<T> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::OnceLock;
    use std::sync::mpsc::{SyncSender, channel, sync_channel};

    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

    type Job = Box<dyn FnOnce() + Send + 'static>;
    static WORKER: OnceLock<Option<SyncSender<Job>>> = OnceLock::new();

    let worker = WORKER
        .get_or_init(|| {
            let (job_tx, job_rx) = sync_channel::<Job>(1);
            std::thread::Builder::new()
                .name("audio-device-mta".to_string())
                .spawn(move || {
                    // SAFETY: This dedicated process-lifetime worker owns its
                    // COM apartment, and all CPAL WASAPI work stays on it.
                    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                    for job in job_rx {
                        let _ = catch_unwind(AssertUnwindSafe(job));
                    }
                })
                .ok()
                .map(|_| job_tx)
        })
        .as_ref()
        .ok_or_else(|| {
            PlaybackError::ControlUnavailable("audio device worker is unavailable".to_string())
        })?;
    let (result_tx, result_rx) = channel();
    worker
        .send(Box::new(move || {
            let result = catch_unwind(AssertUnwindSafe(operation)).unwrap_or_else(|_| {
                Err(PlaybackError::DeviceUnavailable(
                    "audio device operation panicked".to_string(),
                ))
            });
            let _ = result_tx.send(result);
        }))
        .map_err(|_| {
            PlaybackError::ControlUnavailable("audio device worker stopped".to_string())
        })?;
    result_rx.recv().map_err(|_| {
        PlaybackError::ControlUnavailable("audio device result was dropped".to_string())
    })?
}

#[cfg(not(target_os = "windows"))]
fn run_on_device_thread<T>(operation: impl FnOnce() -> PlaybackResult<T>) -> PlaybackResult<T> {
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_default_devices_are_hidden_only_on_linux() {
        assert_eq!(
            is_synthetic_default_device("default_output"),
            cfg!(target_os = "linux")
        );
        assert_eq!(
            is_synthetic_default_device("default_sink"),
            cfg!(target_os = "linux")
        );
        assert!(!is_synthetic_default_device("Built-in Audio Analog Stereo"));
    }

    #[test]
    fn display_names_do_not_parse_as_stable_ids() {
        assert!(
            "Speakers (Example Audio)"
                .parse::<rodio::cpal::DeviceId>()
                .is_err()
        );
    }
}
