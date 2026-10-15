use std::collections::BTreeMap;

use anyhow::{bail, Result};
use qol_desktop_settings::desktop::{classify_desktop, DesktopEnvironment};
use qol_desktop_settings::gsettings;

use super::super::{
    Baseline, Keyboard, Level, Mouse, Setting, Switch, Touchpad, Values, NO_TOUCHPAD,
};

const UDEV_DATA: &str = "/run/udev/data";
const INPUT_DEVICE_PREFIX: &str = "c13:";
const TOUCHPAD_PROPERTY: &str = "E:ID_INPUT_TOUCHPAD=1";

struct Desktop {
    name: &'static str,
    mouse: &'static str,
    touchpad: &'static str,
    keyboard: &'static str,
}

const CINNAMON: Desktop = Desktop {
    name: "Cinnamon",
    mouse: "org.cinnamon.desktop.peripherals.mouse",
    touchpad: "org.cinnamon.desktop.peripherals.touchpad",
    keyboard: "org.cinnamon.desktop.peripherals.keyboard",
};

const GNOME: Desktop = Desktop {
    name: "GNOME",
    mouse: "org.gnome.desktop.peripherals.mouse",
    touchpad: "org.gnome.desktop.peripherals.touchpad",
    keyboard: "org.gnome.desktop.peripherals.keyboard",
};

struct Values(BTreeMap<String, String>);

impl Values {
    fn read(schema: &str) -> Result<Self> {
        Ok(Self(gsettings::list(schema)?.into_iter().collect()))
    }

    fn flag(&self, key: &str) -> Option<bool> {
        match self.0.get(key)?.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    fn number(&self, key: &str) -> Option<f64> {
        self.0.get(key)?.split_whitespace().last()?.parse().ok()
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

pub(crate) fn backend() -> Result<&'static str> {
    Ok(desktop()?.name)
}

pub(crate) fn mouse() -> Result<Mouse> {
    let values = Values::read(desktop()?.mouse)?;
    Ok(Mouse {
        speed: values.number("speed").map(speed_percent),
        acceleration: values
            .text("accel-profile")
            .map(|profile| profile != "flat"),
        swap_buttons: values.flag("left-handed"),
        double_click: values.number("double-click").map(whole),
        natural_scroll: values.flag("natural-scroll"),
    })
}

pub(crate) fn touchpad() -> Result<Touchpad> {
    let desktop = desktop()?;
    if let Some(reason) = touchpad_missing() {
        return Ok(Touchpad::missing(reason));
    }
    let values = Values::read(desktop.touchpad)?;
    Ok(Touchpad {
        enabled: values.text("send-events").map(|mode| mode != "disabled"),
        tap_to_click: values.flag("tap-to-click"),
        natural_scroll: values.flag("natural-scroll"),
        speed: values.number("speed").map(speed_percent),
        missing: None,
    })
}

pub(crate) fn keyboard() -> Result<Keyboard> {
    let values = Values::read(desktop()?.keyboard)?;
    Ok(Keyboard {
        repeat_delay: values.number("delay").map(whole),
        repeat_rate: values
            .number("repeat-interval")
            .and_then(rate_from_interval),
    })
}

pub(crate) fn set_switch(switch: Switch, on: bool) -> Result<()> {
    let (schema, key, value) = switch_write(desktop()?, switch, on);
    gsettings::set(schema, key, value)
}

pub(crate) fn set_level(level: Level, value: u32) -> Result<()> {
    let (schema, key, value) = level_write(desktop()?, level, value);
    gsettings::set(schema, key, &value)
}

pub(crate) fn values(level: Level) -> Values {
    Values::Steps(level.bounds())
}

pub(crate) fn owner(setting: Setting) -> Setting {
    setting
}

pub(crate) fn capture(setting: Setting) -> Result<Baseline> {
    let (schema, key) = location(desktop()?, setting);
    Ok(Baseline::Text(gsettings::get(schema, key)?))
}

pub(crate) fn restore(setting: Setting, baseline: Baseline) -> Result<()> {
    let Baseline::Text(value) = baseline else {
        bail!(
            "the saved {} is not a desktop setting value",
            setting.name()
        );
    };
    let (schema, key) = location(desktop()?, setting);
    gsettings::set(schema, key, &value)
}

fn desktop() -> Result<&'static Desktop> {
    desktop_for(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
}

fn desktop_for(raw: &str) -> Result<&'static Desktop> {
    match classify_desktop(raw) {
        DesktopEnvironment::Cinnamon => Ok(&CINNAMON),
        DesktopEnvironment::Gnome => Ok(&GNOME),
        _ if raw.trim().is_empty() => {
            bail!("input settings need a Cinnamon or GNOME session, and XDG_CURRENT_DESKTOP is not set")
        }
        _ => bail!("input settings work on Cinnamon and GNOME, not on {raw}"),
    }
}

fn location(desktop: &Desktop, setting: Setting) -> (&'static str, &'static str) {
    match setting {
        Setting::Switch(Switch::Acceleration) => (desktop.mouse, "accel-profile"),
        Setting::Switch(Switch::SwapButtons) => (desktop.mouse, "left-handed"),
        Setting::Switch(Switch::MouseNaturalScroll) => (desktop.mouse, "natural-scroll"),
        Setting::Switch(Switch::Touchpad) => (desktop.touchpad, "send-events"),
        Setting::Switch(Switch::TapToClick) => (desktop.touchpad, "tap-to-click"),
        Setting::Switch(Switch::TouchpadNaturalScroll) => (desktop.touchpad, "natural-scroll"),
        Setting::Level(Level::PointerSpeed) => (desktop.mouse, "speed"),
        Setting::Level(Level::DoubleClick) => (desktop.mouse, "double-click"),
        Setting::Level(Level::TouchpadSpeed) => (desktop.touchpad, "speed"),
        Setting::Level(Level::RepeatDelay) => (desktop.keyboard, "delay"),
        Setting::Level(Level::RepeatRate) => (desktop.keyboard, "repeat-interval"),
    }
}

fn switch_write(
    desktop: &Desktop,
    switch: Switch,
    on: bool,
) -> (&'static str, &'static str, &'static str) {
    let (schema, key) = location(desktop, Setting::Switch(switch));
    let value = match (switch, on) {
        (Switch::Acceleration, true) => "adaptive",
        (Switch::Acceleration, false) => "flat",
        (Switch::Touchpad, true) => "enabled",
        (Switch::Touchpad, false) => "disabled",
        (_, true) => "true",
        (_, false) => "false",
    };
    (schema, key, value)
}

fn level_write(
    desktop: &Desktop,
    level: Level,
    value: u32,
) -> (&'static str, &'static str, String) {
    let (schema, key) = location(desktop, Setting::Level(level));
    let value = match level {
        Level::PointerSpeed | Level::TouchpadSpeed => percent_speed(value),
        Level::DoubleClick | Level::RepeatDelay => value.to_string(),
        Level::RepeatRate => interval_from_rate(value).to_string(),
    };
    (schema, key, value)
}

fn touchpad_missing() -> Option<String> {
    let entries = match std::fs::read_dir(UDEV_DATA) {
        Ok(entries) => entries,
        Err(error) => {
            return Some(format!(
                "cannot tell whether a touchpad is connected: {UDEV_DATA}: {error}"
            ))
        }
    };
    let found = entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with(INPUT_DEVICE_PREFIX)
            && std::fs::read_to_string(entry.path()).is_ok_and(|text| marks_touchpad(&text))
    });
    (!found).then(|| NO_TOUCHPAD.to_string())
}

fn marks_touchpad(udev: &str) -> bool {
    udev.lines().any(|line| line.trim() == TOUCHPAD_PROPERTY)
}

fn speed_percent(speed: f64) -> u32 {
    ((speed.clamp(-1.0, 1.0) + 1.0) * 50.0).round() as u32
}

fn percent_speed(percent: u32) -> String {
    format!("{:.2}", f64::from(percent) / 50.0 - 1.0)
}

fn whole(value: f64) -> u32 {
    value.max(0.0).round() as u32
}

fn rate_from_interval(interval: f64) -> Option<u32> {
    (interval > 0.0).then(|| (1000.0 / interval).round() as u32)
}

fn interval_from_rate(rate: u32) -> u32 {
    (1000.0 / f64::from(rate.max(1))).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cinnamon_and_gnome_sessions_pick_their_own_schemas() {
        assert_eq!(desktop_for("X-Cinnamon").unwrap().name, "Cinnamon");
        assert_eq!(desktop_for("ubuntu:GNOME").unwrap().name, "GNOME");
        assert!(desktop_for("X-Cinnamon")
            .unwrap()
            .mouse
            .starts_with("org.cinnamon."));
        assert!(desktop_for("GNOME")
            .unwrap()
            .keyboard
            .starts_with("org.gnome."));
    }

    #[test]
    fn other_desktops_are_refused_by_name() {
        let error = desktop_for("KDE").err().unwrap().to_string();
        assert!(error.contains("KDE"), "{error}");
        let error = desktop_for("Hyprland").err().unwrap().to_string();
        assert!(error.contains("Hyprland"), "{error}");
        let error = desktop_for(" ").err().unwrap().to_string();
        assert!(error.contains("XDG_CURRENT_DESKTOP"), "{error}");
    }

    #[test]
    fn values_read_gsettings_text_as_typed_settings() {
        let values = Values(BTreeMap::from([
            ("left-handed".to_string(), "true".to_string()),
            ("speed".to_string(), "-0.5".to_string()),
            ("delay".to_string(), "uint32 500".to_string()),
            ("accel-profile".to_string(), "flat".to_string()),
            ("broken".to_string(), "maybe".to_string()),
        ]));
        assert_eq!(values.flag("left-handed"), Some(true));
        assert_eq!(values.flag("broken"), None);
        assert_eq!(values.flag("absent"), None);
        assert_eq!(values.number("speed"), Some(-0.5));
        assert_eq!(values.number("delay"), Some(500.0));
        assert_eq!(values.number("broken"), None);
        assert_eq!(values.text("accel-profile"), Some("flat"));
    }

    #[test]
    fn pointer_speed_maps_the_full_range_onto_percent_and_back() {
        assert_eq!(speed_percent(-1.0), 0);
        assert_eq!(speed_percent(0.0), 50);
        assert_eq!(speed_percent(1.0), 100);
        assert_eq!(speed_percent(4.0), 100);
        assert_eq!(percent_speed(0), "-1.00");
        assert_eq!(percent_speed(50), "0.00");
        assert_eq!(percent_speed(75), "0.50");
        for percent in 0..=100 {
            let speed: f64 = percent_speed(percent).parse().unwrap();
            assert_eq!(speed_percent(speed), percent);
        }
    }

    #[test]
    fn repeat_rate_round_trips_through_the_interval() {
        assert_eq!(rate_from_interval(30.0), Some(33));
        assert_eq!(rate_from_interval(0.0), None);
        assert_eq!(interval_from_rate(33), 30);
        assert_eq!(interval_from_rate(50), 20);
        assert_eq!(interval_from_rate(2), 500);
        for rate in [2, 10, 25, 33, 40] {
            assert_eq!(
                rate_from_interval(f64::from(interval_from_rate(rate))),
                Some(rate)
            );
        }
    }

    #[test]
    fn switches_write_the_keys_the_desktop_reads() {
        let on = |switch| switch_write(&CINNAMON, switch, true);
        let off = |switch| switch_write(&CINNAMON, switch, false);
        assert_eq!(
            off(Switch::Acceleration),
            (CINNAMON.mouse, "accel-profile", "flat")
        );
        assert_eq!(
            on(Switch::Acceleration),
            (CINNAMON.mouse, "accel-profile", "adaptive")
        );
        assert_eq!(
            on(Switch::SwapButtons),
            (CINNAMON.mouse, "left-handed", "true")
        );
        assert_eq!(
            off(Switch::Touchpad),
            (CINNAMON.touchpad, "send-events", "disabled")
        );
        assert_eq!(
            on(Switch::Touchpad),
            (CINNAMON.touchpad, "send-events", "enabled")
        );
        assert_eq!(
            off(Switch::TapToClick),
            (CINNAMON.touchpad, "tap-to-click", "false")
        );
        assert_eq!(
            on(Switch::TouchpadNaturalScroll),
            (CINNAMON.touchpad, "natural-scroll", "true")
        );
        assert_eq!(
            on(Switch::MouseNaturalScroll),
            (CINNAMON.mouse, "natural-scroll", "true")
        );
    }

    #[test]
    fn levels_write_the_keys_the_desktop_reads() {
        assert_eq!(
            level_write(&GNOME, Level::PointerSpeed, 75),
            (GNOME.mouse, "speed", "0.50".to_string())
        );
        assert_eq!(
            level_write(&GNOME, Level::DoubleClick, 450),
            (GNOME.mouse, "double-click", "450".to_string())
        );
        assert_eq!(
            level_write(&GNOME, Level::TouchpadSpeed, 25),
            (GNOME.touchpad, "speed", "-0.50".to_string())
        );
        assert_eq!(
            level_write(&GNOME, Level::RepeatDelay, 300),
            (GNOME.keyboard, "delay", "300".to_string())
        );
        assert_eq!(
            level_write(&GNOME, Level::RepeatRate, 25),
            (GNOME.keyboard, "repeat-interval", "40".to_string())
        );
    }

    #[test]
    fn every_setting_saves_and_puts_back_its_own_key() {
        let mut keys: Vec<_> = Setting::ALL
            .into_iter()
            .map(|setting| location(&CINNAMON, setting))
            .collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), Setting::ALL.len());
        let error = restore(Setting::ALL[0], Baseline::Number(5))
            .unwrap_err()
            .to_string();
        assert!(error.contains("pointer-speed"), "{error}");
    }

    #[test]
    fn only_a_touchpad_tag_marks_a_touchpad() {
        assert!(marks_touchpad("E:ID_INPUT=1\nE:ID_INPUT_TOUCHPAD=1\n"));
        assert!(!marks_touchpad("E:ID_INPUT=1\nE:ID_INPUT_MOUSE=1\n"));
        assert!(!marks_touchpad("E:ID_INPUT_TOUCHPAD=0\n"));
        assert!(!marks_touchpad("E:ID_INPUT_TOUCHSCREEN=1\n"));
    }
}
