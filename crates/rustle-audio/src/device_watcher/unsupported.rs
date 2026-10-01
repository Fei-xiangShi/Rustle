use crate::player::{PlaybackError, PlaybackResult};

use super::{DeviceChangedCallback, PlatformBackend};

pub(super) struct Backend;

impl PlatformBackend for Backend {
    const SUPPORTED: bool = false;

    fn new(_callback: DeviceChangedCallback) -> PlaybackResult<Self> {
        Err(PlaybackError::DeviceUnavailable(
            "native audio device monitoring is unsupported on this platform".to_string(),
        ))
    }

    fn stop(&mut self) {}
}
