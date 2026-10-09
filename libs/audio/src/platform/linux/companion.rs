use pulseaudio::protocol;

use crate::devices::{Device, Direction, Identity};
use crate::AudioError;

use super::devices::{sink_device, source_device};
use super::identity::{device_identity, listed};

const ADDRESS_OCTETS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Endpoint {
    identity: Identity,
    card: Option<u32>,
    bluetooth_address: Option<String>,
}

pub(crate) fn companion_input(output: &Identity) -> Result<Option<Identity>, AudioError> {
    super::with_connection(|connection| {
        let sinks =
            connection.request::<Vec<protocol::SinkInfo>>(&protocol::Command::GetSinkInfoList)?;
        let sources = connection
            .request::<Vec<protocol::SourceInfo>>(&protocol::Command::GetSourceInfoList)?;
        let outputs = sinks
            .iter()
            .map(|sink| endpoint(&sink_device(sink), sink.card_index))
            .collect::<Vec<_>>();
        let inputs = sources
            .iter()
            .map(|source| (source_device(source), source.card_index))
            .filter(|(device, _)| listed(Direction::Input, device))
            .map(|(device, card)| endpoint(&device, card))
            .collect::<Vec<_>>();
        Ok(companion_in(output, &outputs, &inputs))
    })
}

fn companion_in(output: &Identity, outputs: &[Endpoint], inputs: &[Endpoint]) -> Option<Identity> {
    let output = outputs
        .iter()
        .find(|candidate| candidate.identity == *output)?;
    inputs
        .iter()
        .find(|input| same_card(output, input))
        .or_else(|| inputs.iter().find(|input| same_address(output, input)))
        .map(|input| input.identity.clone())
}

fn same_card(output: &Endpoint, input: &Endpoint) -> bool {
    output.card.is_some() && output.card == input.card
}

fn same_address(output: &Endpoint, input: &Endpoint) -> bool {
    output.bluetooth_address.is_some() && output.bluetooth_address == input.bluetooth_address
}

fn endpoint(device: &Device, card: Option<u32>) -> Endpoint {
    Endpoint {
        identity: device_identity(device),
        card: card.filter(|index| *index != u32::MAX),
        bluetooth_address: bluetooth_address(device),
    }
}

fn bluetooth_address(device: &Device) -> Option<String> {
    let property = |key: &str| device.properties.get(key).map(String::as_str);
    let on_bluetooth = property("device.bus") == Some("bluetooth");
    property("api.bluez5.address")
        .and_then(normalized_address)
        .or_else(|| {
            property("device.string")
                .filter(|_| on_bluetooth)
                .and_then(normalized_address)
        })
        .or_else(|| {
            property("bluez.path")
                .and_then(|path| path.rsplit('/').next())
                .and_then(|segment| segment.strip_prefix("dev_"))
                .and_then(normalized_address)
        })
}

fn normalized_address(raw: &str) -> Option<String> {
    let octets = raw
        .trim()
        .split([':', '_'])
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    let valid = octets.len() == ADDRESS_OCTETS
        && octets
            .iter()
            .all(|octet| octet.len() == 2 && octet.chars().all(|c| c.is_ascii_hexdigit()));
    valid.then(|| octets.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::State;

    fn device(name: &str, properties: &[(&str, &str)]) -> Device {
        Device {
            index: 0,
            name: name.to_owned(),
            description: name.to_owned(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            active_port: None,
            monitor_of_sink: None,
            state: State::Unknown,
        }
    }

    fn at(name: &str, card: Option<u32>, properties: &[(&str, &str)]) -> Endpoint {
        let mut all = vec![("device.name", name)];
        all.extend_from_slice(properties);
        endpoint(&device(name, &all), card)
    }

    #[test]
    fn the_microphone_on_the_same_card_is_the_companion() {
        let outputs = [
            at("usb_headset_out", Some(4), &[]),
            at("speakers", Some(1), &[]),
        ];
        let inputs = [
            at("internal_mic", Some(1), &[]),
            at("usb_headset_mic", Some(4), &[]),
        ];

        assert_eq!(
            companion_in(&outputs[0].identity, &outputs, &inputs),
            Some(inputs[1].identity.clone())
        );
    }

    #[test]
    fn a_bluetooth_microphone_on_another_card_matches_by_address() {
        let outputs = [at(
            "bluez_output",
            Some(7),
            &[("api.bluez5.address", "74:68:59:7f:5f:e9")],
        )];
        let inputs = [
            at("internal_mic", Some(1), &[]),
            at(
                "bluez_input",
                Some(9),
                &[
                    ("device.bus", "bluetooth"),
                    ("device.string", "74_68_59_7F_5F_E9"),
                ],
            ),
        ];

        assert_eq!(
            companion_in(&outputs[0].identity, &outputs, &inputs),
            Some(inputs[1].identity.clone())
        );
    }

    #[test]
    fn a_card_match_wins_over_an_address_match() {
        let address = ("api.bluez5.address", "74:68:59:7F:5F:E9");
        let outputs = [at("bluez_output", Some(7), &[address])];
        let inputs = [
            at("other_bluez_input", Some(8), &[address]),
            at("bluez_input", Some(7), &[]),
        ];

        assert_eq!(
            companion_in(&outputs[0].identity, &outputs, &inputs),
            Some(inputs[1].identity.clone())
        );
    }

    #[test]
    fn no_shared_card_or_address_and_an_absent_output_have_no_companion() {
        let outputs = [at("hdmi", Some(2), &[]), at("unbound", None, &[])];
        let inputs = [at("internal_mic", Some(1), &[]), at("loose_mic", None, &[])];

        assert_eq!(companion_in(&outputs[0].identity, &outputs, &inputs), None);
        assert_eq!(companion_in(&outputs[1].identity, &outputs, &inputs), None);
        assert_eq!(
            companion_in(&Identity::from_raw("gone:-:-"), &outputs, &inputs),
            None
        );
    }

    #[test]
    fn bluetooth_addresses_normalize_and_ignore_non_addresses() {
        assert_eq!(
            normalized_address("74_68_59_7f_5f_e9").as_deref(),
            Some("74:68:59:7F:5F:E9")
        );
        assert_eq!(normalized_address("hw:1,0"), None);
        assert_eq!(normalized_address("74:68:59:7F:5F"), None);
        assert_eq!(normalized_address("ZZ:68:59:7F:5F:E9"), None);

        let from_path = device(
            "bluez_source",
            &[("bluez.path", "/org/bluez/hci0/dev_74_68_59_7F_5F_E9")],
        );
        assert_eq!(
            bluetooth_address(&from_path).as_deref(),
            Some("74:68:59:7F:5F:E9")
        );

        let wired = device("alsa_input", &[("device.string", "front:1")]);
        assert_eq!(bluetooth_address(&wired), None);
    }
}
