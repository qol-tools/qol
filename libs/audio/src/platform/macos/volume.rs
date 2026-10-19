use crate::devices::Direction;
use crate::AudioError;

use super::default_output::default_device;
use super::hal::{self, ObjectId, Property};

const FULL_SCALE: f32 = 100.0;

fn control_on(device: ObjectId, direction: Direction, selector: [u8; 4]) -> Option<Property> {
    let property = Property::scoped(selector, direction);
    hal::has(device, property).then_some(property)
}

fn control(
    direction: Direction,
    selector: [u8; 4],
) -> Result<Option<(ObjectId, Property)>, AudioError> {
    Ok(default_device(direction)?.and_then(|device| {
        control_on(device, direction, selector).map(|property| (device, property))
    }))
}

fn required_control(
    direction: Direction,
    selector: [u8; 4],
    missing: &str,
) -> Result<(ObjectId, Property), AudioError> {
    let device = default_device(direction)?.ok_or_else(|| AudioError::no_default(direction))?;
    let property = control_on(device, direction, selector)
        .ok_or_else(|| AudioError::Operation(missing.to_owned()))?;
    Ok((device, property))
}

pub(crate) fn volume_percent(direction: Direction) -> Result<Option<u32>, AudioError> {
    control(direction, hal::VIRTUAL_MAIN_VOLUME)?
        .map(|(device, property)| hal::read_f32(device, property).map(percent_of))
        .transpose()
}

pub(crate) fn set_volume_percent(direction: Direction, percent: u32) -> Result<(), AudioError> {
    let (device, property) = required_control(
        direction,
        hal::VIRTUAL_MAIN_VOLUME,
        "the default device has no volume control",
    )?;
    hal::write_f32(device, property, level_of(percent))
}

pub(crate) fn is_muted(direction: Direction) -> Result<Option<bool>, AudioError> {
    control(direction, hal::MUTE)?
        .map(|(device, property)| hal::read_u32(device, property).map(|muted| muted != 0))
        .transpose()
}

pub(crate) fn set_muted(direction: Direction, muted: bool) -> Result<(), AudioError> {
    let (device, property) = required_control(
        direction,
        hal::MUTE,
        "the default device has no mute control",
    )?;
    hal::write_u32(device, property, u32::from(muted))
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
            (0.4375, 44),
            (0.354, 35),
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
