use std::collections::{HashMap, HashSet};

use crate::bluetooth::{device_order, without_settings_twins, DeviceInfo, Pairing};

const RSSI_UNAVAILABLE: i64 = 127;
const BLUETOOTH_BASE_UUID_TAIL: &str = "-0000-1000-8000-00805f9b34fb";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Advertisement {
    pub(super) identifier: String,
    pub(super) local_name: Option<String>,
    pub(super) peripheral_name: Option<String>,
    pub(super) rssi: i64,
    pub(super) connectable: bool,
    pub(super) service_uuids: Vec<String>,
}

impl Advertisement {
    fn absorb(&mut self, latest: Advertisement) {
        self.local_name = named(latest.local_name).or(self.local_name.take());
        self.peripheral_name = named(latest.peripheral_name).or(self.peripheral_name.take());
        if latest.rssi != RSSI_UNAVAILABLE {
            self.rssi = latest.rssi;
        }
        self.connectable |= latest.connectable;
        for uuid in latest.service_uuids {
            if !self.service_uuids.contains(&uuid) {
                self.service_uuids.push(uuid);
            }
        }
    }

    fn device(&self) -> Option<DeviceInfo> {
        if !self.connectable {
            return None;
        }
        let alias = [&self.local_name, &self.peripheral_name]
            .into_iter()
            .flatten()
            .map(|name| name.trim())
            .find(|name| !name.is_empty())?;
        let mut uuids = self
            .service_uuids
            .iter()
            .filter_map(|uuid| full_uuid(uuid))
            .collect::<Vec<_>>();
        uuids.sort();
        uuids.dedup();
        Some(DeviceInfo {
            address: self.identifier.clone(),
            alias: alias.to_string(),
            paired: false,
            trusted: false,
            connected: false,
            audio_connected: None,
            services_resolved: false,
            icon: None,
            class: None,
            uuids,
            rssi: signal(self.rssi),
            pairing: Pairing::SystemSettings,
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct Sightings {
    latest: HashMap<String, Advertisement>,
}

impl Sightings {
    pub(super) fn record(&mut self, advertisement: Advertisement) {
        match self.latest.get_mut(&advertisement.identifier) {
            Some(known) => known.absorb(advertisement),
            None => {
                self.latest
                    .insert(advertisement.identifier.clone(), advertisement);
            }
        }
    }

    pub(super) fn devices(&self) -> Vec<DeviceInfo> {
        self.latest
            .values()
            .filter_map(Advertisement::device)
            .collect()
    }
}

fn named(name: Option<String>) -> Option<String> {
    name.filter(|name| !name.trim().is_empty())
}

fn signal(rssi: i64) -> Option<i16> {
    if rssi == RSSI_UNAVAILABLE {
        return None;
    }
    i16::try_from(rssi).ok()
}

fn full_uuid(uuid: &str) -> Option<String> {
    let uuid = uuid.trim().to_ascii_lowercase();
    let groups = uuid.split('-').collect::<Vec<_>>();
    let hex = groups
        .iter()
        .all(|group| !group.is_empty() && group.chars().all(|digit| digit.is_ascii_hexdigit()));
    let lengths = groups.iter().map(|group| group.len()).collect::<Vec<_>>();
    match lengths.as_slice() {
        _ if !hex => None,
        [4] => Some(format!("0000{uuid}{BLUETOOTH_BASE_UUID_TAIL}")),
        [8] => Some(format!("{uuid}{BLUETOOTH_BASE_UUID_TAIL}")),
        [8, 4, 4, 4, 12] => Some(uuid),
        _ => None,
    }
}

pub(super) fn merge(devices: impl IntoIterator<Item = DeviceInfo>) -> Vec<DeviceInfo> {
    let unique = unique(devices);
    let kept = without_settings_twins(unique.iter().collect())
        .into_iter()
        .map(|device| device.address.clone())
        .collect::<HashSet<_>>();
    let mut merged = unique
        .into_iter()
        .filter(|device| kept.contains(&device.address))
        .collect::<Vec<_>>();
    merged.sort_by(device_order);
    merged
}

pub(super) fn unique(devices: impl IntoIterator<Item = DeviceInfo>) -> Vec<DeviceInfo> {
    let mut unique: Vec<DeviceInfo> = Vec::new();
    for device in devices {
        match unique
            .iter_mut()
            .find(|known| known.address == device.address)
        {
            Some(known) if richer_duplicate(known, &device) => *known = device,
            Some(_) => {}
            None => unique.push(device),
        }
    }
    unique
}

fn richer_duplicate(existing: &DeviceInfo, candidate: &DeviceInfo) -> bool {
    (candidate.connected, candidate.paired, candidate.uuids.len())
        > (existing.connected, existing.paired, existing.uuids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIFT: &str = "31CAE991-2662-5D9B-D7B9-8CE2CD5FF04C";
    const KEYBOARD: &str = "9F6F5B65-F881-6552-4B8E-85914E7FAFD7";
    const HEADSET: &str = "6821FD4A-BC6D-2C39-E44B-FB2906C8B38F";
    const PHONE: &str = "5A1D7C3E-8B2F-4E6A-9C1D-3F7B2E8A6D4C";
    const BATTERY: &str = "0000180f-0000-1000-8000-00805f9b34fb";
    const DEVICE_INFORMATION: &str = "0000180a-0000-1000-8000-00805f9b34fb";

    fn advertisement(identifier: &str, local_name: Option<&str>, rssi: i64) -> Advertisement {
        Advertisement {
            identifier: identifier.into(),
            local_name: local_name.map(str::to_string),
            peripheral_name: None,
            rssi,
            connectable: true,
            service_uuids: Vec::new(),
        }
    }

    fn classic(address: &str, alias: &str, paired: bool, connected: bool) -> DeviceInfo {
        DeviceInfo {
            address: address.into(),
            alias: alias.into(),
            paired,
            trusted: paired,
            connected,
            audio_connected: None,
            services_resolved: false,
            icon: None,
            class: Some(0x240404),
            uuids: Vec::new(),
            rssi: None,
            pairing: Pairing::Backend,
        }
    }

    fn sighted(advertisements: impl IntoIterator<Item = Advertisement>) -> Vec<DeviceInfo> {
        let mut sightings = Sightings::default();
        for advertisement in advertisements {
            sightings.record(advertisement);
        }
        let mut devices = sightings.devices();
        devices.sort_by(|left, right| left.address.cmp(&right.address));
        devices
    }

    fn addresses(devices: &[DeviceInfo]) -> Vec<&str> {
        devices
            .iter()
            .map(|device| device.address.as_str())
            .collect()
    }

    #[test]
    fn a_named_connectable_advertiser_becomes_a_settings_paired_device() {
        let devices = sighted([Advertisement {
            service_uuids: vec!["180F".into(), "180A".into(), "180F".into()],
            ..advertisement(LIFT, Some("LIFT"), -58)
        }]);
        assert_eq!(
            devices,
            [DeviceInfo {
                address: LIFT.into(),
                alias: "LIFT".into(),
                paired: false,
                trusted: false,
                connected: false,
                audio_connected: None,
                services_resolved: false,
                icon: None,
                class: None,
                uuids: vec![DEVICE_INFORMATION.into(), BATTERY.into()],
                rssi: Some(-58),
                pairing: Pairing::SystemSettings,
            }]
        );
    }

    #[test]
    fn advertisers_that_cannot_be_paired_are_left_out() {
        let cases = [
            ("nameless", advertisement(LIFT, None, -40)),
            ("blank name", advertisement(LIFT, Some("  "), -40)),
            (
                "not connectable",
                Advertisement {
                    connectable: false,
                    ..advertisement(LIFT, Some("BYD BLE3"), -40)
                },
            ),
        ];
        for (label, advertisement) in cases {
            assert!(sighted([advertisement]).is_empty(), "case: {label}");
        }
    }

    #[test]
    fn the_advertised_name_wins_over_the_cached_peripheral_name() {
        let cases = [
            (Some("MX KEYS B"), Some("Keyboard"), "MX KEYS B"),
            (Some(" "), Some("Keyboard"), "Keyboard"),
            (None, Some(" MX KEYS B "), "MX KEYS B"),
        ];
        for (local_name, peripheral_name, alias) in cases {
            let devices = sighted([Advertisement {
                peripheral_name: peripheral_name.map(str::to_string),
                ..advertisement(KEYBOARD, local_name, -60)
            }]);
            assert_eq!(
                devices[0].alias, alias,
                "{local_name:?} {peripheral_name:?}"
            );
        }
    }

    #[test]
    fn repeated_advertisements_keep_the_name_signal_and_services_seen_so_far() {
        let devices = sighted([
            Advertisement {
                connectable: false,
                service_uuids: vec!["180F".into()],
                ..advertisement(LIFT, None, -70)
            },
            Advertisement {
                service_uuids: vec!["180A".into()],
                ..advertisement(LIFT, Some("LIFT"), -61)
            },
            advertisement(LIFT, None, RSSI_UNAVAILABLE),
            Advertisement {
                connectable: false,
                ..advertisement(LIFT, Some(""), -55)
            },
        ]);
        assert_eq!(addresses(&devices), [LIFT]);
        assert_eq!(devices[0].alias, "LIFT");
        assert_eq!(devices[0].rssi, Some(-55));
        assert_eq!(devices[0].uuids, [DEVICE_INFORMATION, BATTERY]);
    }

    #[test]
    fn an_unavailable_or_impossible_signal_is_reported_as_unknown() {
        let cases = [
            (-58, Some(-58)),
            (0, Some(0)),
            (RSSI_UNAVAILABLE, None),
            (i64::from(i16::MIN) - 1, None),
        ];
        for (rssi, expected) in cases {
            assert_eq!(signal(rssi), expected, "{rssi}");
        }
    }

    #[test]
    fn service_uuids_reach_the_domain_in_full_lowercase_form() {
        let cases = [
            ("180F", Some(BATTERY)),
            ("0000180f", Some(BATTERY)),
            (
                "6E400001-B5A3-F393-E0A9-E50E24DCCA9E",
                Some("6e400001-b5a3-f393-e0a9-e50e24dcca9e"),
            ),
            ("180", None),
            ("18GF", None),
            ("6E400001B5A3F393E0A9E50E24DCCA9E", None),
            ("6E400001-B5A3-F393-E0A9-E50E24DCCA9", None),
            ("", None),
        ];
        for (uuid, expected) in cases {
            assert_eq!(full_uuid(uuid).as_deref(), expected, "{uuid:?}");
        }
    }

    #[test]
    fn a_low_energy_advertiser_named_like_a_known_device_is_folded_into_it() {
        let merged = merge(
            [
                classic("DF:6F:5A:74:63:71", "MX KEYS B", true, true),
                classic("64:FF:0A:CE:FF:EE", "Galaxy S21 5G", false, false),
            ]
            .into_iter()
            .chain(sighted([
                advertisement(KEYBOARD, Some("mx keys b"), -60),
                advertisement(PHONE, Some("Galaxy S21 5G "), -65),
                advertisement(HEADSET, Some("A Headset KWE"), -71),
            ])),
        );
        assert_eq!(
            addresses(&merged),
            ["DF:6F:5A:74:63:71", HEADSET, "64:FF:0A:CE:FF:EE"]
        );
    }

    #[test]
    fn search_results_list_each_device_once_ordered_like_the_settings_list() {
        let merged = merge([
            classic("88:0E:85:16:CA:67", "soundcore P40i", true, false),
            classic("DA:04:03:7D:08:7E", "LIFT", true, true),
            DeviceInfo {
                rssi: Some(-48),
                ..classic("88:0E:85:16:CA:67", "soundcore P40i", true, false)
            },
            DeviceInfo {
                rssi: Some(-75),
                ..classic("64:FF:0A:CE:FF:EE", "Galaxy S21 5G", false, false)
            },
            sighted([advertisement(HEADSET, Some("A Headset KWE"), -71)]).remove(0),
        ]);
        assert_eq!(
            addresses(&merged),
            [
                "DA:04:03:7D:08:7E",
                "88:0E:85:16:CA:67",
                HEADSET,
                "64:FF:0A:CE:FF:EE"
            ]
        );
    }

    #[test]
    fn a_dual_mode_device_listed_twice_keeps_its_richest_entry() {
        let bare = classic("AA:BB:CC:DD:EE:FF", "Jabra Evolve2 65 Flex", true, false);
        let connected = DeviceInfo {
            connected: true,
            uuids: vec!["0000110b-0000-1000-8000-00805f9b34fb".into()],
            ..bare.clone()
        };

        assert!(richer_duplicate(&bare, &connected));
        assert!(!richer_duplicate(&connected, &bare));
        for listed in [
            [bare.clone(), connected.clone()],
            [connected.clone(), bare.clone()],
        ] {
            assert_eq!(unique(listed), std::slice::from_ref(&connected));
        }
    }
}
