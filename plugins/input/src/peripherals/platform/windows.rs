use std::ffi::c_void;
use std::ptr::{null_mut, without_provenance_mut};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetDoubleClickTime, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_F24, VK_LWIN, VK_MENU, VK_RWIN,
    VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SendNotifyMessageW, SystemParametersInfoW, HWND_BROADCAST, SM_SWAPBUTTON,
    SPIF_UPDATEINIFILE, SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED, SPI_GETMOUSE,
    SPI_GETMOUSESPEED, SPI_SETDOUBLECLICKTIME, SPI_SETKEYBOARDDELAY, SPI_SETKEYBOARDSPEED,
    SPI_SETMOUSE, SPI_SETMOUSEBUTTONSWAP, SPI_SETMOUSESPEED, SYSTEM_PARAMETERS_INFO_ACTION,
    WM_SETTINGCHANGE,
};

use super::super::{
    Baseline, Keyboard, Level, Mouse, Setting, Steps, Switch, Touchpad, NO_TOUCHPAD,
};

const SPI_GETTOUCHPADPARAMETERS: SYSTEM_PARAMETERS_INFO_ACTION = 0x00AE;
const SPI_SETTOUCHPADPARAMETERS: SYSTEM_PARAMETERS_INFO_ACTION = 0x00AF;
const TOUCHPAD_PARAMETERS_VERSION: u32 = 1;
const TOUCHPAD_PRESENT: u32 = 1 << 0;
const LEGACY_TOUCHPAD_PRESENT: u32 = 1 << 1;
const TOUCHPAD_ENABLED: u32 = 1 << 3;
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
const LEGACY_TOUCHPAD: &str =
    "this touchpad is not a Precision Touchpad, so its settings are unavailable here";
const TOUCHPAD_SHORTCUT: [VIRTUAL_KEY; 3] = [VK_CONTROL, VK_LWIN, VK_F24];
const MODIFIERS: [VIRTUAL_KEY; 5] = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN];
const SHORTCUT_SETTLE: Duration = Duration::from_secs(2);
const SHORTCUT_POLL: Duration = Duration::from_millis(20);

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
    Ok(Mouse {
        speed: Some(step_percent(read_level(Level::PointerSpeed)?)),
        acceleration: Some(read_acceleration()?[2] != 0),
        swap_buttons: Some(read_flag(Switch::SwapButtons)?),
        double_click: Some(read_level(Level::DoubleClick)?),
        natural_scroll: None,
    })
}

pub(crate) fn touchpad() -> Result<Touchpad> {
    Ok(match touchpad_parameters() {
        Ok(parameters) => touchpad_from(&parameters),
        Err(error) => Touchpad::missing(format!(
            "touchpad settings need Windows 11 24H2 or later: {error}"
        )),
    })
}

pub(crate) fn keyboard() -> Result<Keyboard> {
    Ok(Keyboard {
        repeat_delay: Some(delay_ms(read_level(Level::RepeatDelay)?)),
        repeat_rate: Some(rate_from_step(read_level(Level::RepeatRate)?)),
    })
}

pub(crate) fn set_switch(switch: Switch, on: bool) -> Result<()> {
    write_flag(switch, on != (switch == Switch::TouchpadNaturalScroll))
}

pub(crate) fn set_level(level: Level, value: u32) -> Result<()> {
    let native = match level {
        Level::PointerSpeed | Level::TouchpadSpeed => percent_step(value),
        Level::DoubleClick => value,
        Level::RepeatDelay => delay_step(value),
        Level::RepeatRate => rate_step(value),
    };
    write_level(level, native)
}

pub(crate) fn capture(setting: Setting) -> Result<Baseline> {
    Ok(match setting {
        Setting::Level(level) => Baseline::Number(read_level(level)?),
        Setting::Switch(Switch::Acceleration) => Baseline::Numbers(read_acceleration()?.to_vec()),
        Setting::Switch(switch) => Baseline::Number(u32::from(read_flag(switch)?)),
    })
}

pub(crate) fn restore(setting: Setting, baseline: Baseline) -> Result<()> {
    match (setting, baseline) {
        (Setting::Level(level), Baseline::Number(native)) => write_level(level, native),
        (Setting::Switch(Switch::Acceleration), Baseline::Numbers(values)) => {
            match <[i32; 3]>::try_from(values) {
                Ok(values) => write_acceleration(values),
                Err(values) => bail!("the saved acceleration {values:?} is not three numbers"),
            }
        }
        (Setting::Switch(switch), Baseline::Number(flag)) => write_flag(switch, flag != 0),
        _ => bail!(
            "the saved {} is not a Windows setting value",
            setting.name()
        ),
    }
}

pub(crate) fn values(level: Level) -> Steps {
    match level {
        Level::PointerSpeed | Level::TouchpadSpeed => Steps {
            first: step_percent(1),
            last: step_percent(SPEED_STEPS),
            step: step_percent(1),
        },
        Level::DoubleClick => level.bounds(),
        Level::RepeatDelay => Steps {
            first: delay_ms(0),
            last: delay_ms(DELAY_STEPS),
            step: DELAY_STEP_MS,
        },
        Level::RepeatRate => Steps::span(rate_from_step(0), rate_from_step(RATE_STEPS)),
    }
}

fn read_level(level: Level) -> Result<u32> {
    let action = match level {
        Level::PointerSpeed => SPI_GETMOUSESPEED,
        Level::DoubleClick => return Ok(unsafe { GetDoubleClickTime() }),
        Level::TouchpadSpeed => return Ok(touchpad_parameters()?.cursor_speed),
        Level::RepeatDelay => SPI_GETKEYBOARDDELAY,
        Level::RepeatRate => SPI_GETKEYBOARDSPEED,
    };
    let mut native = 0u32;
    get(action, 0, &mut native)?;
    Ok(native)
}

fn write_level(level: Level, native: u32) -> Result<()> {
    match level {
        Level::PointerSpeed => set(
            SPI_SETMOUSESPEED,
            0,
            without_provenance_mut(native as usize),
        ),
        Level::DoubleClick => set(SPI_SETDOUBLECLICKTIME, native, null_mut()),
        Level::TouchpadSpeed => write_touchpad(|parameters| parameters.cursor_speed = native),
        Level::RepeatDelay => set(SPI_SETKEYBOARDDELAY, native, null_mut()),
        Level::RepeatRate => set(SPI_SETKEYBOARDSPEED, native, null_mut()),
    }
}

fn read_acceleration() -> Result<[i32; 3]> {
    let mut acceleration = [0i32; 3];
    get(SPI_GETMOUSE, 0, &mut acceleration)?;
    Ok(acceleration)
}

fn write_acceleration(mut values: [i32; 3]) -> Result<()> {
    set(SPI_SETMOUSE, 0, values.as_mut_ptr().cast())
}

fn read_flag(switch: Switch) -> Result<bool> {
    let touchpad_bit =
        |bit: u32| -> Result<bool> { Ok(touchpad_parameters()?.settings & bit != 0) };
    match switch {
        Switch::Acceleration => Ok(read_acceleration()?[2] != 0),
        Switch::SwapButtons => Ok(unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0),
        Switch::MouseNaturalScroll => bail!(NO_MOUSE_NATURAL_SCROLL),
        Switch::Touchpad => Ok(touchpad_parameters()?.status & TOUCHPAD_ENABLED != 0),
        Switch::TapToClick => touchpad_bit(TAP_ENABLED),
        Switch::TouchpadNaturalScroll => touchpad_bit(SCROLL_DIRECTION_REVERSED),
    }
}

fn write_flag(switch: Switch, native: bool) -> Result<()> {
    let touchpad_bit = |bit: u32| {
        write_touchpad(|parameters| {
            parameters.settings = with_bit(parameters.settings, bit, native)
        })
    };
    match switch {
        Switch::Acceleration => write_acceleration(if native {
            ACCELERATION_ON
        } else {
            ACCELERATION_OFF
        }),
        Switch::SwapButtons => set(SPI_SETMOUSEBUTTONSWAP, u32::from(native), null_mut()),
        Switch::MouseNaturalScroll => bail!(NO_MOUSE_NATURAL_SCROLL),
        Switch::Touchpad => switch_touchpad(native),
        Switch::TapToClick => touchpad_bit(TAP_ENABLED),
        Switch::TouchpadNaturalScroll => touchpad_bit(SCROLL_DIRECTION_REVERSED),
    }
}

fn touchpad_from(parameters: &TouchpadParameters) -> Touchpad {
    if parameters.status & TOUCHPAD_PRESENT == 0 {
        return Touchpad::missing(if parameters.status & LEGACY_TOUCHPAD_PRESENT != 0 {
            LEGACY_TOUCHPAD
        } else {
            NO_TOUCHPAD
        });
    }
    Touchpad {
        enabled: Some(parameters.status & TOUCHPAD_ENABLED != 0),
        tap_to_click: Some(parameters.settings & TAP_ENABLED != 0),
        natural_scroll: Some(parameters.settings & SCROLL_DIRECTION_REVERSED == 0),
        speed: Some(step_percent(parameters.cursor_speed)),
        missing: None,
    }
}

fn switch_touchpad(on: bool) -> Result<()> {
    let enabled = || read_flag(Switch::Touchpad);
    if enabled()? == on {
        return Ok(());
    }
    settle(|| Ok(!modifiers_held()))?;
    press(&TOUCHPAD_SHORTCUT)?;
    if settle(|| Ok(enabled()? == on))? {
        return Ok(());
    }
    bail!(
        "Windows did not turn the touchpad {}",
        if on { "on" } else { "off" }
    )
}

fn settle(mut done: impl FnMut() -> Result<bool>) -> Result<bool> {
    let deadline = Instant::now() + SHORTCUT_SETTLE;
    loop {
        if done()? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        sleep(SHORTCUT_POLL);
    }
}

fn modifiers_held() -> bool {
    MODIFIERS
        .iter()
        .any(|&key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0)
}

fn press(keys: &[VIRTUAL_KEY]) -> Result<()> {
    let inputs = chord(keys);
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        bail!("SendInput failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

fn chord(keys: &[VIRTUAL_KEY]) -> Vec<INPUT> {
    let down = keys.iter().map(|&key| key_input(key, 0));
    let up = keys
        .iter()
        .rev()
        .map(|&key| key_input(key, KEYEVENTF_KEYUP));
    down.chain(up).collect()
}

fn key_input(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
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
    call(action, ui_param, pv_param, SPIF_UPDATEINIFILE)?;
    if unsafe { SendNotifyMessageW(HWND_BROADCAST, WM_SETTINGCHANGE, action as usize, 0) } == 0 {
        log::warn!(
            "WM_SETTINGCHANGE for {action:#06x} was not announced: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
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
    fn every_value_windows_accepts_reads_back_unchanged() {
        let read_back = |level, value| match level {
            Level::PointerSpeed | Level::TouchpadSpeed => step_percent(percent_step(value)),
            Level::DoubleClick => value,
            Level::RepeatDelay => delay_ms(delay_step(value)),
            Level::RepeatRate => rate_from_step(rate_step(value)),
        };
        assert_eq!(values(Level::RepeatRate), Steps::span(3, 30));
        assert_eq!(values(Level::RepeatDelay).step, 250);
        assert_eq!(values(Level::PointerSpeed).first, 5);
        for level in Level::ALL {
            let Steps { first, last, step } = values(level);
            for value in (first..=last).step_by(step as usize) {
                assert_eq!(read_back(level, value), value, "{level:?}");
            }
        }
    }

    #[test]
    fn the_default_unreversed_scroll_direction_reads_as_natural_scrolling() {
        let touchpad = TouchpadParameters {
            status: TOUCHPAD_PRESENT | TOUCHPAD_ENABLED,
            settings: TAP_ENABLED,
            cursor_speed: 10,
            ..TouchpadParameters::default()
        };
        assert_eq!(
            touchpad_from(&touchpad),
            Touchpad {
                enabled: Some(true),
                tap_to_click: Some(true),
                natural_scroll: Some(true),
                speed: Some(50),
                missing: None,
            }
        );
    }

    #[test]
    fn a_reversed_switched_off_touchpad_reads_off_with_traditional_scrolling() {
        let touchpad = touchpad_from(&TouchpadParameters {
            status: TOUCHPAD_PRESENT,
            settings: SCROLL_DIRECTION_REVERSED,
            cursor_speed: 20,
            ..TouchpadParameters::default()
        });
        assert_eq!(touchpad.enabled, Some(false));
        assert_eq!(touchpad.tap_to_click, Some(false));
        assert_eq!(touchpad.natural_scroll, Some(false));
        assert_eq!(touchpad.speed, Some(100));
        assert_eq!(
            touchpad_from(&TouchpadParameters::default()),
            Touchpad::missing(NO_TOUCHPAD)
        );
    }

    #[test]
    fn a_touchpad_that_is_not_precision_is_named_instead_of_reported_missing() {
        let legacy = touchpad_from(&TouchpadParameters {
            status: LEGACY_TOUCHPAD_PRESENT,
            ..TouchpadParameters::default()
        });
        assert_eq!(legacy, Touchpad::missing(LEGACY_TOUCHPAD));
        let both = touchpad_from(&TouchpadParameters {
            status: TOUCHPAD_PRESENT | LEGACY_TOUCHPAD_PRESENT,
            ..TouchpadParameters::default()
        });
        assert_eq!(both.missing, None);
    }

    #[test]
    fn the_touchpad_shortcut_presses_ctrl_win_f24_and_lets_go_in_reverse() {
        let events: Vec<(VIRTUAL_KEY, bool)> = chord(&TOUCHPAD_SHORTCUT)
            .iter()
            .map(|input| {
                assert_eq!(input.r#type, INPUT_KEYBOARD);
                let key = unsafe { input.Anonymous.ki };
                (key.wVk, key.dwFlags & KEYEVENTF_KEYUP != 0)
            })
            .collect();
        assert_eq!(
            events,
            [
                (VK_CONTROL, false),
                (VK_LWIN, false),
                (VK_F24, false),
                (VK_F24, true),
                (VK_LWIN, true),
                (VK_CONTROL, true),
            ]
        );
    }

    #[test]
    fn a_saved_value_of_the_wrong_shape_is_refused_before_the_host_is_touched() {
        let error = restore(
            Setting::Level(Level::PointerSpeed),
            Baseline::Text("fast".to_string()),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("pointer-speed"), "{error}");
        let error = restore(
            Setting::Switch(Switch::Acceleration),
            Baseline::Numbers(vec![6, 10]),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("[6, 10]"), "{error}");
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
