mod platform;
pub mod session;

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub(crate) const NO_TOUCHPAD: &str = "no touchpad is connected";
pub(crate) const TOUCHPAD_FIXED: &str =
    "this computer does not let apps turn the touchpad on or off";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Mouse {
    pub speed: Option<u32>,
    pub acceleration: Option<bool>,
    pub swap_buttons: Option<bool>,
    pub double_click: Option<u32>,
    pub natural_scroll: Option<bool>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub unavailable: BTreeMap<&'static str, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Touchpad {
    pub enabled: Option<bool>,
    pub tap_to_click: Option<bool>,
    pub natural_scroll: Option<bool>,
    pub speed: Option<u32>,
    pub missing: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub unavailable: BTreeMap<&'static str, String>,
}

impl Touchpad {
    pub fn missing(reason: impl Into<String>) -> Self {
        Self {
            missing: Some(reason.into()),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Keyboard {
    pub repeat_delay: Option<u32>,
    pub repeat_rate: Option<u32>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub unavailable: BTreeMap<&'static str, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peripheral {
    Mouse,
    Touchpad,
    Keyboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Device {
    pub id: &'static str,
    pub kind: &'static str,
    pub name: &'static str,
    pub picture: &'static str,
    pub status: &'static str,
}

const MOUSE: Device = Device {
    id: "mouse",
    kind: "mouse",
    name: "Mouse",
    picture: "mouse",
    status: "Every mouse",
};

const TOUCHPAD: Device = Device {
    id: "touchpad",
    kind: "touchpad",
    name: "Touchpad",
    picture: "trackpad",
    status: "Every touchpad",
};

const KEYBOARD: Device = Device {
    id: "keyboard",
    kind: "keyboard",
    name: "Keyboard",
    picture: "keyboard",
    status: "Every keyboard",
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub backend: &'static str,
    pub mouse: Mouse,
    pub touchpad: Touchpad,
    pub keyboard: Keyboard,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Baseline {
    Text(String),
    Number(u32),
    Numbers(Vec<i32>),
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
pub enum Values {
    Steps(Steps),
    Only(&'static [u32]),
}

impl Values {
    pub fn first(self) -> u32 {
        match self {
            Self::Steps(steps) => steps.first,
            Self::Only(values) => values[0],
        }
    }

    pub fn last(self) -> u32 {
        match self {
            Self::Steps(steps) => steps.last,
            Self::Only(values) => values[values.len() - 1],
        }
    }

    pub fn contains(self, value: u32) -> bool {
        match self {
            Self::Steps(steps) => steps.contains(value),
            Self::Only(values) => values.contains(&value),
        }
    }

    pub fn each(self) -> Vec<u32> {
        match self {
            Self::Steps(Steps { first, last, step }) => {
                (first..=last).step_by(step as usize).collect()
            }
            Self::Only(values) => values.to_vec(),
        }
    }

    pub fn refused_between(self) -> Option<u32> {
        (self.first()..self.last()).find(|value| !self.contains(*value))
    }

    fn describe(self, unit: &str) -> String {
        match self {
            Self::Steps(Steps {
                first,
                last,
                step: 1,
            }) => format!("{first} to {last}{unit}"),
            Self::Steps(Steps { first, last, step }) => {
                format!("{first} to {last}{unit} in steps of {step}{unit}")
            }
            Self::Only(values) => {
                let (last, rest) = values.split_last().expect("a value list is never empty");
                let rest: Vec<String> = rest.iter().map(u32::to_string).collect();
                format!("{} or {last}{unit}", rest.join(", "))
            }
        }
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

    pub fn values(self) -> Values {
        platform::values(self)
    }

    pub fn accepted(self) -> String {
        self.values().describe(self.unit())
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

    pub fn peripheral(self) -> Peripheral {
        match self {
            Self::Level(Level::PointerSpeed | Level::DoubleClick)
            | Self::Switch(
                Switch::Acceleration | Switch::SwapButtons | Switch::MouseNaturalScroll,
            ) => Peripheral::Mouse,
            Self::Level(Level::TouchpadSpeed)
            | Self::Switch(Switch::Touchpad | Switch::TapToClick | Switch::TouchpadNaturalScroll) => {
                Peripheral::Touchpad
            }
            Self::Level(Level::RepeatDelay | Level::RepeatRate) => Peripheral::Keyboard,
        }
    }

    pub fn field(self) -> &'static str {
        match self {
            Self::Level(Level::PointerSpeed | Level::TouchpadSpeed) => "speed",
            Self::Level(Level::DoubleClick) => "double_click",
            Self::Level(Level::RepeatDelay) => "repeat_delay",
            Self::Level(Level::RepeatRate) => "repeat_rate",
            Self::Switch(Switch::Acceleration) => "acceleration",
            Self::Switch(Switch::SwapButtons) => "swap_buttons",
            Self::Switch(Switch::MouseNaturalScroll | Switch::TouchpadNaturalScroll) => {
                "natural_scroll"
            }
            Self::Switch(Switch::Touchpad) => "enabled",
            Self::Switch(Switch::TapToClick) => "tap_to_click",
        }
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
    let mut mouse = platform::mouse()?;
    mouse.unavailable = unavailable(Peripheral::Mouse, None);
    Ok(mouse)
}

pub fn touchpad() -> Result<Touchpad> {
    let mut touchpad = platform::touchpad()?;
    touchpad.unavailable = unavailable(Peripheral::Touchpad, touchpad.missing.as_deref());
    Ok(touchpad)
}

pub fn keyboard() -> Result<Keyboard> {
    let mut keyboard = platform::keyboard()?;
    keyboard.unavailable = unavailable(Peripheral::Keyboard, None);
    Ok(keyboard)
}

fn unavailable(peripheral: Peripheral, missing: Option<&str>) -> BTreeMap<&'static str, String> {
    Setting::ALL
        .into_iter()
        .filter(|setting| setting.peripheral() == peripheral)
        .filter_map(|setting| {
            let reason = missing.or_else(|| platform::unsupported(setting))?;
            Some((setting.field(), sentence(reason)))
        })
        .collect()
}

fn sentence(reason: &str) -> String {
    let first_word = reason.split_whitespace().next().unwrap_or_default();
    let mut text = if first_word.chars().all(|c| c.is_ascii_lowercase()) {
        let mut chars = reason.chars();
        chars
            .next()
            .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
            .unwrap_or_default()
    } else {
        reason.to_string()
    };
    if !text.ends_with('.') {
        text.push('.');
    }
    text
}

pub fn devices() -> Vec<Device> {
    listed_devices(&touchpad())
}

fn listed_devices(touchpad: &Result<Touchpad>) -> Vec<Device> {
    let no_touchpad = matches!(
        touchpad,
        Ok(Touchpad { missing: Some(reason), .. }) if reason == NO_TOUCHPAD
    );
    [
        Some(MOUSE),
        (!no_touchpad).then_some(TOUCHPAD),
        Some(KEYBOARD),
    ]
    .into_iter()
    .flatten()
    .collect()
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
    session::claim(Setting::Switch(switch))?;
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
    session::claim(Setting::Level(level))?;
    platform::set_level(level, value)
}

pub fn toggle_touchpad() -> Result<bool> {
    let touchpad = touchpad()?;
    if let Some(reason) = touchpad.missing {
        bail!("{reason}");
    }
    let Some(enabled) = touchpad.enabled else {
        bail!(
            "{}",
            platform::unsupported(Setting::Switch(Switch::Touchpad)).unwrap_or(TOUCHPAD_FIXED)
        );
    };
    session::claim(Setting::Switch(Switch::Touchpad))?;
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
    fn every_setting_names_a_field_its_peripheral_answers_with() {
        let answers = [
            (
                Peripheral::Mouse,
                serde_json::to_value(Mouse::default()).unwrap(),
            ),
            (
                Peripheral::Touchpad,
                serde_json::to_value(Touchpad::default()).unwrap(),
            ),
            (
                Peripheral::Keyboard,
                serde_json::to_value(Keyboard::default()).unwrap(),
            ),
        ];
        for setting in Setting::ALL {
            let (_, answer) = answers
                .iter()
                .find(|(peripheral, _)| *peripheral == setting.peripheral())
                .unwrap();
            assert!(answer.get(setting.field()).is_some(), "{setting:?}");
        }
    }

    #[test]
    fn a_missing_touchpad_explains_every_touchpad_setting_and_nothing_else() {
        let reasons = unavailable(Peripheral::Touchpad, Some(NO_TOUCHPAD));
        assert_eq!(
            reasons.keys().copied().collect::<Vec<_>>(),
            ["enabled", "natural_scroll", "speed", "tap_to_click"]
        );
        assert!(reasons
            .values()
            .all(|reason| reason == "No touchpad is connected."));
    }

    #[test]
    fn a_reason_reads_as_a_sentence_without_breaking_a_product_name() {
        let cases = [
            ("no touchpad is connected", "No touchpad is connected."),
            ("macOS has no setting", "macOS has no setting."),
            ("Windows has no setting", "Windows has no setting."),
            (
                "this touchpad is not a Precision Touchpad",
                "This touchpad is not a Precision Touchpad.",
            ),
            ("already a sentence.", "Already a sentence."),
            ("", "."),
        ];
        for (reason, expected) in cases {
            assert_eq!(sentence(reason), expected, "{reason}");
        }
    }

    #[test]
    fn a_setting_this_platform_cannot_change_is_refused_with_the_reason_it_is_listed_with() {
        for setting in Setting::ALL {
            let Some(reason) = platform::unsupported(setting) else {
                continue;
            };
            let refused = match setting {
                Setting::Switch(switch) => platform::set_switch(switch, true),
                Setting::Level(level) => platform::set_level(level, level.values().first()),
            };
            assert_eq!(refused.unwrap_err().to_string(), reason, "{setting:?}");
        }
    }

    #[test]
    fn a_level_outside_its_values_is_refused_before_the_host_is_touched() {
        for level in Level::ALL {
            let values = level.values();
            for value in values
                .first()
                .checked_sub(1)
                .into_iter()
                .chain([values.last() + 1])
                .chain(values.refused_between())
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
            let bounds = level.bounds();
            for value in level.values().each() {
                assert!(bounds.contains(value), "{level:?} {value}");
            }
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
            Values::Steps(Steps::span(3, 30)).describe(" keys/s"),
            "3 to 30 keys/s"
        );
        assert_eq!(
            Values::Steps(quarters).describe(" ms"),
            "250 to 1000 ms in steps of 250 ms"
        );
        let rates = Values::Only(&[2, 3, 5, 30]);
        assert_eq!(rates.describe(" keys/s"), "2, 3, 5 or 30 keys/s");
        assert!(rates.contains(5));
        assert!(!rates.contains(4));
        assert_eq!(rates.refused_between(), Some(4));
        assert_eq!(rates.each(), [2, 3, 5, 30]);
        assert_eq!(Values::Steps(quarters).each(), [250, 500, 750, 1000]);
        assert_eq!(Values::Steps(Steps::span(2, 50)).refused_between(), None);
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
    fn the_device_list_drops_the_touchpad_only_when_none_is_connected() {
        let kinds = |touchpad: Result<Touchpad>| -> Vec<&str> {
            listed_devices(&touchpad)
                .into_iter()
                .map(|device| device.kind)
                .collect()
        };
        assert_eq!(
            kinds(Ok(Touchpad::default())),
            ["mouse", "touchpad", "keyboard"]
        );
        assert_eq!(
            kinds(Ok(Touchpad::missing(TOUCHPAD_FIXED))),
            ["mouse", "touchpad", "keyboard"]
        );
        assert_eq!(
            kinds(Err(anyhow::anyhow!("unreadable"))),
            ["mouse", "touchpad", "keyboard"]
        );
        assert_eq!(
            kinds(Ok(Touchpad::missing(NO_TOUCHPAD))),
            ["mouse", "keyboard"]
        );
        assert_eq!(
            serde_json::to_value(listed_devices(&Ok(Touchpad::missing(NO_TOUCHPAD)))).unwrap()[0],
            serde_json::json!({
                "id": "mouse",
                "kind": "mouse",
                "name": "Mouse",
                "picture": "mouse",
                "status": "Every mouse",
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
