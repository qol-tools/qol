use evdev::{AbsoluteAxisCode, AttributeSet, Device, EventType, KeyCode, RelativeAxisCode};

pub(crate) const VIRTUAL_KEYBOARD_NAME: &str = "qol-tray-virtual-keyboard";

pub(crate) struct DeviceCapabilities {
    pub(crate) name: String,
    pub(crate) event_types: AttributeSet<EventType>,
    pub(crate) keys: AttributeSet<KeyCode>,
    pub(crate) relative_axes: AttributeSet<RelativeAxisCode>,
    pub(crate) absolute_axes: AttributeSet<AbsoluteAxisCode>,
}

impl DeviceCapabilities {
    pub(crate) fn of(device: &Device) -> Self {
        Self {
            name: device.name().unwrap_or_default().to_string(),
            event_types: device.supported_events().iter().collect(),
            keys: device
                .supported_keys()
                .map(|keys| keys.iter().collect())
                .unwrap_or_default(),
            relative_axes: device
                .supported_relative_axes()
                .map(|axes| axes.iter().collect())
                .unwrap_or_default(),
            absolute_axes: device
                .supported_absolute_axes()
                .map(|axes| axes.iter().collect())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SkipReason {
    VirtualKeyboard,
    NoKeyboardKeys,
    PointerAxes,
    AbsoluteAxes,
    ButtonCodes,
    ExtraEventTypes(Vec<EventType>),
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::VirtualKeyboard => write!(f, "qol-tray virtual keyboard"),
            SkipReason::NoKeyboardKeys => write!(f, "no keyboard keys"),
            SkipReason::PointerAxes => write!(f, "pointer axes"),
            SkipReason::AbsoluteAxes => write!(f, "absolute axes"),
            SkipReason::ButtonCodes => write!(f, "button codes"),
            SkipReason::ExtraEventTypes(types) => {
                let names: Vec<String> = types.iter().map(|t| format!("{t:?}")).collect();
                write!(f, "extra event types: {}", names.join(", "))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeviceClass {
    Keyboard,
    Skipped(SkipReason),
}

pub(crate) fn classify(caps: &DeviceCapabilities) -> DeviceClass {
    if caps.name == VIRTUAL_KEYBOARD_NAME {
        return DeviceClass::Skipped(SkipReason::VirtualKeyboard);
    }
    if !caps.keys.contains(KeyCode::KEY_ESC) || !caps.keys.contains(KeyCode::KEY_A) {
        return DeviceClass::Skipped(SkipReason::NoKeyboardKeys);
    }
    if caps.relative_axes.iter().next().is_some() {
        return DeviceClass::Skipped(SkipReason::PointerAxes);
    }
    if caps.absolute_axes.iter().next().is_some() {
        return DeviceClass::Skipped(SkipReason::AbsoluteAxes);
    }
    if caps
        .keys
        .iter()
        .any(|key| (0x100..=0x151).contains(&key.0) || (0x2c0..=0x2e7).contains(&key.0))
    {
        return DeviceClass::Skipped(SkipReason::ButtonCodes);
    }
    let allowed = [
        EventType::SYNCHRONIZATION,
        EventType::KEY,
        EventType::MISC,
        EventType::LED,
        EventType::REPEAT,
    ];
    let mut extra: Vec<EventType> = caps
        .event_types
        .iter()
        .filter(|t| !allowed.contains(t))
        .collect();
    if !extra.is_empty() {
        extra.sort_by_key(|t| t.0);
        return DeviceClass::Skipped(SkipReason::ExtraEventTypes(extra));
    }
    DeviceClass::Keyboard
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(
        name: &str,
        types: &[EventType],
        keys: &[KeyCode],
        rel: &[RelativeAxisCode],
        abs: &[AbsoluteAxisCode],
    ) -> DeviceCapabilities {
        DeviceCapabilities {
            name: name.to_string(),
            event_types: types.iter().copied().collect(),
            keys: keys.iter().copied().collect(),
            relative_axes: rel.iter().copied().collect(),
            absolute_axes: abs.iter().copied().collect(),
        }
    }

    fn keyboard_types() -> Vec<EventType> {
        vec![
            EventType::SYNCHRONIZATION,
            EventType::KEY,
            EventType::MISC,
            EventType::LED,
            EventType::REPEAT,
        ]
    }

    fn keyboard_keys() -> Vec<KeyCode> {
        vec![KeyCode::KEY_ESC, KeyCode::KEY_A]
    }

    #[test]
    fn pure_keyboard_classifies_as_keyboard() {
        let node = caps(
            "AT Translated Set 2 keyboard",
            &keyboard_types(),
            &keyboard_keys(),
            &[],
            &[],
        );
        assert_eq!(classify(&node), DeviceClass::Keyboard);
    }

    #[test]
    fn mouse_combo_node_skips_on_pointer_axes_before_buttons() {
        let node = caps(
            "Logitech G305",
            &[
                EventType::SYNCHRONIZATION,
                EventType::KEY,
                EventType::RELATIVE,
                EventType::MISC,
            ],
            &[KeyCode::KEY_ESC, KeyCode::KEY_A, KeyCode::BTN_LEFT],
            &[RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::PointerAxes)
        );
    }

    #[test]
    fn scroll_wheel_only_keyboard_skips_on_pointer_axes() {
        let node = caps(
            "Keyboard with wheel knob",
            &[
                EventType::SYNCHRONIZATION,
                EventType::KEY,
                EventType::RELATIVE,
            ],
            &keyboard_keys(),
            &[RelativeAxisCode::REL_WHEEL],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::PointerAxes)
        );
    }

    #[test]
    fn pen_tablet_skips_on_absolute_axes() {
        let node = caps(
            "Wacom pen tablet",
            &[
                EventType::SYNCHRONIZATION,
                EventType::KEY,
                EventType::ABSOLUTE,
            ],
            &keyboard_keys(),
            &[],
            &[AbsoluteAxisCode::ABS_X, AbsoluteAxisCode::ABS_Y],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::AbsoluteAxes)
        );
    }

    #[test]
    fn gamepad_buttons_skip_on_button_codes() {
        let node = caps(
            "Gamepad node",
            &[EventType::SYNCHRONIZATION, EventType::KEY],
            &[KeyCode::KEY_ESC, KeyCode::KEY_A, KeyCode::BTN_SOUTH],
            &[],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::ButtonCodes)
        );
    }

    #[test]
    fn trigger_happy_buttons_skip_on_button_codes() {
        let node = caps(
            "Joystick node",
            &[EventType::SYNCHRONIZATION, EventType::KEY],
            &[
                KeyCode::KEY_ESC,
                KeyCode::KEY_A,
                KeyCode::BTN_TRIGGER_HAPPY1,
            ],
            &[],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::ButtonCodes)
        );
    }

    #[test]
    fn consumer_control_without_esc_and_a_skips_on_no_keyboard_keys() {
        let node = caps(
            "Consumer control",
            &[EventType::SYNCHRONIZATION, EventType::KEY],
            &[KeyCode::KEY_VOLUMEUP],
            &[],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::NoKeyboardKeys)
        );
    }

    #[test]
    fn esc_without_a_skips_on_no_keyboard_keys() {
        let node = caps(
            "Partial keyboard",
            &[EventType::SYNCHRONIZATION, EventType::KEY],
            &[KeyCode::KEY_ESC],
            &[],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::NoKeyboardKeys)
        );
    }

    #[test]
    fn virtual_keyboard_name_skips_before_anything_else() {
        let node = caps(
            VIRTUAL_KEYBOARD_NAME,
            &keyboard_types(),
            &keyboard_keys(),
            &[],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::VirtualKeyboard)
        );
    }

    #[test]
    fn forcefeedback_reports_extra_event_types() {
        let node = caps(
            "Force feedback keyboard",
            &[
                EventType::SYNCHRONIZATION,
                EventType::KEY,
                EventType::FORCEFEEDBACK,
            ],
            &keyboard_keys(),
            &[],
            &[],
        );
        assert_eq!(
            classify(&node),
            DeviceClass::Skipped(SkipReason::ExtraEventTypes(vec![EventType::FORCEFEEDBACK]))
        );
    }

    #[test]
    fn skip_reason_display_strings() {
        assert_eq!(
            SkipReason::VirtualKeyboard.to_string(),
            "qol-tray virtual keyboard"
        );
        assert_eq!(SkipReason::NoKeyboardKeys.to_string(), "no keyboard keys");
        assert_eq!(SkipReason::PointerAxes.to_string(), "pointer axes");
        assert_eq!(SkipReason::AbsoluteAxes.to_string(), "absolute axes");
        assert_eq!(SkipReason::ButtonCodes.to_string(), "button codes");
        assert_eq!(
            SkipReason::ExtraEventTypes(vec![EventType::FORCEFEEDBACK]).to_string(),
            "extra event types: FORCEFEEDBACK"
        );
        assert_eq!(
            SkipReason::ExtraEventTypes(vec![EventType::SWITCH, EventType::SOUND,]).to_string(),
            "extra event types: SWITCH, SOUND"
        );
    }
}
