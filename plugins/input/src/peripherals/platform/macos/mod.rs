mod apply;
mod prefs;

use anyhow::{bail, Context, Result};

use super::super::{
    Baseline, Keyboard, Level, Mouse, Setting, Steps, Switch, Touchpad, Values, NO_TOUCHPAD,
};
use prefs::{Pref, Value};

const NO_BUTTON_SWAP: &str = "macOS has no setting that swaps the buttons of a mouse";
const NO_TOUCHPAD_SWITCH: &str = "macOS has no switch for the touchpad, so qol cannot turn it off";
const FASTEST_SCALING: f64 = 3.0;
const KEY_TICKS_PER_SECOND: f64 = 60.0;
const DELAY_STEP_MS: u32 = 50;
const RATES: [u32; 10] = [2, 3, 4, 5, 6, 10, 12, 15, 20, 30];

const MOUSE_SCALING: Pref = Pref::global("com.apple.mouse.scaling", Value::Float(0.6875));
const LINEAR_MOUSE: Pref = Pref::global("com.apple.mouse.linear", Value::Bool(false));
const DOUBLE_CLICK: Pref = Pref::global("com.apple.mouse.doubleClickThreshold", Value::Float(0.5));
const NATURAL_SCROLL: Pref = Pref::global("com.apple.swipescrolldirection", Value::Bool(true));
const TRACKPAD_SCALING: Pref = Pref::global("com.apple.trackpad.scaling", Value::Float(0.6875));
const INITIAL_KEY_REPEAT: Pref = Pref::global("InitialKeyRepeat", Value::Int(30));
const KEY_REPEAT: Pref = Pref::global("KeyRepeat", Value::Int(5));
const TAP_TO_CLICK: [Pref; 3] = [
    Pref::app(
        "com.apple.AppleMultitouchTrackpad",
        "Clicking",
        Value::Int(0),
    ),
    Pref::app(
        "com.apple.driver.AppleBluetoothMultitouch.trackpad",
        "Clicking",
        Value::Int(0),
    ),
    Pref::global_this_mac("com.apple.mouse.tapBehavior", Value::Int(0)),
];

pub(crate) fn backend() -> Result<&'static str> {
    Ok(qol_platform::Os::Macos.label())
}

pub(crate) fn unsupported(setting: Setting) -> Option<&'static str> {
    match setting {
        Setting::Switch(Switch::SwapButtons) => Some(NO_BUTTON_SWAP),
        Setting::Switch(Switch::Touchpad) => Some(NO_TOUCHPAD_SWITCH),
        _ => None,
    }
}

pub(crate) fn mouse() -> Result<Mouse> {
    Ok(Mouse {
        speed: Some(scaling_percent(MOUSE_SCALING.read()?.number())),
        acceleration: Some(!LINEAR_MOUSE.read()?.flag()),
        swap_buttons: None,
        double_click: Some(seconds_ms(DOUBLE_CLICK.read()?.number())),
        natural_scroll: Some(NATURAL_SCROLL.read()?.flag()),
        ..Default::default()
    })
}

pub(crate) fn touchpad() -> Result<Touchpad> {
    if !apply::trackpad_present() {
        return Ok(Touchpad::missing(NO_TOUCHPAD));
    }
    Ok(Touchpad {
        enabled: None,
        tap_to_click: Some(TAP_TO_CLICK[0].read()?.flag()),
        natural_scroll: Some(NATURAL_SCROLL.read()?.flag()),
        speed: Some(scaling_percent(TRACKPAD_SCALING.read()?.number())),
        missing: None,
        ..Default::default()
    })
}

pub(crate) fn keyboard() -> Result<Keyboard> {
    Ok(Keyboard {
        repeat_delay: Some(ticks_ms(INITIAL_KEY_REPEAT.read()?.number())),
        repeat_rate: Some(ticks_rate(KEY_REPEAT.read()?.number())),
        ..Default::default()
    })
}

pub(crate) fn set_switch(switch: Switch, on: bool) -> Result<()> {
    match switch {
        Switch::Acceleration => write(Setting::Switch(switch), &[Value::Bool(!on)]),
        Switch::TapToClick => write(Setting::Switch(switch), &[Value::Int(i64::from(on)); 3]),
        Switch::MouseNaturalScroll | Switch::TouchpadNaturalScroll => {
            write(Setting::Switch(switch), &[Value::Bool(on)])
        }
        Switch::SwapButtons => bail!(NO_BUTTON_SWAP),
        Switch::Touchpad => bail!(NO_TOUCHPAD_SWITCH),
    }
}

pub(crate) fn set_level(level: Level, value: u32) -> Result<()> {
    let native = match level {
        Level::PointerSpeed | Level::TouchpadSpeed => Value::Float(percent_scaling(value)),
        Level::DoubleClick => Value::Float(f64::from(value) / 1000.0),
        Level::RepeatDelay => Value::Int(ms_ticks(value)),
        Level::RepeatRate => Value::Int(rate_ticks(value)),
    };
    write(Setting::Level(level), &[native])
}

pub(crate) fn capture(setting: Setting) -> Result<Baseline> {
    let saved = prefs(setting)?
        .iter()
        .map(Pref::raw)
        .collect::<Result<Vec<_>>>()?;
    Ok(Baseline::Text(serde_json::to_string(&saved)?))
}

pub(crate) fn restore(setting: Setting, baseline: Baseline) -> Result<()> {
    let prefs = prefs(setting)?;
    let saved = match baseline {
        Baseline::Text(text) => {
            serde_json::from_str::<Vec<Option<Value>>>(&text).with_context(|| {
                format!("the saved {} is not a macOS setting value", setting.name())
            })?
        }
        _ => bail!("the saved {} is not a macOS setting value", setting.name()),
    };
    if saved.len() != prefs.len() {
        bail!(
            "the saved {} holds {} values, not {}",
            setting.name(),
            saved.len(),
            prefs.len()
        );
    }
    let values: Vec<Value> = prefs
        .iter()
        .zip(&saved)
        .map(|(pref, value)| value.unwrap_or(pref.default))
        .collect();
    write(setting, &values)?;
    for (pref, value) in prefs.iter().zip(saved) {
        if value.is_none() {
            pref.write(None)?;
        }
    }
    Ok(())
}

pub(crate) fn values(level: Level) -> Values {
    match level {
        Level::RepeatDelay => Values::Steps(Steps {
            first: level.bounds().first,
            last: level.bounds().last,
            step: DELAY_STEP_MS,
        }),
        Level::RepeatRate => Values::Only(&RATES),
        _ => Values::Steps(level.bounds()),
    }
}

pub(crate) fn owner(setting: Setting) -> Setting {
    match setting {
        Setting::Switch(Switch::TouchpadNaturalScroll) => {
            Setting::Switch(Switch::MouseNaturalScroll)
        }
        setting => setting,
    }
}

fn prefs(setting: Setting) -> Result<&'static [Pref]> {
    Ok(match setting {
        Setting::Level(Level::PointerSpeed) => &[MOUSE_SCALING],
        Setting::Level(Level::DoubleClick) => &[DOUBLE_CLICK],
        Setting::Level(Level::TouchpadSpeed) => &[TRACKPAD_SCALING],
        Setting::Level(Level::RepeatDelay) => &[INITIAL_KEY_REPEAT],
        Setting::Level(Level::RepeatRate) => &[KEY_REPEAT],
        Setting::Switch(Switch::Acceleration) => &[LINEAR_MOUSE],
        Setting::Switch(Switch::MouseNaturalScroll | Switch::TouchpadNaturalScroll) => {
            &[NATURAL_SCROLL]
        }
        Setting::Switch(Switch::TapToClick) => &TAP_TO_CLICK,
        Setting::Switch(Switch::SwapButtons) => bail!(NO_BUTTON_SWAP),
        Setting::Switch(Switch::Touchpad) => bail!(NO_TOUCHPAD_SWITCH),
    })
}

fn write(setting: Setting, values: &[Value]) -> Result<()> {
    for (pref, value) in prefs(setting)?.iter().zip(values) {
        pref.write(Some(*value))?;
    }
    match owner(setting) {
        Setting::Switch(Switch::MouseNaturalScroll) => apply::scroll_direction(values[0].flag()),
        _ => apply::settings(),
    }
}

fn scaling_percent(scaling: f64) -> u32 {
    (scaling.clamp(0.0, FASTEST_SCALING) * 100.0 / FASTEST_SCALING).round() as u32
}

fn percent_scaling(percent: u32) -> f64 {
    f64::from(percent.min(100)) * FASTEST_SCALING / 100.0
}

fn seconds_ms(seconds: f64) -> u32 {
    (seconds.max(0.0) * 1000.0).round() as u32
}

fn ticks_ms(ticks: f64) -> u32 {
    (ticks.max(0.0) * 1000.0 / KEY_TICKS_PER_SECOND).round() as u32
}

fn ms_ticks(ms: u32) -> i64 {
    (f64::from(ms) * KEY_TICKS_PER_SECOND / 1000.0).round() as i64
}

fn ticks_rate(ticks: f64) -> u32 {
    (KEY_TICKS_PER_SECOND / ticks.max(1.0)).round() as u32
}

fn rate_ticks(rate: u32) -> i64 {
    (KEY_TICKS_PER_SECOND / f64::from(rate.max(1))).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_macos_accepts_reads_back_unchanged() {
        let read_back = |level, value| match level {
            Level::PointerSpeed | Level::TouchpadSpeed => scaling_percent(percent_scaling(value)),
            Level::DoubleClick => seconds_ms(f64::from(value) / 1000.0),
            Level::RepeatDelay => ticks_ms(ms_ticks(value) as f64),
            Level::RepeatRate => ticks_rate(rate_ticks(value) as f64),
        };
        for level in Level::ALL {
            for value in values(level).each() {
                assert_eq!(read_back(level, value), value, "{level:?}");
            }
        }
    }

    #[test]
    fn the_settings_macos_ships_with_read_as_its_own_panel_shows_them() {
        assert_eq!(scaling_percent(MOUSE_SCALING.default.number()), 23);
        assert_eq!(seconds_ms(DOUBLE_CLICK.default.number()), 500);
        assert_eq!(ticks_ms(INITIAL_KEY_REPEAT.default.number()), 500);
        assert_eq!(ticks_rate(KEY_REPEAT.default.number()), 12);
        assert_eq!(ticks_ms(15.0), 250);
        assert_eq!(ticks_rate(2.0), 30);
    }

    #[test]
    fn a_legacy_negative_scaling_reads_as_the_slowest_speed() {
        assert_eq!(scaling_percent(-1.0), 0);
        assert_eq!(scaling_percent(3.0), 100);
        assert_eq!(percent_scaling(100), 3.0);
    }

    #[test]
    fn mouse_and_touchpad_natural_scrolling_share_one_saved_value() {
        assert_eq!(
            owner(Setting::Switch(Switch::TouchpadNaturalScroll)),
            Setting::Switch(Switch::MouseNaturalScroll)
        );
        assert_eq!(
            prefs(Setting::Switch(Switch::TouchpadNaturalScroll)).unwrap(),
            prefs(Setting::Switch(Switch::MouseNaturalScroll)).unwrap()
        );
        for setting in Setting::ALL {
            if setting != Setting::Switch(Switch::TouchpadNaturalScroll) {
                assert_eq!(owner(setting), setting);
            }
        }
    }

    #[test]
    fn tap_to_click_writes_every_place_macos_reads_it_from() {
        let keys: Vec<&str> = TAP_TO_CLICK.iter().map(|pref| pref.key).collect();
        assert_eq!(
            keys,
            ["Clicking", "Clicking", "com.apple.mouse.tapBehavior"]
        );
    }

    #[test]
    fn a_setting_macos_lacks_is_refused_before_the_host_is_touched() {
        let error = set_switch(Switch::SwapButtons, true)
            .unwrap_err()
            .to_string();
        assert_eq!(error, NO_BUTTON_SWAP);
        let error = set_switch(Switch::Touchpad, false).unwrap_err().to_string();
        assert_eq!(error, NO_TOUCHPAD_SWITCH);
        let error = restore(Setting::Level(Level::RepeatRate), Baseline::Number(5))
            .unwrap_err()
            .to_string();
        assert!(error.contains("repeat-rate"), "{error}");
        let error = restore(
            Setting::Switch(Switch::TapToClick),
            Baseline::Text(r#"[{"int":1}]"#.to_string()),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("holds 1 values, not 3"), "{error}");
    }
}
