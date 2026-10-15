use std::ffi::c_void;
use std::ptr::{null_mut, without_provenance_mut};

use anyhow::{bail, Result};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SystemParametersInfoW, SM_SWAPBUTTON, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
    SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED, SPI_GETMOUSE, SPI_GETMOUSESPEED,
    SPI_SETDOUBLECLICKTIME, SPI_SETKEYBOARDDELAY, SPI_SETKEYBOARDSPEED, SPI_SETMOUSE,
    SPI_SETMOUSEBUTTONSWAP, SPI_SETMOUSESPEED, SYSTEM_PARAMETERS_INFO_ACTION,
};

use super::super::{Keyboard, Level, Mouse, Switch, Touchpad, NO_TOUCHPAD, TOUCHPAD_FIXED};

const SPI_GETTOUCHPADPARAMETERS: SYSTEM_PARAMETERS_INFO_ACTION = 0x00AE;
const SPI_SETTOUCHPADPARAMETERS: SYSTEM_PARAMETERS_INFO_ACTION = 0x00AF;
const TOUCHPAD_PARAMETERS_VERSION: u32 = 1;
const TOUCHPAD_PRESENT: u32 = 1 << 0;
const TAP_ENABLED: u32 = 1 << 2;
const SCROLL_DIRECTION_REVERSED: u32 = 1 << 9;
const ACCELERATION_ON: [i32; 3] = [6, 10, 1];
const ACCELERATION_OFF: [i32; 3] = [0, 0, 0];
const SPEED_STEPS: u32 = 20;
const DELAY_STEP_MS: u32 = 250;
const DELAY_STEPS: u32 = 3;
const RATE_STEPS: u32 = 31;
const SLOWEST_RATE: f64 = 2.5;
const FASTEST_RATE: f64 = 30.0;
const NO_MOUSE_NATURAL_SCROLL: &str = "Windows has no natural scrolling setting for a mouse";

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TouchpadParameters {
    version_number: u32,
    max_supported_contacts: u32,
    legacy_touchpad_features: i32,
    status: u32,
    settings: u32,
    sensitivity_level: i32,
    cursor_speed: u32,
    feedback_intensity: u32,
    click_force_sensitivity: u32,
    right_click_zone_width: u32,
    right_click_zone_height: u32,
}

const TOUCHPAD_PARAMETERS_SIZE: u32 = size_of::<TouchpadParameters>() as u32;

pub(crate) fn backend() -> Result<&'static str> {
    Ok("Windows")
}

pub(crate) fn mouse() -> Result<Mouse> {
    let mut speed = 0u32;
    get(SPI_GETMOUSESPEED, 0, &mut speed)?;
    let mut acceleration = [0i32; 3];
    get(SPI_GETMOUSE, 0, &mut acceleration)?;
    Ok(Mouse {
        speed: Some(step_percent(speed)),
        acceleration: Some(acceleration[2] != 0),
        swap_buttons: Some(unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0),
        double_click: Some(unsafe { GetDoubleClickTime() }),
        natural_scroll: None,
    })
}

pub(crate) fn touchpad() -> Result<Touchpad> {
    let parameters = match touchpad_parameters() {
        Ok(parameters) => parameters,
        Err(error) => {
            return Ok(Touchpad::missing(format!(
                "touchpad settings need Windows 11 24H2 or later: {error}"
            )))
        }
    };
    if parameters.status & TOUCHPAD_PRESENT == 0 {
        return Ok(Touchpad::missing(NO_TOUCHPAD));
    }
    Ok(Touchpad {
        enabled: None,
        tap_to_click: Some(parameters.settings & TAP_ENABLED != 0),
        natural_scroll: Some(parameters.settings & SCROLL_DIRECTION_REVERSED == 0),
        speed: Some(step_percent(parameters.cursor_speed)),
        missing: None,
    })
}

pub(crate) fn keyboard() -> Result<Keyboard> {
    let mut delay = 0u32;
    get(SPI_GETKEYBOARDDELAY, 0, &mut delay)?;
    let mut speed = 0u32;
    get(SPI_GETKEYBOARDSPEED, 0, &mut speed)?;
    Ok(Keyboard {
        repeat_delay: Some(delay_ms(delay)),
        repeat_rate: Some(rate_from_step(speed)),
    })
}

pub(crate) fn set_switch(switch: Switch, on: bool) -> Result<()> {
    match switch {
        Switch::Acceleration => {
            let mut values = if on {
                ACCELERATION_ON
            } else {
                ACCELERATION_OFF
            };
            set(SPI_SETMOUSE, 0, values.as_mut_ptr().cast())
        }
        Switch::SwapButtons => set(SPI_SETMOUSEBUTTONSWAP, u32::from(on), null_mut()),
        Switch::MouseNaturalScroll => bail!(NO_MOUSE_NATURAL_SCROLL),
        Switch::Touchpad => bail!(TOUCHPAD_FIXED),
        Switch::TapToClick => write_touchpad(|parameters| {
            parameters.settings = with_bit(parameters.settings, TAP_ENABLED, on);
        }),
        Switch::TouchpadNaturalScroll => write_touchpad(|parameters| {
            parameters.settings = with_bit(parameters.settings, SCROLL_DIRECTION_REVERSED, !on);
        }),
    }
}

pub(crate) fn set_level(level: Level, value: u32) -> Result<()> {
    match level {
        Level::PointerSpeed => set(
            SPI_SETMOUSESPEED,
            0,
            without_provenance_mut(percent_step(value) as usize),
        ),
        Level::DoubleClick => set(SPI_SETDOUBLECLICKTIME, value, null_mut()),
        Level::TouchpadSpeed => write_touchpad(|parameters| {
            parameters.cursor_speed = percent_step(value);
        }),
        Level::RepeatDelay => set(SPI_SETKEYBOARDDELAY, delay_step(value), null_mut()),
        Level::RepeatRate => set(SPI_SETKEYBOARDSPEED, rate_step(value), null_mut()),
    }
}

fn touchpad_parameters() -> Result<TouchpadParameters> {
    let mut parameters = TouchpadParameters {
        version_number: TOUCHPAD_PARAMETERS_VERSION,
        ..TouchpadParameters::default()
    };
    get(
        SPI_GETTOUCHPADPARAMETERS,
        TOUCHPAD_PARAMETERS_SIZE,
        &mut parameters,
    )?;
    Ok(parameters)
}

fn write_touchpad(change: impl FnOnce(&mut TouchpadParameters)) -> Result<()> {
    let mut parameters = touchpad_parameters()?;
    change(&mut parameters);
    set(
        SPI_SETTOUCHPADPARAMETERS,
        TOUCHPAD_PARAMETERS_SIZE,
        (&raw mut parameters).cast(),
    )
}

fn get<T>(action: SYSTEM_PARAMETERS_INFO_ACTION, ui_param: u32, value: &mut T) -> Result<()> {
    call(action, ui_param, (value as *mut T).cast(), 0)
}

fn set(action: SYSTEM_PARAMETERS_INFO_ACTION, ui_param: u32, pv_param: *mut c_void) -> Result<()> {
    call(
        action,
        ui_param,
        pv_param,
        SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
    )
}

fn call(
    action: SYSTEM_PARAMETERS_INFO_ACTION,
    ui_param: u32,
    pv_param: *mut c_void,
    flags: u32,
) -> Result<()> {
    if unsafe { SystemParametersInfoW(action, ui_param, pv_param, flags) } == 0 {
        bail!(
            "SystemParametersInfoW {action:#06x} failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn with_bit(bits: u32, bit: u32, on: bool) -> u32 {
    if on {
        bits | bit
    } else {
        bits & !bit
    }
}

fn step_percent(step: u32) -> u32 {
    step.min(SPEED_STEPS) * 100 / SPEED_STEPS
}

fn percent_step(percent: u32) -> u32 {
    ((f64::from(percent) * f64::from(SPEED_STEPS) / 100.0).round() as u32).clamp(1, SPEED_STEPS)
}

fn delay_ms(step: u32) -> u32 {
    DELAY_STEP_MS * (step.min(DELAY_STEPS) + 1)
}

fn delay_step(ms: u32) -> u32 {
    ((f64::from(ms) / f64::from(DELAY_STEP_MS)).round() as u32)
        .saturating_sub(1)
        .min(DELAY_STEPS)
}

fn rate_from_step(step: u32) -> u32 {
    let span = FASTEST_RATE - SLOWEST_RATE;
    (SLOWEST_RATE + f64::from(step.min(RATE_STEPS)) * span / f64::from(RATE_STEPS)).round() as u32
}

fn rate_step(rate: u32) -> u32 {
    let span = FASTEST_RATE - SLOWEST_RATE;
    (((f64::from(rate) - SLOWEST_RATE) * f64::from(RATE_STEPS) / span).round() as u32)
        .min(RATE_STEPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_touchpad_parameters_match_the_documented_44_byte_layout() {
        assert_eq!(TOUCHPAD_PARAMETERS_SIZE, 44);
        assert_eq!(std::mem::offset_of!(TouchpadParameters, status), 12);
        assert_eq!(std::mem::offset_of!(TouchpadParameters, settings), 16);
        assert_eq!(std::mem::offset_of!(TouchpadParameters, cursor_speed), 24);
    }

    #[test]
    fn pointer_speed_steps_map_onto_percent_and_back() {
        assert_eq!(step_percent(1), 5);
        assert_eq!(step_percent(10), 50);
        assert_eq!(step_percent(20), 100);
        assert_eq!(step_percent(40), 100);
        assert_eq!(percent_step(0), 1);
        assert_eq!(percent_step(50), 10);
        assert_eq!(percent_step(100), 20);
        for step in 1..=SPEED_STEPS {
            assert_eq!(percent_step(step_percent(step)), step);
        }
    }

    #[test]
    fn repeat_delay_snaps_to_the_nearest_quarter_second() {
        assert_eq!(delay_ms(0), 250);
        assert_eq!(delay_ms(3), 1000);
        assert_eq!(delay_ms(9), 1000);
        assert_eq!(delay_step(100), 0);
        assert_eq!(delay_step(250), 0);
        assert_eq!(delay_step(600), 1);
        assert_eq!(delay_step(1000), 3);
        for step in 0..=DELAY_STEPS {
            assert_eq!(delay_step(delay_ms(step)), step);
        }
    }

    #[test]
    fn repeat_rate_steps_span_two_and_a_half_to_thirty_keys_a_second() {
        assert_eq!(rate_from_step(0), 3);
        assert_eq!(rate_from_step(31), 30);
        assert_eq!(rate_step(2), 0);
        assert_eq!(rate_step(30), 31);
        assert_eq!(rate_step(50), 31);
        for rate in [5, 10, 20, 30] {
            assert_eq!(rate_from_step(rate_step(rate)), rate);
        }
    }

    #[test]
    fn a_bit_turns_on_and_off_without_touching_the_others() {
        let bits = TAP_ENABLED | 1;
        assert_eq!(
            with_bit(bits, SCROLL_DIRECTION_REVERSED, true),
            bits | SCROLL_DIRECTION_REVERSED
        );
        assert_eq!(with_bit(bits, TAP_ENABLED, false), 1);
        assert_eq!(with_bit(bits, TAP_ENABLED, true), bits);
    }
}
