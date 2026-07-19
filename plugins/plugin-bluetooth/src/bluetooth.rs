use std::collections::{HashMap, HashSet};

use serde::Serialize;

use anyhow::{bail, Result};

pub fn normalize_address(value: &str) -> Result<String> {
    let parts = value.trim().split(':').collect::<Vec<_>>();
    if parts.len() != 6
        || parts
            .iter()
            .any(|part| part.len() != 2 || !part.chars().all(|char| char.is_ascii_hexdigit()))
    {
        bail!("invalid Bluetooth address `{value}`; expected AA:BB:CC:DD:EE:FF");
    }
    Ok(parts.join(":").to_ascii_uppercase())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    pub address: String,
    pub alias: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub services_resolved: bool,
    pub icon: Option<String>,
    pub class: Option<u32>,
    pub uuids: Vec<String>,
    pub rssi: Option<i16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceActionState {
    pub address: String,
    pub status: String,
    pub pending: bool,
}

const AUDIO_MAJOR_CLASS: u32 = 0x0400;
const MAJOR_CLASS_MASK: u32 = 0x1f00;
const AUDIO_SINK_UUID: &str = "0000110b-0000-1000-8000-00805f9b34fb";

pub fn is_audio_device(device: &DeviceInfo) -> bool {
    device
        .icon
        .as_deref()
        .is_some_and(|icon| icon.starts_with("audio-"))
        || has_audio_class(device)
}

pub fn has_audio_class(device: &DeviceInfo) -> bool {
    device
        .class
        .is_some_and(|class| class & MAJOR_CLASS_MASK == AUDIO_MAJOR_CLASS)
}

pub fn supports_audio_sink(device: &DeviceInfo) -> bool {
    device.uuids.iter().any(|uuid| uuid == AUDIO_SINK_UUID)
}

pub fn connection_ready(device: &DeviceInfo) -> bool {
    if !device.connected {
        return false;
    }
    if !is_audio_device(device) {
        return true;
    }
    supports_audio_sink(device)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveryState {
    searching: bool,
    addresses: HashSet<String>,
    devices: HashMap<String, DeviceInfo>,
}

impl DiscoveryState {
    pub fn start(&mut self) {
        self.addresses.clear();
        self.devices.clear();
        self.searching = true;
    }

    pub fn stop(&mut self) {
        self.searching = false;
    }

    pub fn record(&mut self, address: impl Into<String>) {
        self.addresses.insert(address.into());
    }

    pub fn record_device(&mut self, device: DeviceInfo) {
        self.addresses.insert(device.address.clone());
        self.devices.insert(device.address.clone(), device);
    }

    pub fn remove(&mut self, address: &str) {
        self.addresses.remove(address);
        self.devices.remove(address);
    }

    pub fn searching(&self) -> bool {
        self.searching
    }

    pub fn discovered_count(&self) -> usize {
        self.addresses.len()
    }

    pub fn contains(&self, address: &str) -> bool {
        self.addresses.contains(address)
    }

    pub fn device(&self, address: &str) -> Option<DeviceInfo> {
        self.devices.get(address).cloned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconnectSelection {
    Managed,
    Trusted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReconnectFailure {
    pub address: String,
    pub alias: String,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReconnectReport {
    pub connected: Vec<DeviceInfo>,
    pub already_connected: Vec<DeviceInfo>,
    pub failures: Vec<ReconnectFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdapterHealth {
    pub name: String,
    pub address: String,
    pub powered: bool,
}

pub fn devices_payload(
    devices: &[DeviceInfo],
    managed_devices: &[String],
    discovery: &DiscoveryState,
    action: Option<&DeviceActionState>,
) -> serde_json::Value {
    let managed = managed_devices
        .iter()
        .filter_map(|address| normalize_address(address).ok())
        .collect::<HashSet<_>>();
    let mut visible = devices
        .iter()
        .filter(|device| device.paired || discovery.contains(&device.address))
        .collect::<Vec<_>>();
    let current_addresses = devices
        .iter()
        .map(|device| device.address.as_str())
        .collect::<HashSet<_>>();
    visible.extend(
        discovery
            .devices
            .values()
            .filter(|device| !current_addresses.contains(device.address.as_str())),
    );
    visible.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.paired.cmp(&left.paired))
            .then_with(|| right.rssi.cmp(&left.rssi))
            .then_with(|| left.alias.to_lowercase().cmp(&right.alias.to_lowercase()))
            .then_with(|| left.address.cmp(&right.address))
    });
    let connected_count = visible.iter().filter(|device| device.connected).count();
    let ready_count = visible
        .iter()
        .filter(|device| connection_ready(device))
        .count();
    let paired_count = visible.iter().filter(|device| device.paired).count();
    let items = visible
        .iter()
        .map(|device| {
            let ready = connection_ready(device);
            let audio = is_audio_device(device);
            let device_action = action.filter(|action| action.address == device.address);
            let action_pending = device_action.is_some_and(|action| action.pending);
            let device_status = if ready {
                "Connected"
            } else if device.connected && audio {
                "Connected without audio"
            } else if device.paired {
                "Paired"
            } else {
                "Available"
            };
            let signal = device
                .rssi
                .map(|rssi| format!("{rssi} dBm"))
                .unwrap_or_else(|| "Signal unavailable".into());
            let status = device_action
                .map(|action| action.status.as_str())
                .unwrap_or(device_status);
            let action_failed = device_action.is_some_and(|action| !action.pending);
            let accent = if action_failed {
                "danger"
            } else if device.connected {
                "success"
            } else if device.paired {
                "accent"
            } else {
                "muted"
            };
            let badge = if action_pending {
                "Working"
            } else if action_failed {
                "Needs attention"
            } else {
                device_status
            };
            let detail = if device_action.is_some() {
                format!("{status} · {signal} · {}", device.address)
            } else {
                format!("{signal} · {}", device.address)
            };
            serde_json::json!({
                "accent": accent,
                "address": device.address,
                "action_pending": action_pending,
                "audio": audio,
                "badge": badge,
                "badge_tone": accent,
                "can_connect": !device.connected,
                "can_disconnect": device.connected,
                "can_pair": !device.paired,
                "can_remove": device.paired,
                "can_trust": device.paired && !device.trusted,
                "can_untrust": device.paired && device.trusted,
                "connected": device.connected,
                "detail": detail,
                "managed": managed.contains(&device.address),
                "name": device.alias,
                "paired": device.paired,
                "rssi": device.rssi,
                "ready": ready,
                "services_resolved": device.services_resolved,
                "signal": signal,
                "status": status,
                "trusted": device.trusted,
                "uuids": device.uuids,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "connected_count": connected_count,
        "count": items.len(),
        "items": items,
        "paired_count": paired_count,
        "ready_count": ready_count,
        "searching": discovery.searching(),
    })
}

pub fn managed_device_options(devices: &[DeviceInfo]) -> Vec<DeviceOption> {
    let mut paired = devices
        .iter()
        .filter(|device| device.paired)
        .collect::<Vec<_>>();
    paired.sort_by(|left, right| {
        left.alias
            .to_lowercase()
            .cmp(&right.alias.to_lowercase())
            .then_with(|| left.address.cmp(&right.address))
    });
    paired
        .into_iter()
        .map(|device| DeviceOption {
            value: device.address.clone(),
            label: format!("{} · {}", device.alias, device.address),
        })
        .collect()
}

pub fn search_status_payload(discovery: &DiscoveryState) -> serde_json::Value {
    serde_json::json!({
        "discovered_count": discovery.discovered_count(),
        "searching": discovery.searching(),
        "state": if discovery.searching() { "searching" } else { "idle" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_normalization_accepts_only_six_hex_octets() {
        let cases = [
            ("aa:bb:cc:dd:ee:ff", Some("AA:BB:CC:DD:EE:FF")),
            (" 01:23:45:67:89:ab ", Some("01:23:45:67:89:AB")),
            ("AA-BB-CC-DD-EE-FF", None),
            ("AA:BB:CC:DD:EE", None),
            ("AA:BB:CC:DD:EE:GG", None),
            ("A:BB:CC:DD:EE:FF", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_address(input).ok().as_deref(),
                expected,
                "input={input:?}"
            );
        }
    }

    #[test]
    fn devices_payload_keeps_paired_and_discovered_devices_together() {
        let devices = [
            device("04", "Old Device", false, false, false, None),
            device("03", "Nearby Speaker", false, false, false, Some(-42)),
            device("02", "Luna 2", true, true, false, None),
            device("01", "Alpha", true, true, true, Some(-30)),
        ];
        let mut discovery = DiscoveryState::default();
        discovery.start();
        discovery.record("AA:BB:CC:DD:EE:03");
        let payload = devices_payload(
            &devices,
            &["AA:BB:CC:DD:EE:01".into(), "not-an-address".into()],
            &discovery,
            None,
        );
        assert_eq!(
            payload,
            serde_json::json!({
                "connected_count": 1,
                "count": 3,
                "items": [
                    {
                        "accent": "success",
                        "address": "AA:BB:CC:DD:EE:01",
                        "action_pending": false,
                        "audio": false,
                        "badge": "Connected",
                        "badge_tone": "success",
                        "can_connect": false,
                        "can_disconnect": true,
                        "can_pair": false,
                        "can_remove": true,
                        "can_trust": false,
                        "can_untrust": true,
                        "connected": true,
                        "detail": "-30 dBm · AA:BB:CC:DD:EE:01",
                        "managed": true,
                        "name": "Alpha",
                        "paired": true,
                        "ready": true,
                        "rssi": -30,
                        "services_resolved": true,
                        "signal": "-30 dBm",
                        "status": "Connected",
                        "trusted": true,
                        "uuids": [],
                    },
                    {
                        "accent": "accent",
                        "address": "AA:BB:CC:DD:EE:02",
                        "action_pending": false,
                        "audio": false,
                        "badge": "Paired",
                        "badge_tone": "accent",
                        "can_connect": true,
                        "can_disconnect": false,
                        "can_pair": false,
                        "can_remove": true,
                        "can_trust": false,
                        "can_untrust": true,
                        "connected": false,
                        "detail": "Signal unavailable · AA:BB:CC:DD:EE:02",
                        "managed": false,
                        "name": "Luna 2",
                        "paired": true,
                        "ready": false,
                        "rssi": null,
                        "services_resolved": false,
                        "signal": "Signal unavailable",
                        "status": "Paired",
                        "trusted": true,
                        "uuids": [],
                    },
                    {
                        "accent": "muted",
                        "address": "AA:BB:CC:DD:EE:03",
                        "action_pending": false,
                        "audio": false,
                        "badge": "Available",
                        "badge_tone": "muted",
                        "can_connect": true,
                        "can_disconnect": false,
                        "can_pair": true,
                        "can_remove": false,
                        "can_trust": false,
                        "can_untrust": false,
                        "connected": false,
                        "detail": "-42 dBm · AA:BB:CC:DD:EE:03",
                        "managed": false,
                        "name": "Nearby Speaker",
                        "paired": false,
                        "ready": false,
                        "rssi": -42,
                        "services_resolved": false,
                        "signal": "-42 dBm",
                        "status": "Available",
                        "trusted": false,
                        "uuids": [],
                    },
                ],
                "paired_count": 2,
                "ready_count": 1,
                "searching": true,
            })
        );
    }

    #[test]
    fn managed_device_options_include_only_paired_devices() {
        let devices = [
            device("03", "Nearby Speaker", false, false, false, Some(-42)),
            device("02", "Luna 2", true, true, false, None),
            device("01", "Alpha", true, true, true, Some(-30)),
        ];

        assert_eq!(
            managed_device_options(&devices),
            vec![
                DeviceOption {
                    value: "AA:BB:CC:DD:EE:01".into(),
                    label: "Alpha · AA:BB:CC:DD:EE:01".into(),
                },
                DeviceOption {
                    value: "AA:BB:CC:DD:EE:02".into(),
                    label: "Luna 2 · AA:BB:CC:DD:EE:02".into(),
                },
            ]
        );
    }

    #[test]
    fn devices_payload_keeps_a_pending_action_visible_with_its_stable_action_label() {
        let devices = [device("03", "Luna 2", false, false, false, Some(-42))];
        let mut discovery = DiscoveryState::default();
        discovery.record("AA:BB:CC:DD:EE:03");
        let action = DeviceActionState {
            address: "AA:BB:CC:DD:EE:03".into(),
            status: "Connecting...".into(),
            pending: true,
        };
        let payload = devices_payload(&devices, &[], &discovery, Some(&action));
        let item = &payload["items"][0];
        assert_eq!(item["status"], "Connecting...");
        assert_eq!(item["action_pending"], true);
        assert_eq!(item["can_connect"], true);
        assert_eq!(item["can_pair"], true);
        assert_eq!(item["can_remove"], false);
    }

    #[test]
    fn devices_payload_retains_a_discovered_snapshot_after_bluez_drops_the_object() {
        let luna = device("03", "Luna 2", false, false, false, Some(-42));
        let mut discovery = DiscoveryState::default();
        discovery.record_device(luna);
        let payload = devices_payload(&[], &[], &discovery, None);
        assert_eq!(payload["count"], 1);
        assert_eq!(payload["items"][0]["name"], "Luna 2");
        assert_eq!(payload["items"][0]["status"], "Available");
    }

    #[test]
    fn discovery_state_runs_until_stopped_and_retains_results() {
        let mut state = DiscoveryState::default();
        state.record("stale");
        state.start();
        assert!(state.searching());
        assert_eq!(state.discovered_count(), 0);

        state.record("AA:BB:CC:DD:EE:04");
        state.stop();
        assert_eq!(
            search_status_payload(&state),
            serde_json::json!({
                "discovered_count": 1,
                "searching": false,
                "state": "idle",
            })
        );

        state.remove("AA:BB:CC:DD:EE:04");
        assert_eq!(state.discovered_count(), 0);
    }

    #[test]
    fn audio_connection_requires_a_connected_a2dp_sink_profile() {
        let cases = [
            ("BLE-only speaker", audio_device(true, true, &[]), false),
            (
                "A2DP speaker",
                audio_device(true, true, &[AUDIO_SINK_UUID]),
                true,
            ),
            (
                "unresolved A2DP speaker",
                audio_device(true, false, &[AUDIO_SINK_UUID]),
                true,
            ),
            (
                "disconnected speaker",
                audio_device(false, true, &[AUDIO_SINK_UUID]),
                false,
            ),
            (
                "connected controller",
                device("05", "Controller", true, true, true, None),
                true,
            ),
        ];
        for (label, device, expected) in cases {
            assert_eq!(connection_ready(&device), expected, "case: {label}");
        }
    }

    #[test]
    fn connected_audio_device_never_exposes_connect_again() {
        let device = audio_device(true, false, &[AUDIO_SINK_UUID]);
        let payload = devices_payload(&[device], &[], &DiscoveryState::default(), None);
        let item = &payload["items"][0];
        assert_eq!(item["status"], "Connected");
        assert_eq!(item["can_connect"], false);
        assert_eq!(item["can_disconnect"], true);
        assert_eq!(item["ready"], true);
    }

    fn audio_device(connected: bool, services_resolved: bool, uuids: &[&str]) -> DeviceInfo {
        DeviceInfo {
            icon: Some("audio-card".into()),
            services_resolved,
            uuids: uuids.iter().map(|uuid| (*uuid).to_string()).collect(),
            ..device("06", "Speaker", true, true, connected, None)
        }
    }

    fn device(
        address_suffix: &str,
        alias: &str,
        paired: bool,
        trusted: bool,
        connected: bool,
        rssi: Option<i16>,
    ) -> DeviceInfo {
        DeviceInfo {
            address: format!("AA:BB:CC:DD:EE:{address_suffix}"),
            alias: alias.into(),
            paired,
            trusted,
            connected,
            services_resolved: connected,
            icon: None,
            class: None,
            uuids: Vec::new(),
            rssi,
        }
    }
}
