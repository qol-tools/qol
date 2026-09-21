use crate::devices::{AudioDevice, Device, Direction, Identity, Kind, Resolution};
use crate::AudioError;

pub(crate) fn list_audio_devices(direction: Direction) -> Result<Vec<AudioDevice>, AudioError> {
    super::with_connection(|connection| {
        let devices = super::devices::list_devices(connection, direction)?;
        Ok(devices
            .iter()
            .filter(|device| listed(direction, device))
            .map(|device| audio_device(device, direction))
            .collect())
    })
}

/// Maps a server node name back to the stable identity the settings page and
/// the ownership record both speak, so a saved choice can be compared with what
/// the server actually plays on.
pub(crate) fn identity_for_node(
    direction: Direction,
    node: &str,
) -> Result<Option<Identity>, AudioError> {
    super::with_connection(|connection| {
        let devices = super::devices::list_devices(connection, direction)?;
        Ok(devices
            .iter()
            .filter(|device| listed(direction, device))
            .find(|device| device.name == node)
            .map(device_identity))
    })
}

pub(crate) fn node_for_identity(
    direction: Direction,
    output: &Identity,
) -> Result<Option<String>, AudioError> {
    super::with_connection(|connection| {
        let devices = super::devices::list_devices(connection, direction)?;
        Ok(devices
            .iter()
            .filter(|device| listed(direction, device))
            .find(|device| device_identity(device) == *output)
            .map(|device| device.name.clone()))
    })
}

pub(crate) fn resolve_audio_device(
    direction: Direction,
    requested: &str,
) -> Result<Resolution, AudioError> {
    let devices = list_audio_devices(direction)?;
    Ok(resolve_in(&devices, requested))
}

fn listed(direction: Direction, device: &Device) -> bool {
    match direction {
        Direction::Output => true,
        Direction::Input => !is_monitor(device),
    }
}

fn is_monitor(device: &Device) -> bool {
    device.monitor_of_sink.is_some()
        || device.name.ends_with(".monitor")
        || device
            .properties
            .get("device.class")
            .is_some_and(|class| class == "monitor")
}

fn audio_device(device: &Device, direction: Direction) -> AudioDevice {
    let identity = device_identity(device);
    let kind = device_kind(device);
    AudioDevice {
        value: identity.as_str().to_owned(),
        label: device.description.clone(),
        picture: device_picture(direction, kind).to_owned(),
        kind,
        identity,
    }
}

fn device_identity(device: &Device) -> Identity {
    let device_name = device
        .properties
        .get("device.name")
        .map(String::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(device.name.as_str());
    let profile = device
        .properties
        .get("device.profile.name")
        .map(String::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("-");
    let port = device.active_port.as_deref().unwrap_or("-");
    Identity::from_raw(format!("{device_name}:{profile}:{port}"))
}

fn device_kind(device: &Device) -> Kind {
    let bus = device.properties.get("device.bus").map(String::as_str);
    let form_factor = device
        .properties
        .get("device.form_factor")
        .map(String::as_str);
    let api = device.properties.get("device.api").map(String::as_str);
    let driver = device
        .properties
        .get("alsa.driver_name")
        .map(String::as_str);

    if bus == Some("bluetooth")
        || api.is_some_and(|value| value.starts_with("bluez"))
        || driver.is_some_and(|value| value.contains("bluez"))
    {
        return Kind::Bluetooth;
    }
    if bus == Some("usb") || driver.is_some_and(|value| value.starts_with("snd_usb")) {
        return Kind::Usb;
    }
    if is_hdmi(device) {
        return Kind::Hdmi;
    }
    if bus.is_some_and(|value| matches!(value, "pci" | "platform"))
        || form_factor == Some("internal")
        || driver.is_some_and(|value| value.starts_with("snd_hda") || value.starts_with("snd_soc"))
    {
        return Kind::BuiltIn;
    }
    Kind::Unknown
}

fn is_hdmi(device: &Device) -> bool {
    device.active_port.as_deref().is_some_and(mentions_hdmi)
        || device
            .properties
            .get("device.profile.name")
            .is_some_and(|profile| mentions_hdmi(profile))
        || device
            .properties
            .get("device.icon_name")
            .is_some_and(|icon| icon.starts_with("video-display"))
        || device
            .properties
            .get("alsa.driver_name")
            .is_some_and(|driver| mentions_hdmi(driver))
}

fn mentions_hdmi(value: &str) -> bool {
    value.to_ascii_lowercase().contains("hdmi")
}

fn device_picture(direction: Direction, kind: Kind) -> &'static str {
    match (direction, kind) {
        (Direction::Input, Kind::Bluetooth) => "headset",
        (Direction::Input, Kind::Usb) => "usb-mic",
        (Direction::Input, _) => "mic-default",
        (Direction::Output, Kind::Bluetooth | Kind::Usb) => "headphones",
        (Direction::Output, Kind::Hdmi) => "hdmi",
        (Direction::Output, _) => "speaker-default",
    }
}

fn resolve_in(devices: &[AudioDevice], requested: &str) -> Resolution {
    let exact = devices
        .iter()
        .filter(|device| device.identity.as_str() == requested)
        .cloned()
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return narrow(exact);
    }
    let labelled = devices
        .iter()
        .filter(|device| device.label.eq_ignore_ascii_case(requested))
        .cloned()
        .collect::<Vec<_>>();
    narrow(labelled)
}

fn narrow(mut matches: Vec<AudioDevice>) -> Resolution {
    if matches.len() > 1 {
        return Resolution::Ambiguous(matches);
    }
    match matches.pop() {
        Some(device) => Resolution::Resolved(device),
        None => Resolution::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::State;

    fn device(
        index: u32,
        name: &str,
        description: &str,
        properties: &[(&str, &str)],
        active_port: Option<&str>,
    ) -> Device {
        Device {
            index,
            name: name.to_owned(),
            description: description.to_owned(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            active_port: active_port.map(str::to_owned),
            monitor_of_sink: None,
            state: State::Unknown,
        }
    }

    fn recorded(
        direction: Direction,
        index: u32,
        name: &str,
        description: &str,
        properties: &[(&str, &str)],
        active_port: Option<&str>,
    ) -> AudioDevice {
        audio_device(
            &device(index, name, description, properties, active_port),
            direction,
        )
    }

    #[test]
    fn identity_is_stable_across_a_changed_server_index() {
        let properties = [
            ("device.name", "alsa_card.pci-0000_00_1f.3"),
            ("device.profile.name", "analog-stereo"),
            ("device.bus", "pci"),
            ("device.form_factor", "internal"),
        ];
        let first = device(
            3,
            "alsa_output.pci",
            "Speakers",
            &properties,
            Some("analog-output-speaker"),
        );
        let second = device(
            9,
            "alsa_output.pci",
            "Speakers",
            &properties,
            Some("analog-output-speaker"),
        );

        assert_ne!(first.index, second.index);
        assert_eq!(device_identity(&first), device_identity(&second));
        assert_eq!(
            device_identity(&first).as_str(),
            "alsa_card.pci-0000_00_1f.3:analog-stereo:analog-output-speaker"
        );

        let mapped = audio_device(&first, Direction::Output);
        assert_eq!(mapped.value, mapped.identity.as_str());
    }

    #[test]
    fn two_ports_on_one_card_get_different_identities() {
        let properties = [
            ("device.name", "alsa_card.pci-0000_01_00.1"),
            ("device.profile.name", "hdmi-stereo"),
        ];
        let first = device(
            4,
            "alsa_output.hdmi",
            "HDMI",
            &properties,
            Some("hdmi-output-0"),
        );
        let second = device(
            4,
            "alsa_output.hdmi",
            "HDMI",
            &properties,
            Some("hdmi-output-1"),
        );

        assert_ne!(device_identity(&first), device_identity(&second));
    }

    #[test]
    fn kind_and_picture_come_from_the_reported_properties() {
        let cases = [
            (
                "bluetooth microphone",
                Direction::Input,
                device(
                    1,
                    "bluez_input.74_68_59_7F_5F_E9",
                    "Luna 2",
                    &[
                        ("device.bus", "bluetooth"),
                        ("device.form_factor", "headset"),
                    ],
                    None,
                ),
                Kind::Bluetooth,
                "headset",
            ),
            (
                "bluetooth output",
                Direction::Output,
                device(
                    1,
                    "bluez_output.74_68_59_7F_5F_E9.1",
                    "Luna 2",
                    &[("device.bus", "bluetooth"), ("device.api", "bluez5")],
                    None,
                ),
                Kind::Bluetooth,
                "headphones",
            ),
            (
                "usb microphone",
                Direction::Input,
                device(
                    2,
                    "alsa_input.usb",
                    "USB Mic",
                    &[("device.bus", "usb"), ("device.form_factor", "microphone")],
                    None,
                ),
                Kind::Usb,
                "usb-mic",
            ),
            (
                "usb output",
                Direction::Output,
                device(
                    2,
                    "alsa_output.usb",
                    "USB Headset",
                    &[("device.bus", "usb"), ("device.form_factor", "headset")],
                    None,
                ),
                Kind::Usb,
                "headphones",
            ),
            (
                "hdmi output",
                Direction::Output,
                device(
                    3,
                    "alsa_output.hdmi",
                    "HDMI",
                    &[
                        ("device.bus", "pci"),
                        ("device.profile.name", "hdmi-stereo"),
                    ],
                    Some("hdmi-output-0"),
                ),
                Kind::Hdmi,
                "hdmi",
            ),
            (
                "built-in output",
                Direction::Output,
                device(
                    4,
                    "alsa_output.pci",
                    "Speakers",
                    &[
                        ("device.bus", "pci"),
                        ("device.form_factor", "internal"),
                        ("device.class", "sound"),
                        ("alsa.driver_name", "snd_hda_intel"),
                    ],
                    Some("analog-output-speaker"),
                ),
                Kind::BuiltIn,
                "speaker-default",
            ),
            (
                "built-in microphone",
                Direction::Input,
                device(
                    4,
                    "alsa_input.pci",
                    "Internal Mic",
                    &[("device.bus", "pci"), ("device.form_factor", "internal")],
                    Some("analog-input-mic"),
                ),
                Kind::BuiltIn,
                "mic-default",
            ),
            (
                "unknown output",
                Direction::Output,
                device(5, "virtual_output", "Virtual", &[], None),
                Kind::Unknown,
                "speaker-default",
            ),
            (
                "unknown microphone",
                Direction::Input,
                device(5, "virtual_input", "Virtual", &[], None),
                Kind::Unknown,
                "mic-default",
            ),
        ];

        for (label, direction, record, kind, picture) in cases {
            let mapped = audio_device(&record, direction);
            assert_eq!(mapped.kind, kind, "{label}");
            assert_eq!(mapped.picture, picture, "{label}");
        }
    }

    #[test]
    fn microphone_listing_excludes_monitor_sources() {
        let mut monitor = device(1, "alsa_output.pci.monitor", "Monitor", &[], None);
        monitor.monitor_of_sink = Some(1);
        let classified = device(
            2,
            "alsa_input.remote",
            "Remote Monitor",
            &[("device.class", "monitor")],
            None,
        );
        let suffixed = device(3, "alsa_output.pci.monitor", "Monitor", &[], None);
        let microphone = device(
            4,
            "alsa_input.usb",
            "USB Mic",
            &[("device.bus", "usb")],
            None,
        );

        assert!(!listed(Direction::Input, &monitor));
        assert!(!listed(Direction::Input, &classified));
        assert!(!listed(Direction::Input, &suffixed));
        assert!(listed(Direction::Input, &microphone));
    }

    #[test]
    fn an_exact_identity_match_wins_over_an_ambiguous_label() {
        let devices = [
            recorded(
                Direction::Output,
                1,
                "alsa_output.first",
                "USB Audio Device",
                &[("device.name", "alsa_card.usb-one")],
                Some("analog-output"),
            ),
            recorded(
                Direction::Output,
                2,
                "alsa_output.second",
                "USB Audio Device",
                &[("device.name", "alsa_card.usb-two")],
                Some("analog-output"),
            ),
        ];

        let requested = devices[1].identity.as_str();
        assert_eq!(
            resolve_in(&devices, requested),
            Resolution::Resolved(devices[1].clone())
        );
    }

    #[test]
    fn an_unambiguous_label_match_resolves_case_insensitively() {
        let devices = [
            recorded(
                Direction::Output,
                1,
                "alsa_output.first",
                "Desk Speakers",
                &[("device.name", "alsa_card.first")],
                Some("analog-output"),
            ),
            recorded(
                Direction::Output,
                2,
                "alsa_output.second",
                "USB Headset",
                &[("device.name", "alsa_card.second")],
                Some("analog-output"),
            ),
        ];

        assert_eq!(
            resolve_in(&devices, "usb headset"),
            Resolution::Resolved(devices[1].clone())
        );
    }

    #[test]
    fn an_ambiguous_label_refuses_with_every_candidate() {
        let shared = [("device.name", "alsa_card.usb-audio")];
        let devices = [
            recorded(
                Direction::Output,
                1,
                "alsa_output.first",
                "USB Audio Device",
                &shared,
                Some("analog-output"),
            ),
            recorded(
                Direction::Output,
                2,
                "alsa_output.second",
                "USB Audio Device",
                &shared,
                Some("analog-output"),
            ),
            recorded(
                Direction::Output,
                3,
                "alsa_output.third",
                "Desk Speakers",
                &[("device.name", "alsa_card.desk")],
                Some("analog-output"),
            ),
        ];

        match resolve_in(&devices, "usb audio device") {
            Resolution::Ambiguous(candidates) => {
                assert_eq!(candidates.len(), 2);
                assert_eq!(candidates[0], devices[0]);
                assert_eq!(candidates[1], devices[1]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn no_match_is_not_found() {
        let devices = [recorded(
            Direction::Output,
            1,
            "alsa_output.first",
            "Desk Speakers",
            &[("device.name", "alsa_card.first")],
            Some("analog-output"),
        )];

        assert_eq!(resolve_in(&devices, "missing output"), Resolution::NotFound);
    }
}
