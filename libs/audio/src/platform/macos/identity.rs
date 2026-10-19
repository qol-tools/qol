use crate::devices::{AudioDevice, Device, Direction, Identity, Kind, Resolution, State};
use crate::AudioError;

use super::hal::{self, ObjectId, Property};

const BUILT_IN: [u8; 4] = *b"bltn";
const USB: [u8; 4] = *b"usb ";
const BLUETOOTH: [u8; 4] = *b"blue";
const BLUETOOTH_LOW_ENERGY: [u8; 4] = *b"blea";
const HDMI: [u8; 4] = *b"hdmi";
const DISPLAY_PORT: [u8; 4] = *b"dprt";

struct Facts {
    id: ObjectId,
    identity: Identity,
    label: String,
    kind: Kind,
}

fn offers(id: ObjectId, direction: Direction) -> bool {
    let flag = |property| hal::read_u32(id, property).is_ok_and(|value| value != 0);
    hal::read_ids(id, Property::scoped(hal::STREAMS, direction))
        .is_ok_and(|streams| !streams.is_empty())
        && flag(Property::scoped(hal::CAN_BE_DEFAULT, direction))
        && flag(Property::global(hal::IS_ALIVE))
        && !flag(Property::global(hal::IS_HIDDEN))
}

fn facts(id: ObjectId) -> Option<Facts> {
    let uid = hal::read_text(id, Property::global(hal::DEVICE_UID)).ok()?;
    let label = hal::read_text(id, Property::global(hal::NAME)).unwrap_or_else(|_| uid.clone());
    let transport = hal::read_u32(id, Property::global(hal::TRANSPORT)).unwrap_or_default();
    Some(Facts {
        id,
        identity: Identity::from_raw(uid),
        label,
        kind: kind_of(transport),
    })
}

fn active_facts(direction: Direction) -> Result<Vec<Facts>, AudioError> {
    Ok(hal::read_ids(hal::SYSTEM, Property::global(hal::DEVICES))?
        .into_iter()
        .filter(|id| offers(*id, direction))
        .filter_map(facts)
        .collect())
}

pub(super) fn device_for(
    direction: Direction,
    identity: &Identity,
) -> Result<Option<ObjectId>, AudioError> {
    Ok(active_facts(direction)?
        .into_iter()
        .find(|facts| facts.identity == *identity)
        .map(|facts| facts.id))
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
        .map(|facts| Device {
            index: facts.id,
            state: state_of(facts.id),
            name: facts.identity.as_str().to_owned(),
            description: facts.label,
            properties: Default::default(),
            active_port: None,
            monitor_of_sink: None,
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
    Ok(active_facts(direction)?
        .into_iter()
        .find(|facts| facts.identity.as_str() == node)
        .map(|facts| facts.identity))
}

pub(crate) fn companion_input(output: &Identity) -> Result<Option<Identity>, AudioError> {
    let outputs = active_facts(Direction::Output)?;
    let Some(output) = outputs.iter().find(|facts| facts.identity == *output) else {
        return Ok(None);
    };
    let related =
        hal::read_ids(output.id, Property::global(hal::RELATED_DEVICES)).unwrap_or_default();
    Ok(companion_in(
        output,
        &related,
        &active_facts(Direction::Input)?,
    ))
}

fn companion_in(output: &Facts, related: &[ObjectId], inputs: &[Facts]) -> Option<Identity> {
    let shared = inputs.iter().find(|input| input.id == output.id);
    let linked = || inputs.iter().find(|input| related.contains(&input.id));
    let paired = || {
        let address = bluetooth_address(output)?;
        inputs
            .iter()
            .find(|input| bluetooth_address(input) == Some(address))
    };
    shared
        .or_else(linked)
        .or_else(paired)
        .map(|input| input.identity.clone())
}

fn bluetooth_address(facts: &Facts) -> Option<&str> {
    if facts.kind != Kind::Bluetooth {
        return None;
    }
    facts
        .identity
        .as_str()
        .rsplit_once(':')
        .map(|(address, _)| address)
}

pub(super) fn state_of(id: ObjectId) -> State {
    match hal::read_u32(id, Property::global(hal::IS_RUNNING_SOMEWHERE)) {
        Ok(0) => State::Idle,
        Ok(_) => State::Running,
        Err(_) => State::Unknown,
    }
}

pub(super) fn is_bluetooth(id: ObjectId) -> bool {
    let transport = hal::read_u32(id, Property::global(hal::TRANSPORT)).unwrap_or_default();
    kind_of(transport) == Kind::Bluetooth
}

fn kind_of(transport: u32) -> Kind {
    match transport.to_be_bytes() {
        BLUETOOTH | BLUETOOTH_LOW_ENERGY => Kind::Bluetooth,
        USB => Kind::Usb,
        HDMI | DISPLAY_PORT => Kind::Hdmi,
        BUILT_IN => Kind::BuiltIn,
        _ => Kind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(id: ObjectId, uid: &str, kind: Kind) -> Facts {
        Facts {
            id,
            identity: Identity::from_raw(uid),
            label: uid.to_owned(),
            kind,
        }
    }

    #[test]
    fn the_kind_comes_from_the_transport_type() {
        let cases = [
            (*b"bltn", Kind::BuiltIn),
            (*b"usb ", Kind::Usb),
            (*b"blue", Kind::Bluetooth),
            (*b"blea", Kind::Bluetooth),
            (*b"hdmi", Kind::Hdmi),
            (*b"dprt", Kind::Hdmi),
            (*b"airp", Kind::Unknown),
            (*b"virt", Kind::Unknown),
            ([0; 4], Kind::Unknown),
        ];
        for (transport, kind) in cases {
            assert_eq!(
                kind_of(u32::from_be_bytes(transport)),
                kind,
                "{}",
                String::from_utf8_lossy(&transport)
            );
        }
    }

    #[test]
    fn a_companion_is_the_same_device_then_a_related_one_then_the_paired_headset() {
        let speakers = facts(70, "BuiltInSpeakerDevice", Kind::BuiltIn);
        let headset_out = facts(82, "88-0E-85-16-CA-67:output", Kind::Bluetooth);
        let other_out = facts(90, "11-22-33-44-55-66:output", Kind::Bluetooth);
        let usb_headset = facts(60, "AppleUSBAudioEngine:Headset", Kind::Usb);
        let display = facts(50, "DisplayAudio", Kind::Hdmi);
        let inputs = [
            facts(77, "BuiltInMicrophoneDevice", Kind::BuiltIn),
            facts(88, "88-0E-85-16-CA-67:input", Kind::Bluetooth),
            facts(60, "AppleUSBAudioEngine:Headset", Kind::Usb),
        ];
        let cases = [
            (
                "built-in speakers",
                &speakers,
                vec![70, 77],
                Some("BuiltInMicrophoneDevice"),
            ),
            (
                "bluetooth headset",
                &headset_out,
                vec![82],
                Some("88-0E-85-16-CA-67:input"),
            ),
            ("another headset", &other_out, vec![90], None),
            (
                "usb headset",
                &usb_headset,
                vec![60],
                Some("AppleUSBAudioEngine:Headset"),
            ),
            ("display has no microphone", &display, vec![50], None),
        ];
        for (label, output, related, expected) in cases {
            assert_eq!(
                companion_in(output, &related, &inputs),
                expected.map(Identity::from_raw),
                "{label}"
            );
        }
    }
}
