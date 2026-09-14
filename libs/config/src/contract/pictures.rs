pub const CARD_DESCRIPTION_MAX: usize = 36;

pub const PICTURE_NAMES: &[&str] = &[
    "hold-to-switch",
    "sticky",
    "cycle-once",
    "show-only",
    "icon-corner",
    "aa",
    "desktop-theme",
    "web-theme",
    "swatch",
    "qol-toasts",
    "system-bubbles",
    "both-notices",
    "launch-app",
    "open-url",
    "bundle-ref",
    "path-ref",
    "name-ref",
    "browser-bundle-ref",
    "browser-path-ref",
    "browser-name-ref",
    "letters",
    "next-window",
    "previous-window",
    "gear",
    "adapter-auto",
    "adapter-chip",
    "schedule-off",
    "schedule-daily",
    "mic-default",
    "usb-mic",
    "webcam",
    "headset",
    "speaker-default",
    "hdmi",
    "headphones",
    "preset",
    "format",
    "copy-image",
    "copy-path",
    "no-terminal",
    "terminal-session",
    "insert-only",
    "insert-submit",
    "prefer-local",
    "local-engine",
    "remote-engine",
    "no-model",
    "model",
    "family-auto",
    "fixed-size",
    "relative-size",
];

const SWATCH_ARGUMENTS: [&str; 6] = ["amber", "green", "cyan", "magenta", "blue", "violet"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDirection {
    Input,
    Output,
}

pub fn is_picture_spec(spec: &str) -> bool {
    let (name, argument) = match spec.split_once(':') {
        Some((name, argument)) => (name, Some(argument)),
        None => (spec, None),
    };
    if !PICTURE_NAMES.contains(&name) {
        return false;
    }
    match name {
        "icon-corner" => argument.is_some_and(|value| matches!(value, "left" | "right")),
        "aa" => argument.is_some_and(|value| bounded_integer(value, 8, 48)),
        "desktop-theme" => argument.is_some_and(|value| matches!(value, "bone" | "slate")),
        "web-theme" => argument.is_some_and(|value| matches!(value, "slate" | "midnight")),
        "swatch" => argument.is_some_and(is_swatch),
        "letters" => argument.is_some_and(|value| text_without_whitespace(value, 1, 3)),
        "preset" => argument.is_some_and(preset_pair),
        "format" => argument.is_some_and(|value| text_without_whitespace(value, 1, 5)),
        "terminal-session" => argument.is_some_and(|value| text_length(value, 1, 24)),
        "local-engine" => argument.is_some_and(|value| text_length(value, 1, 12)),
        _ => argument.is_none(),
    }
}

pub fn audio_device_picture(
    entry: &serde_json::Value,
    direction: AudioDirection,
) -> Option<&'static str> {
    match direction {
        AudioDirection::Input => {
            let form_factor = json_property(entry, "device.form_factor");
            if form_factor == Some("webcam") {
                return Some("webcam");
            }
            if form_factor == Some("headset") {
                return Some("headset");
            }
            if json_property(entry, "device.bus") == Some("usb") {
                return Some("usb-mic");
            }
            None
        }
        AudioDirection::Output => {
            let form_factor = json_property(entry, "device.form_factor");
            let active_port = json_string(entry, "active_port");
            if matches!(form_factor, Some("headset") | Some("headphone"))
                || active_port.is_some_and(|port| port.contains("headphone"))
            {
                return Some("headphones");
            }
            let profile = json_property(entry, "device.profile.name");
            let name = json_string(entry, "name");
            let contains_hdmi = |text: &str| text.to_lowercase().contains("hdmi");
            if profile.is_some_and(contains_hdmi)
                || active_port.is_some_and(contains_hdmi)
                || name.is_some_and(contains_hdmi)
            {
                return Some("hdmi");
            }
            None
        }
    }
}

fn json_string<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn json_property<'a>(entry: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    entry.get("properties")?.get(name)?.as_str()
}

fn is_swatch(value: &str) -> bool {
    SWATCH_ARGUMENTS.contains(&value)
}

fn bounded_integer(value: &str, min: u32, max: u32) -> bool {
    let Ok(number) = value.parse::<u32>() else {
        return false;
    };
    (min..=max).contains(&number)
}

fn preset_pair(value: &str) -> bool {
    let mut parts = value.split(',');
    let (Some(speed), Some(size), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    bounded_integer(speed, 0, 100) && bounded_integer(size, 0, 100)
}

fn text_length(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.chars().count())
}

fn text_without_whitespace(value: &str, min: usize, max: usize) -> bool {
    text_length(value, min, max) && !value.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn input_webcam_form_factor_draws_a_webcam() {
        let entry = json!({
            "name": "alsa_input.usb-camera",
            "active_port": null,
            "properties": {
                "device.form_factor": "webcam",
                "device.bus": "usb",
                "device.profile.name": "mono-fallback",
            },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Input),
            Some("webcam")
        );
    }

    #[test]
    fn input_headset_form_factor_draws_a_headset() {
        let entry = json!({
            "name": "alsa_input.usb-headset",
            "active_port": "headset-input",
            "properties": { "device.form_factor": "headset", "device.bus": "usb" },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Input),
            Some("headset")
        );
    }

    #[test]
    fn input_usb_bus_draws_a_usb_mic() {
        let entry = json!({
            "name": "alsa_input.usb-mic",
            "active_port": "mic",
            "properties": { "device.form_factor": "internal", "device.bus": "usb" },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Input),
            Some("usb-mic")
        );
    }

    #[test]
    fn input_without_a_matching_hint_draws_nothing() {
        let entry = json!({
            "name": "alsa_input.pci-0000_00_1f.3.analog-stereo",
            "active_port": "analog-input",
            "properties": { "device.form_factor": "internal", "device.bus": "pci" },
        });
        assert_eq!(audio_device_picture(&entry, AudioDirection::Input), None);
    }

    #[test]
    fn output_headset_form_factor_draws_headphones() {
        let entry = json!({
            "name": "alsa_output.usb-headset",
            "active_port": null,
            "properties": { "device.form_factor": "headset" },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Output),
            Some("headphones")
        );
    }

    #[test]
    fn output_headphone_active_port_draws_headphones() {
        let entry = json!({
            "name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
            "active_port": "headphones-output",
            "properties": { "device.form_factor": "internal" },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Output),
            Some("headphones")
        );
    }

    #[test]
    fn output_hdmi_profile_draws_hdmi() {
        let entry = json!({
            "name": "alsa_output.pci-0000_00_1f.3",
            "active_port": "digital-output",
            "properties": { "device.profile.name": "hdmi-stereo" },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Output),
            Some("hdmi")
        );
    }

    #[test]
    fn output_hdmi_active_port_draws_hdmi() {
        let entry = json!({
            "name": "alsa_output.pci-0000_00_1f.3",
            "active_port": "hdmi-output-0",
            "properties": { "device.profile.name": "analog-stereo" },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Output),
            Some("hdmi")
        );
    }

    #[test]
    fn output_uppercase_hdmi_name_draws_hdmi() {
        let entry = json!({
            "name": "alsa_output.HDMI-0",
            "active_port": null,
            "properties": { "device.profile.name": "analog-stereo" },
        });
        assert_eq!(
            audio_device_picture(&entry, AudioDirection::Output),
            Some("hdmi")
        );
    }

    #[test]
    fn output_without_a_matching_hint_draws_nothing() {
        let entry = json!({
            "name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
            "active_port": "analog-output",
            "properties": { "device.form_factor": "speaker", "device.bus": "pci" },
        });
        assert_eq!(audio_device_picture(&entry, AudioDirection::Output), None);
    }
}
