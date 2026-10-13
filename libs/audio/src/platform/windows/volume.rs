use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::IMMDeviceEnumerator;
use windows::Win32::System::Com::CLSCTX_ALL;

use crate::devices::Direction;
use crate::AudioError;

use super::com;

const FULL_SCALE: f32 = 100.0;

fn endpoint_volume(
    enumerator: &IMMDeviceEnumerator,
    direction: Direction,
) -> Result<Option<IAudioEndpointVolume>, AudioError> {
    let Some(endpoint) = com::default_endpoint(enumerator, direction)? else {
        return Ok(None);
    };
    unsafe { endpoint.device.Activate(CLSCTX_ALL, None) }
        .map(Some)
        .map_err(com::failed("cannot open the endpoint volume"))
}

fn required_volume(
    enumerator: &IMMDeviceEnumerator,
    direction: Direction,
) -> Result<IAudioEndpointVolume, AudioError> {
    endpoint_volume(enumerator, direction)?.ok_or_else(|| AudioError::no_default(direction))
}

pub(crate) fn volume_percent(direction: Direction) -> Result<Option<u32>, AudioError> {
    com::with_enumerator(|enumerator| {
        endpoint_volume(enumerator, direction)?
            .map(|volume| {
                unsafe { volume.GetMasterVolumeLevelScalar() }
                    .map(percent_of)
                    .map_err(com::failed("cannot read the volume"))
            })
            .transpose()
    })
}

pub(crate) fn set_volume_percent(direction: Direction, percent: u32) -> Result<(), AudioError> {
    com::with_enumerator(|enumerator| {
        let volume = required_volume(enumerator, direction)?;
        unsafe { volume.SetMasterVolumeLevelScalar(level_of(percent), std::ptr::null()) }
            .map_err(com::failed("cannot set the volume"))
    })
}

pub(crate) fn is_muted(direction: Direction) -> Result<Option<bool>, AudioError> {
    com::with_enumerator(|enumerator| {
        endpoint_volume(enumerator, direction)?
            .map(|volume| {
                unsafe { volume.GetMute() }
                    .map(|muted| muted.as_bool())
                    .map_err(com::failed("cannot read the mute state"))
            })
            .transpose()
    })
}

pub(crate) fn set_muted(direction: Direction, muted: bool) -> Result<(), AudioError> {
    com::with_enumerator(|enumerator| {
        let volume = required_volume(enumerator, direction)?;
        unsafe { volume.SetMute(muted, std::ptr::null()) }
            .map_err(com::failed("cannot change the mute state"))
    })
}

fn percent_of(level: f32) -> u32 {
    (level.clamp(0.0, 1.0) * FULL_SCALE).round() as u32
}

fn level_of(percent: u32) -> f32 {
    percent.min(100) as f32 / FULL_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scalar_level_maps_to_whole_percent_and_back() {
        let cases = [
            (0.0, 0),
            (0.354, 35),
            (0.356, 36),
            (1.0, 100),
            (1.5, 100),
            (-0.2, 0),
        ];
        for (level, percent) in cases {
            assert_eq!(percent_of(level), percent, "{level}");
        }
        for (percent, level) in [(0, 0.0), (35, 0.35), (100, 1.0), (140, 1.0)] {
            assert!(
                (level_of(percent) - level).abs() < f32::EPSILON,
                "{percent}"
            );
        }
    }
}
