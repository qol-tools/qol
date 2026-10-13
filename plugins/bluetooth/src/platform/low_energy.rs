use crate::bluetooth::DeviceInfo;

const DEVICE_PREFIX: &str = "dev_";
const ADDRESS_DIGITS: usize = 12;
const DEVNODE_DISCONNECTED: u32 = 0x0200_0000;

pub(super) fn address(device_id: &str) -> Option<u64> {
    let lower = device_id.to_ascii_lowercase();
    let start = lower.find(DEVICE_PREFIX)? + DEVICE_PREFIX.len();
    let rest = lower.get(start..)?;
    let digits = rest.get(..ADDRESS_DIGITS)?;
    let bounded = !rest[ADDRESS_DIGITS..].starts_with(|next: char| next.is_ascii_hexdigit());
    if !bounded || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

pub(super) fn connected(status: Option<u32>) -> bool {
    status.is_some_and(|status| status & DEVNODE_DISCONNECTED == 0)
}

pub(super) fn device(
    address: String,
    name: &str,
    connected: bool,
    audio_connected: Option<bool>,
) -> DeviceInfo {
    let name = name.trim();
    DeviceInfo {
        alias: if name.is_empty() {
            address.clone()
        } else {
            name.to_owned()
        },
        paired: true,
        trusted: true,
        connected,
        audio_connected,
        services_resolved: false,
        icon: None,
        class: None,
        uuids: Vec::new(),
        rssi: None,
        address,
    }
}

pub(super) fn merge(mut classic: Vec<DeviceInfo>, low_energy: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    for device in low_energy {
        let address = device.address.as_str();
        match classic.iter_mut().find(|known| known.address == address) {
            Some(known) => known.connected |= device.connected,
            None => classic.push(device),
        }
    }
    classic
}

pub(super) fn refusal(alias: &str) -> String {
    format!(
        "{alias} is a Bluetooth LE device, and Windows has no call that lets qol connect, disconnect, pair or remove it; use Settings > Bluetooth & devices"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUSE: &str = "AA:BB:CC:DD:EE:01";
    const HEADSET: &str = "AA:BB:CC:DD:EE:02";

    fn classic(address: &str, connected: bool) -> DeviceInfo {
        DeviceInfo {
            class: Some(0x240404),
            services_resolved: true,
            ..device(address.to_string(), "Headset", connected, None)
        }
    }

    #[test]
    fn the_address_comes_from_the_device_segment_of_an_id() {
        let cases = [
            (
                r"\\?\BTHLE#Dev_aabbccddee01#7&2a1c0b5e&0&BluetoothLE#{781aee18-7733-4ce4-add0-91f41c67b592}",
                Some(0xaabb_ccdd_ee01),
            ),
            (
                r"BTHLE\DEV_AABBCCDDEE01\7&2A1C0B5E&0&AABBCCDDEE01",
                Some(0xaabb_ccdd_ee01),
            ),
            (r"BTHLE\DEV_AABBCCDDEE\7&0", None),
            (r"BTHLE\DEV_AABBCCDDEE0123\7&0", None),
            (r"BTHLE\DEV_AABBCCDDEEZZ\7&0", None),
            (r"BTHENUM\{0000110b}_LOCALMFG&0002\7&0", None),
            ("", None),
        ];
        for (id, expected) in cases {
            assert_eq!(address(id), expected, "{id}");
        }
    }

    #[test]
    fn a_device_is_connected_only_when_its_node_is_not_disconnected() {
        let cases = [
            (Some(0x0180_200a), true),
            (Some(0x0180_200a | DEVNODE_DISCONNECTED), false),
            (Some(DEVNODE_DISCONNECTED), false),
            (Some(0), true),
            (None, false),
        ];
        for (status, expected) in cases {
            assert_eq!(connected(status), expected, "{status:?}");
        }
    }

    #[test]
    fn a_device_without_a_name_is_called_by_its_address() {
        let cases = [
            ("MX Master 3", "MX Master 3"),
            ("  Pad  ", "Pad"),
            ("", MOUSE),
            (" ", MOUSE),
        ];
        for (name, alias) in cases {
            let device = device(MOUSE.to_string(), name, true, Some(false));
            assert_eq!(device.alias, alias, "{name:?}");
            assert!(device.paired && device.trusted && device.connected);
            assert_eq!(device.audio_connected, Some(false));
            assert!(device.uuids.is_empty() && device.class.is_none());
        }
    }

    #[test]
    fn a_dual_mode_device_appears_once_as_its_classic_entry() {
        let merged = merge(
            vec![classic(HEADSET, false)],
            vec![
                device(HEADSET.to_string(), "Headset LE", true, None),
                device(MOUSE.to_string(), "Mouse", false, None),
                device(MOUSE.to_string(), "Mouse", true, None),
            ],
        );
        let addresses = merged
            .iter()
            .map(|device| device.address.as_str())
            .collect::<Vec<_>>();
        assert_eq!(addresses, [HEADSET, MOUSE]);
        assert_eq!(merged[0].alias, "Headset");
        assert_eq!(merged[0].class, Some(0x240404));
        assert!(merged[0].connected);
        assert!(merged[1].connected);
    }

    #[test]
    fn a_classic_list_without_low_energy_devices_is_unchanged() {
        let devices = vec![classic(HEADSET, true), classic(MOUSE, false)];
        assert_eq!(merge(devices.clone(), Vec::new()), devices);
    }

    #[test]
    fn the_refusal_names_the_device_and_where_to_manage_it() {
        let message = refusal("MX Master 3");
        assert!(
            message.starts_with("MX Master 3 is a Bluetooth LE device"),
            "{message}"
        );
        assert!(
            message.contains("Settings > Bluetooth & devices"),
            "{message}"
        );
    }
}
