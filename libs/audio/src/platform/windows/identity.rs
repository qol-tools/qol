use std::collections::BTreeMap;

use windows::core::GUID;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    DigitalAudioDisplayDevice, PKEY_AudioEndpoint_FormFactor, DEVICE_STATE_ACTIVE,
};

use crate::devices::{AudioDevice, Device, Direction, Identity, Kind, Resolution, State};
use crate::AudioError;

use super::com::{self, Endpoint};

const SYSTEM_CONTAINER: GUID = GUID::from_u128(0x00000000_0000_0000_ffff_ffffffffffff);
const BLUETOOTH_BUSES: [&str; 3] = ["bthenum#", "bthhfenum#", "bthledevice#"];
const USB_BUS: &str = "usb#";
const BUILT_IN_BUSES: [&str; 4] = ["hdaudio#", "intelaudio#", "acpi#", "pci#"];
const PATH_PROPERTY: &str = "device.path";

struct Facts {
    identity: Identity,
    label: String,
    path: Option<String>,
    kind: Kind,
    container: Option<GUID>,
}

fn facts(endpoint: &Endpoint) -> Facts {
    let path = com::topology_link(&endpoint.device).map(|(path, _)| path);
    let form_factor = com::number_property(&endpoint.device, &PKEY_AudioEndpoint_FormFactor);
    Facts {
        identity: Identity::from_raw(endpoint.id.clone()),
        label: com::text_property(&endpoint.device, &PKEY_Device_FriendlyName)
            .unwrap_or_else(|| endpoint.id.clone()),
        kind: kind_of(path.as_deref().unwrap_or_default(), form_factor),
        container: com::container(&endpoint.device),
        path,
    }
}

fn active_facts(direction: Direction) -> Result<Vec<Facts>, AudioError> {
    com::with_enumerator(|enumerator| {
        Ok(com::endpoints(enumerator, direction, DEVICE_STATE_ACTIVE)?
            .iter()
            .map(facts)
            .collect())
    })
}

pub(crate) fn list_audio_devices(direction: Direction) -> Result<Vec<AudioDevice>, AudioError> {
    Ok(active_facts(direction)?
        .into_iter()
        .map(|facts| AudioDevice {
            value: facts.identity.as_str().to_owned(),
            picture: facts.kind.picture(direction).to_owned(),
            label: facts.label,
            kind: facts.kind,
            identity: facts.identity,
        })
        .collect())
}

pub(crate) fn list_devices(direction: Direction) -> Result<Vec<Device>, AudioError> {
    Ok(active_facts(direction)?
        .into_iter()
        .zip(0u32..)
        .map(|(facts, index)| Device {
            index,
            name: facts.identity.as_str().to_owned(),
            description: facts.label,
            properties: facts
                .path
                .map(|path| BTreeMap::from([(PATH_PROPERTY.to_owned(), path)]))
                .unwrap_or_default(),
            active_port: None,
            monitor_of_sink: None,
            state: State::Unknown,
        })
        .collect())
}

pub(crate) fn resolve_audio_device(
    direction: Direction,
    requested: &str,
) -> Result<Resolution, AudioError> {
    let devices = list_audio_devices(direction)?;
    Ok(Resolution::among(&devices, requested))
}

pub(crate) fn identity_for_node(
    direction: Direction,
    node: &str,
) -> Result<Option<Identity>, AudioError> {
    com::with_enumerator(|enumerator| {
        Ok(com::endpoints(enumerator, direction, DEVICE_STATE_ACTIVE)?
            .into_iter()
            .find(|endpoint| endpoint.id == node)
            .map(|endpoint| Identity::from_raw(endpoint.id)))
    })
}

pub(crate) fn companion_input(output: &Identity) -> Result<Option<Identity>, AudioError> {
    let outputs = active_facts(Direction::Output)?;
    let Some(output) = outputs.iter().find(|facts| facts.identity == *output) else {
        return Ok(None);
    };
    Ok(companion_in(output, &active_facts(Direction::Input)?))
}

fn companion_in(output: &Facts, inputs: &[Facts]) -> Option<Identity> {
    let container = output.container?;
    let instance = output.path.as_deref().map(device_instance);
    inputs
        .iter()
        .find(|input| {
            input.container == Some(container)
                && (container != SYSTEM_CONTAINER
                    || (instance.is_some()
                        && input.path.as_deref().map(device_instance) == instance))
        })
        .map(|input| input.identity.clone())
}

fn kind_of(path: &str, form_factor: Option<u32>) -> Kind {
    let path = path.to_ascii_lowercase();
    if BLUETOOTH_BUSES.iter().any(|bus| path.contains(bus)) {
        return Kind::Bluetooth;
    }
    if path.contains(USB_BUS) {
        return Kind::Usb;
    }
    if form_factor == u32::try_from(DigitalAudioDisplayDevice.0).ok() {
        return Kind::Hdmi;
    }
    if BUILT_IN_BUSES.iter().any(|bus| path.contains(bus)) {
        return Kind::BuiltIn;
    }
    Kind::Unknown
}

fn device_instance(path: &str) -> String {
    let path = path.to_ascii_lowercase();
    let path = path
        .split_once("}.")
        .map_or(path.as_str(), |(_, rest)| rest);
    path.split("#{").next().unwrap_or(path).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEAKERS: &str = r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0256&subsys_10280a20&rev_1000#4&2a1d2f6&0&0001#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\espeakerwave";
    const INTERNAL_MIC: &str = r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0256&subsys_10280a20&rev_1000#4&2a1d2f6&0&0001#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\emicinwave";
    const HDMI: &str = r"{2}.\\?\hdaudio#func_01&ven_8086&dev_2812&subsys_80860101&rev_1000#4&1b9e0b5d&0&0201#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\ehdmiwave";
    const USB_HEADSET: &str = r"{2}.\\?\usb#vid_046d&pid_0a44&mi_00#7&1f2b6c1&0&0000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\global";
    const A2DP: &str = r"{2}.\\?\bthenum#{0000110b-0000-1000-8000-00805f9b34fb}_vid&0001000f_pid&1200_ver&0001#7&1a5ab4f3&0&7468597f5fe9_c00000000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\aanalogout";
    const HANDS_FREE: &str = r"{2}.\\?\bthhfenum#bthhfpaudio#8&2cb2a0a5&0&97#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\bthhfpaudiowave";
    const HEADSET_CONTAINER: GUID = GUID::from_u128(0x5b0bd0a1_7a3c_4c3e_9a8c_1f3f5d1e2b11);
    const OTHER_CONTAINER: GUID = GUID::from_u128(0x9e0d3a44_1c2b_4f6e_8a7d_2b4c6d8e0f12);

    fn facts(id: &str, path: Option<&str>, container: Option<GUID>) -> Facts {
        Facts {
            identity: Identity::from_raw(id),
            label: id.to_owned(),
            path: path.map(str::to_owned),
            kind: Kind::Unknown,
            container,
        }
    }

    #[test]
    fn the_kind_comes_from_the_bus_and_the_form_factor() {
        let hdmi = u32::try_from(DigitalAudioDisplayDevice.0).ok();
        let cases = [
            ("bluetooth stereo", A2DP, None, Kind::Bluetooth),
            ("bluetooth hands-free", HANDS_FREE, None, Kind::Bluetooth),
            ("usb headset", USB_HEADSET, None, Kind::Usb),
            ("hdmi display", HDMI, hdmi, Kind::Hdmi),
            ("built-in speakers", SPEAKERS, Some(1), Kind::BuiltIn),
            (
                "virtual cable",
                r"{2}.\\?\root#media#0000#{guid}",
                Some(1),
                Kind::Unknown,
            ),
            ("no topology", "", None, Kind::Unknown),
        ];
        for (label, path, form_factor, kind) in cases {
            assert_eq!(kind_of(path, form_factor), kind, "{label}");
        }
    }

    #[test]
    fn the_device_instance_drops_the_prefix_and_the_interface() {
        let cases = [
            (
                SPEAKERS,
                r"\\?\hdaudio#func_01&ven_10ec&dev_0256&subsys_10280a20&rev_1000#4&2a1d2f6&0&0001",
            ),
            (
                USB_HEADSET,
                r"\\?\usb#vid_046d&pid_0a44&mi_00#7&1f2b6c1&0&0000",
            ),
            ("plain", "plain"),
        ];
        for (path, instance) in cases {
            assert_eq!(device_instance(path), instance, "{path}");
        }
    }

    #[test]
    fn a_companion_shares_the_container_and_built_in_parts_share_the_codec() {
        let headset = facts("headset-out", Some(A2DP), Some(HEADSET_CONTAINER));
        let speakers = facts("speakers", Some(SPEAKERS), Some(SYSTEM_CONTAINER));
        let hdmi = facts("hdmi", Some(HDMI), Some(SYSTEM_CONTAINER));
        let lonely = facts("lonely", Some(USB_HEADSET), None);
        let inputs = [
            facts("internal-mic", Some(INTERNAL_MIC), Some(SYSTEM_CONTAINER)),
            facts("other-mic", Some(USB_HEADSET), Some(OTHER_CONTAINER)),
            facts("headset-mic", Some(HANDS_FREE), Some(HEADSET_CONTAINER)),
        ];
        let cases = [
            ("bluetooth headset", &headset, Some("headset-mic")),
            ("built-in speakers", &speakers, Some("internal-mic")),
            ("hdmi has no microphone", &hdmi, None),
            ("no container", &lonely, None),
        ];
        for (label, output, expected) in cases {
            assert_eq!(
                companion_in(output, &inputs),
                expected.map(Identity::from_raw),
                "{label}"
            );
        }
    }
}
