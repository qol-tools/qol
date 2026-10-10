use std::collections::HashMap;

use windows::core::{Interface, GUID};
use windows::Win32::Media::Audio::{
    IConnector, IMMDevice, IMMDeviceEnumerator, IPart, DEVICE_STATE, DEVICE_STATE_ACTIVE,
    DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, KSPROPSETID_BtAudio, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0,
    KSPROPERTY_BTAUDIO, KSPROPERTY_ONESHOT_DISCONNECT, KSPROPERTY_ONESHOT_RECONNECT,
    KSPROPERTY_TYPE_GET,
};
use windows::Win32::System::Com::CLSCTX_INPROC_SERVER;

use crate::bluetooth::BluetoothEndpoint;
use crate::devices::Direction;
use crate::AudioError;

use super::com;

const DIRECTIONS: [Direction; 2] = [Direction::Output, Direction::Input];
const LISTED_STATES: DEVICE_STATE = DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_UNPLUGGED.0);
const CLASSIC_BUS: &str = "bthenum#";
const CLASSIC_SUFFIX: &str = "_c00000000";
const ADDRESS_DIGITS: usize = 12;

struct Link {
    address: String,
    direction: Direction,
    active: bool,
    path: String,
    connector: IConnector,
}

struct Candidate {
    direction: Direction,
    active: bool,
    path: String,
    connector: IConnector,
    container: Option<GUID>,
}

pub fn bluetooth_endpoints() -> Result<Vec<BluetoothEndpoint>, AudioError> {
    com::with_enumerator(|enumerator| {
        Ok(links(enumerator)?
            .into_iter()
            .map(|link| BluetoothEndpoint {
                address: link.address,
                direction: link.direction,
                active: link.active,
            })
            .collect())
    })
}

pub fn reconnect_bluetooth(address: &str) -> Result<usize, AudioError> {
    request(address, KSPROPERTY_ONESHOT_RECONNECT)
}

pub fn disconnect_bluetooth(address: &str) -> Result<usize, AudioError> {
    request(address, KSPROPERTY_ONESHOT_DISCONNECT)
}

fn request(address: &str, property: KSPROPERTY_BTAUDIO) -> Result<usize, AudioError> {
    let wanted = address.to_ascii_uppercase();
    com::with_enumerator(|enumerator| {
        let mut asked: Vec<String> = Vec::new();
        let mut refusal = None;
        for link in links(enumerator)? {
            if link.address != wanted || asked.contains(&link.path) {
                continue;
            }
            match send(&link.connector, property) {
                Ok(()) => asked.push(link.path),
                Err(error) => refusal = Some(error),
            }
        }
        match (asked.len(), refusal) {
            (0, Some(error)) => Err(com::failed("Windows refused the Bluetooth audio request")(
                error,
            )),
            (count, _) => Ok(count),
        }
    })
}

fn send(connector: &IConnector, property: KSPROPERTY_BTAUDIO) -> windows::core::Result<()> {
    let part: IPart = unsafe { connector.GetConnectedTo() }?.cast()?;
    let mut raw = std::ptr::null_mut();
    unsafe { part.Activate(CLSCTX_INPROC_SERVER.0, &IKsControl::IID, Some(&mut raw)) }?;
    let control = unsafe { IKsControl::from_raw(raw) };
    let identifier = KSIDENTIFIER {
        Anonymous: KSIDENTIFIER_0 {
            Anonymous: KSIDENTIFIER_0_0 {
                Set: KSPROPSETID_BtAudio,
                Id: property.0 as u32,
                Flags: KSPROPERTY_TYPE_GET,
            },
        },
    };
    let mut returned = 0;
    unsafe {
        control.KsProperty(
            &identifier,
            size_of::<KSIDENTIFIER>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
        )
    }
}

fn links(enumerator: &IMMDeviceEnumerator) -> Result<Vec<Link>, AudioError> {
    let mut candidates = Vec::new();
    for direction in DIRECTIONS {
        for endpoint in com::endpoints(enumerator, direction, LISTED_STATES)? {
            if let Some(candidate) = candidate(direction, &endpoint.device) {
                candidates.push(candidate);
            }
        }
    }
    let containers = candidates
        .iter()
        .filter_map(|candidate| Some((candidate.container?, bluetooth_address(&candidate.path)?)))
        .collect::<HashMap<_, _>>();
    Ok(candidates
        .into_iter()
        .filter_map(|candidate| {
            let address = bluetooth_address(&candidate.path).or_else(|| {
                candidate
                    .container
                    .and_then(|container| containers.get(&container).cloned())
            })?;
            Some(Link {
                address,
                direction: candidate.direction,
                active: candidate.active,
                path: candidate.path,
                connector: candidate.connector,
            })
        })
        .collect())
}

fn candidate(direction: Direction, device: &IMMDevice) -> Option<Candidate> {
    let (path, connector) = com::topology_link(device)?;
    let active = unsafe { device.GetState() }.is_ok_and(|state| state == DEVICE_STATE_ACTIVE);
    Some(Candidate {
        direction,
        active,
        path,
        connector,
        container: com::container(device),
    })
}

fn bluetooth_address(path: &str) -> Option<String> {
    let path = path.to_ascii_lowercase();
    if !path.contains(CLASSIC_BUS) {
        return None;
    }
    let (head, _) = path.split_once(CLASSIC_SUFFIX)?;
    let digits = head.rsplit('&').next()?;
    if digits.len() != ADDRESS_DIGITS || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(
        digits
            .as_bytes()
            .chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_classic_bluetooth_filter_names_its_device_address() {
        let cases = [
            (
                "stereo audio",
                r"{2}.\\?\bthenum#{0000110b-0000-1000-8000-00805f9b34fb}_vid&0001000f_pid&1200_ver&0001#7&1a5ab4f3&0&7468597f5fe9_c00000000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\aanalogout",
                Some("74:68:59:7F:5F:E9"),
            ),
            (
                "upper case path",
                r"{2}.\\?\BTHENUM#{0000110B-0000-1000-8000-00805F9B34FB}_LOCALMFG&0002#7&2E5E4C6E&0&04FFAABBCCDD_C00000000#{6994AD04-93EF-11D0-A3CC-00A0C9223196}\WAVE",
                Some("04:FF:AA:BB:CC:DD"),
            ),
            (
                "hands-free filter names no address",
                r"{2}.\\?\bthhfenum#bthhfpaudio#8&2cb2a0a5&0&97#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\bthhfpaudiowave",
                None,
            ),
            (
                "usb device",
                r"{2}.\\?\usb#vid_046d&pid_0a44&mi_00#7&1f2b6c1&0&0000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\global",
                None,
            ),
            (
                "short address",
                r"{2}.\\?\bthenum#{0000110b-0000-1000-8000-00805f9b34fb}#7&1a5ab4f3&0&7468597f_c00000000#{guid}",
                None,
            ),
        ];
        for (label, path, address) in cases {
            assert_eq!(bluetooth_address(path).as_deref(), address, "{label}");
        }
    }
}
