mod platform;

use anyhow::{bail, Result};
use serde::Serialize;

pub(crate) const NO_TOUCHPAD: &str = "no touchpad is connected";
pub(crate) const TOUCHPAD_FIXED: &str =
    "this computer does not let apps turn the touchpad on or off";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Mouse {
    pub speed: Option<u32>,
    pub acceleration: Option<bool>,
    pub swap_buttons: Option<bool>,
    pub double_click: Option<u32>,
    pub natural_scroll: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Touchpad {
    pub enabled: Option<bool>,
    pub tap_to_click: Option<bool>,
    pub natural_scroll: Option<bool>,
    pub speed: Option<u32>,
    pub missing: Option<String>,
}

impl Touchpad {
    pub fn missing(reason: impl Into<String>) -> Self {
        Self {
            missing: Some(reason.into()),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Keyboard {
    pub repeat_delay: Option<u32>,
    pub repeat_rate: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub backend: &'static str,
    pub mouse: Mouse,
    pub touchpad: Touchpad,
    pub keyboard: Keyboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Acceleration,
    SwapButtons,
    MouseNaturalScroll,
    Touchpad,
    TapToClick,
    TouchpadNaturalScroll,
}

impl Switch {
    pub const ALL: [Self; 6] = [
        Self::Acceleration,
        Self::SwapButtons,
        Self::MouseNaturalScroll,
        Self::Touchpad,
        Self::TapToClick,
        Self::TouchpadNaturalScroll,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Acceleration => "acceleration",
            Self::SwapButtons => "swap_buttons",
            Self::MouseNaturalScroll => "mouse_natural_scroll",
            Self::Touchpad => "touchpad",
            Self::TapToClick => "tap_to_click",
            Self::TouchpadNaturalScroll => "touchpad_natural_scroll",
        }
    }

    pub fn read(self, status: &Status) -> Option<bool> {
        match self {
            Self::Acceleration => status.mouse.acceleration,
            Self::SwapButtons => status.mouse.swap_buttons,
            Self::MouseNaturalScroll => status.mouse.natural_scroll,
            Self::Touchpad => status.touchpad.enabled,
            Self::TapToClick => status.touchpad.tap_to_click,
            Self::TouchpadNaturalScroll => status.touchpad.natural_scroll,
        }
    }

    fn on_touchpad(self) -> bool {
        matches!(
            self,
            Self::Touchpad | Self::TapToClick | Self::TouchpadNaturalScroll
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Steps {
    pub first: u32,
    pub last: u32,
    pub step: u32,
}

impl Steps {
    pub const fn span(first: u32, last: u32) -> Self {
        Self {
            first,
            last,
            step: 1,
        }
    }

    pub fn contains(self, value: u32) -> bool {
        (self.first..=self.last).contains(&value) && (value - self.first).is_multiple_of(self.step)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    PointerSpeed,
    DoubleClick,
    TouchpadSpeed,
    RepeatDelay,
    RepeatRate,
}

impl Level {
    pub const ALL: [Self; 5] = [
        Self::PointerSpeed,
        Self::DoubleClick,
        Self::TouchpadSpeed,
        Self::RepeatDelay,
        Self::RepeatRate,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::PointerSpeed => "pointer_speed",
            Self::DoubleClick => "double_click",
            Self::TouchpadSpeed => "touchpad_speed",
            Self::RepeatDelay => "repeat_delay",
            Self::RepeatRate => "repeat_rate",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PointerSpeed => "pointer speed",
            Self::DoubleClick => "double-click time",
            Self::TouchpadSpeed => "touchpad speed",
            Self::RepeatDelay => "repeat delay",
            Self::RepeatRate => "repeat rate",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::PointerSpeed | Self::TouchpadSpeed => "%",
            Self::DoubleClick | Self::RepeatDelay => " ms",
            Self::RepeatRate => " keys/s",
        }
    }

    pub fn bounds(self) -> Steps {
        match self {
            Self::PointerSpeed | Self::TouchpadSpeed => Steps::span(0, 100),
            Self::DoubleClick | Self::RepeatDelay => Steps::span(100, 1000),
            Self::RepeatRate => Steps::span(2, 50),
        }
    }

    pub fn values(self) -> Steps {
        platform::values(self)
    }

    pub fn accepted(self) -> String {
        let Steps { first, last, step } = self.values();
        let unit = self.unit();
        match step {
            1 => format!("{first} to {last}{unit}"),
            step => format!("{first} to {last}{unit} in steps of {step}{unit}"),
        }
    }

    pub fn read(self, status: &Status) -> Option<u32> {
        match self {
            Self::PointerSpeed => status.mouse.speed,
            Self::DoubleClick => status.mouse.double_click,
            Self::TouchpadSpeed => status.touchpad.speed,
            Self::RepeatDelay => status.keyboard.repeat_delay,
            Self::RepeatRate => status.keyboard.repeat_rate,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Switch(Switch),
    Level(Level),
}

impl Setting {
    pub const ALL: [Self; 11] = [
        Self::Level(Level::PointerSpeed),
        Self::Switch(Switch::Acceleration),
        Self::Switch(Switch::SwapButtons),
        Self::Level(Level::DoubleClick),
        Self::Switch(Switch::MouseNaturalScroll),
        Self::Switch(Switch::Touchpad),
        Self::Switch(Switch::TapToClick),
        Self::Switch(Switch::TouchpadNaturalScroll),
        Self::Level(Level::TouchpadSpeed),
        Self::Level(Level::RepeatDelay),
        Self::Level(Level::RepeatRate),
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Switch(switch) => switch.key(),
            Self::Level(level) => level.key(),
        }
    }

    pub fn name(self) -> String {
        self.key().replace('_', "-")
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|setting| setting.name() == name || setting.key() == name)
    }
}

pub fn backend() -> Result<&'static str> {
    platform::backend()
}

pub fn mouse() -> Result<Mouse> {
    platform::mouse()
}

pub fn touchpad() -> Result<Touchpad> {
    platform::touchpad()
}

pub fn keyboard() -> Result<Keyboard> {
    platform::keyboard()
}

pub fn status() -> Result<Status> {
    Ok(Status {
        backend: backend()?,
        mouse: mouse()?,
        touchpad: touchpad()?,
        keyboard: keyboard()?,
    })
}

pub fn set_switch(switch: Switch, on: bool) -> Result<()> {
    if switch.on_touchpad() {
        require_touchpad()?;
    }
    platform::set_switch(switch, on)
}

pub fn set_level(level: Level, value: u32) -> Result<()> {
    if !level.values().contains(value) {
        bail!(
            "the {} takes a value from {}",
            level.label(),
            level.accepted()
        );
    }
    if level == Level::TouchpadSpeed {
        require_touchpad()?;
    }
    platform::set_level(level, value)
}

pub fn toggle_touchpad() -> Result<bool> {
    let touchpad = touchpad()?;
    if let Some(reason) = touchpad.missing {
        bail!("{reason}");
    }
    let Some(enabled) = touchpad.enabled else {
        bail!("{TOUCHPAD_FIXED}");
    };
    platform::set_switch(Switch::Touchpad, !enabled)?;
    Ok(!enabled)
}

fn require_touchpad() -> Result<()> {
    match touchpad()?.missing {
        Some(reason) => bail!("{reason}"),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_is_listed_once_and_parses_from_its_key_and_its_name() {
        for switch in Switch::ALL {
            let setting = Setting::Switch(switch);
            assert_eq!(Setting::ALL.iter().filter(|s| **s == setting).count(), 1);
        }
        for level in Level::ALL {
            let setting = Setting::Level(level);
            assert_eq!(Setting::ALL.iter().filter(|s| **s == setting).count(), 1);
        }
        for setting in Setting::ALL {
            assert_eq!(Setting::parse(setting.key()), Some(setting));
            assert_eq!(Setting::parse(&setting.name()), Some(setting));
        }
        assert_eq!(
            Setting::parse("pointer-speed"),
            Some(Setting::Level(Level::PointerSpeed))
        );
        assert_eq!(Setting::parse("speed"), None);
        assert_eq!(Setting::parse("pointer_speed-"), None);
        assert_eq!(Setting::parse(""), None);
    }

    #[test]
    fn a_level_outside_its_values_is_refused_before_the_host_is_touched() {
        for level in Level::ALL {
            let values = level.values();
            let between = (values.step > 1).then(|| values.first + 1);
            for value in values
                .first
                .checked_sub(1)
                .into_iter()
                .chain([values.last + 1])
                .chain(between)
            {
                let error = set_level(level, value).unwrap_err().to_string();
                assert!(error.contains(level.label()), "{error}");
                assert!(error.contains(&level.accepted()), "{error}");
            }
        }
    }

    #[test]
    fn this_computer_accepts_only_values_inside_the_bounds_the_settings_offer() {
        for level in Level::ALL {
            let values = level.values();
            let bounds = level.bounds();
            assert!(bounds.contains(values.first), "{level:?}");
            assert!(bounds.contains(values.last), "{level:?}");
            assert!(values.step.is_multiple_of(bounds.step), "{level:?}");
        }
    }

    #[test]
    fn values_hold_their_ends_and_steps_and_read_out_with_the_unit() {
        let quarters = Steps {
            first: 250,
            last: 1000,
            step: 250,
        };
        assert!(quarters.contains(250));
        assert!(quarters.contains(750));
        assert!(quarters.contains(1000));
        assert!(!quarters.contains(300));
        assert!(!quarters.contains(0));
        assert!(!quarters.contains(1250));
        assert!(Steps::span(2, 50).contains(33));
        assert!(!Steps::span(2, 50).contains(51));
        assert_eq!(
            Level::RepeatRate.accepted(),
            format!(
                "{} to {} keys/s",
                Level::RepeatRate.values().first,
                Level::RepeatRate.values().last
            )
        );
    }

    #[test]
    fn a_missing_touchpad_serializes_its_reason_and_null_values() {
        let value = serde_json::to_value(Touchpad::missing(NO_TOUCHPAD)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "enabled": null,
                "tap_to_click": null,
                "natural_scroll": null,
                "speed": null,
                "missing": NO_TOUCHPAD,
            })
        );
    }

    #[test]
    fn the_touchpad_switches_are_the_ones_that_need_a_touchpad() {
        let needing: Vec<Switch> = Switch::ALL
            .into_iter()
            .filter(|switch| switch.on_touchpad())
            .collect();
        assert_eq!(
            needing,
            [
                Switch::Touchpad,
                Switch::TapToClick,
                Switch::TouchpadNaturalScroll
            ]
        );
    }
}
