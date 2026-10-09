use std::time::Duration;

use qol_audio::default_output;
use qol_audio::meter::Meter;
use serde::{Deserialize, Serialize};

use crate::device::{Role, INPUT, OUTPUT};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    pub output: f32,
    pub input: f32,
}

pub fn open(role: Role) -> Option<Meter> {
    match Meter::open(role.direction) {
        Ok(meter) => Some(meter),
        Err(error) => {
            log::debug!("cannot measure the {}: {error}", role.noun);
            None
        }
    }
}

pub fn is_current(meter: &Meter, role: Role) -> bool {
    if meter.is_finished() {
        return false;
    }
    match default_output::effective(role.direction) {
        Ok(default) => measures(meter.device(), default.as_deref()),
        Err(error) => {
            log::debug!("cannot read the default {}: {error}", role.noun);
            true
        }
    }
}

fn measures(device: &str, default: Option<&str>) -> bool {
    default == Some(device)
}

pub fn peak(meter: Option<&Meter>) -> f32 {
    meter.map_or(0.0, |meter| clamped(meter.peak()))
}

fn clamped(level: f32) -> f32 {
    if level.is_nan() {
        return 0.0;
    }
    level.clamp(0.0, 1.0)
}

pub fn before_volume(meter: Option<&Meter>, percent: Option<u32>) -> f32 {
    match (meter, percent) {
        (Some(meter), Some(percent)) => clamped(meter.peak_before_volume(percent)),
        (meter, None) => peak(meter),
        (None, Some(_)) => 0.0,
    }
}

pub fn sample(settle: Duration) -> Levels {
    let output = open(OUTPUT);
    let input = open(INPUT);
    std::thread::sleep(settle);
    Levels {
        output: before_volume(output.as_ref(), crate::volume::percent().ok().flatten()),
        input: before_volume(
            input.as_ref(),
            crate::volume::input_percent().ok().flatten(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_is_current_only_while_it_measures_the_default() {
        assert!(measures("alsa_output.usb", Some("alsa_output.usb")));
        assert!(!measures("alsa_output.usb", Some("bluez_output.luna")));
        assert!(!measures("alsa_output.usb", None));
    }

    #[test]
    fn a_level_is_clamped_to_the_unit_range() {
        assert_eq!(clamped(0.42), 0.42);
        assert_eq!(clamped(-0.1), 0.0);
        assert_eq!(clamped(1.7), 1.0);
        assert_eq!(clamped(f32::NAN), 0.0);
    }

    #[test]
    fn no_meter_reads_as_silence_at_any_volume() {
        assert_eq!(before_volume(None, Some(80)), 0.0);
        assert_eq!(before_volume(None, None), 0.0);
    }

    #[test]
    fn no_meter_reads_as_silence() {
        assert_eq!(peak(None), 0.0);
    }

    #[test]
    fn the_payload_is_an_object_the_slider_reads_by_direction() {
        let payload = serde_json::to_value(Levels {
            output: 0.5,
            input: 0.0,
        })
        .unwrap();
        assert_eq!(payload, serde_json::json!({ "output": 0.5, "input": 0.0 }));
    }
}
