use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::{
    low_energy_identifier, without_settings_twins, BackendCapabilities, DeviceActionState,
    DeviceInfo, DeviceIntent, DiscoveryState, Pairing, SEARCH_WINDOW, SYSTEM_BLUETOOTH_SETTINGS,
};

const OTHER_DEVICES: &str = "other devices";
const UNKNOWN_KIND: &str = "Unknown kind";
const LOW_ENERGY: &str = "Bluetooth LE";
const HID_UUID: &str = "00001812-0000-1000-8000-00805f9b34fb";
const AUDIO_SINK_UUID: &str = "0000110b-0000-1000-8000-00805f9b34fb";
const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPhase {
    Off,
    Searching(Duration),
    Fresh,
    Ended(Duration),
}

impl DiscoveryState {
    pub fn phase(&self, powered: bool, now: Instant) -> SearchPhase {
        if !powered {
            return SearchPhase::Off;
        }
        if self.searching {
            let elapsed = self
                .started
                .map(|started| now.saturating_duration_since(started))
                .unwrap_or_default();
            return SearchPhase::Searching(SEARCH_WINDOW.saturating_sub(elapsed));
        }
        match self.ended {
            Some(ended) => SearchPhase::Ended(now.saturating_duration_since(ended)),
            None => SearchPhase::Fresh,
        }
    }
}

pub fn nearby_payload(
    devices: &[DeviceInfo],
    discovery: &DiscoveryState,
    action: Option<&DeviceActionState>,
    powered: bool,
    capabilities: BackendCapabilities,
    now: Instant,
) -> serde_json::Value {
    let phase = discovery.phase(powered, now);
    let items = match phase {
        SearchPhase::Off => Vec::new(),
        _ => nearby_devices(devices, discovery)
            .into_iter()
            .map(|(kind, device)| nearby_item(device, kind, action, capabilities))
            .collect(),
    };
    let count = items.len();
    let summary = match phase {
        _ if count == 0 => None,
        SearchPhase::Off => None,
        SearchPhase::Searching(_) => Some(format!("{count} so far")),
        SearchPhase::Fresh | SearchPhase::Ended(_) => Some(format!("{count} found")),
    };
    serde_json::json!({
        "count": count,
        "empty": empty_copy(phase),
        "items": items,
        "lead": lead(phase),
        "searching": discovery.searching(),
        "summary": summary,
    })
}

pub fn device_kind(device: &DeviceInfo) -> Option<&'static str> {
    device
        .icon
        .as_deref()
        .and_then(icon_kind)
        .or_else(|| device.class.and_then(class_kind))
        .or_else(|| uuid_kind(&device.uuids))
}

pub fn signal_bars(rssi: i16) -> u8 {
    match rssi {
        rssi if rssi > -50 => 4,
        rssi if rssi > -65 => 3,
        rssi if rssi > -80 => 2,
        _ => 1,
    }
}

fn nearby_devices<'a>(
    devices: &'a [DeviceInfo],
    discovery: &'a DiscoveryState,
) -> Vec<(Option<&'static str>, &'a DeviceInfo)> {
    let listed = devices
        .iter()
        .map(|device| device.address.as_str())
        .collect::<HashSet<_>>();
    let everyone = devices
        .iter()
        .chain(
            discovery
                .devices
                .values()
                .filter(|device| !listed.contains(device.address.as_str())),
        )
        .collect::<Vec<_>>();
    let mut nearby = without_settings_twins(everyone)
        .into_iter()
        .filter(|device| discovery.contains(&device.address))
        .filter(|device| !device.paired || discovery.claimed.contains(&device.address))
        .map(|device| (device_kind(device), device))
        .collect::<Vec<_>>();
    nearby.sort_by(|(left_kind, left), (right_kind, right)| {
        left_kind
            .is_none()
            .cmp(&right_kind.is_none())
            .then_with(|| right.rssi.cmp(&left.rssi))
            .then_with(|| left.alias.to_lowercase().cmp(&right.alias.to_lowercase()))
            .then_with(|| left.address.cmp(&right.address))
    });
    nearby
}

fn nearby_item(
    device: &DeviceInfo,
    kind: Option<&'static str>,
    action: Option<&DeviceActionState>,
    capabilities: BackendCapabilities,
) -> serde_json::Value {
    let action = action.filter(|action| action.address == device.address);
    let pending = action.filter(|action| action.pending);
    let failed = action.is_some_and(|action| {
        !action.pending && action.intent == DeviceIntent::Pair && !device.paired
    });
    let pairs_here = device.pairing == Pairing::Backend;
    let low_energy = device.pairing == Pairing::SystemSettings
        || low_energy_identifier(&device.address).is_some();
    let kind_line = match kind {
        Some(kind) if low_energy => format!("{kind} · {LOW_ENERGY}"),
        Some(kind) => kind.to_string(),
        None => UNKNOWN_KIND.to_string(),
    };
    let (badge, tone, detail) = if let Some(action) = pending {
        (action.status.clone(), "accent", kind_line)
    } else if failed {
        (
            "Couldn't pair".to_string(),
            "danger",
            format!("Put {} in pairing mode, then try again", device.alias),
        )
    } else if device.paired {
        (
            "Paired".to_string(),
            "success",
            "Now in bluetooth devices".to_string(),
        )
    } else if !pairs_here && capabilities.settings_pane.is_none() {
        let settings = capabilities
            .settings_pairing
            .unwrap_or(SYSTEM_BLUETOOTH_SETTINGS);
        (format!("Pair in {settings}"), "muted", kind_line)
    } else {
        (String::new(), "muted", kind_line)
    };
    let opens_settings = !pairs_here && !device.paired && capabilities.settings_pane.is_some();
    serde_json::json!({
        "action_pending": pending.is_some(),
        "address": device.address,
        "badge": badge,
        "badge_tone": tone,
        "can_open_settings": opens_settings,
        "can_pair": pairs_here && !device.paired && pending.is_none() && !failed,
        "can_retry": pairs_here && failed,
        "detail": detail,
        "group": if kind.is_some() { "" } else { OTHER_DEVICES },
        "kind": kind,
        "low_energy": low_energy,
        "name": device.alias,
        "paired": device.paired,
        "rssi": device.rssi,
        "signal": device.rssi.map(signal_bars),
    })
}

fn lead(phase: SearchPhase) -> serde_json::Value {
    let (state, label, detail) = match phase {
        SearchPhase::Off => (
            "off",
            "Bluetooth is off",
            "Turn it on to search".to_string(),
        ),
        SearchPhase::Searching(left) => (
            "active",
            "Searching nearby",
            format!("{} s left", (left.as_secs_f32().ceil() as u64).max(1)),
        ),
        SearchPhase::Fresh => ("rest", "Search nearby", "Not searched yet".to_string()),
        SearchPhase::Ended(ago) => ("rest", "Search finished", since(ago)),
    };
    let description = match phase {
        SearchPhase::Off => "needs bluetooth on.".to_string(),
        _ => format!(
            "finds devices for up to {} seconds.",
            SEARCH_WINDOW.as_secs()
        ),
    };
    serde_json::json!({
        "can_enable": phase == SearchPhase::Off,
        "can_search_again": matches!(phase, SearchPhase::Ended(_)),
        "can_start": phase == SearchPhase::Fresh,
        "can_stop": matches!(phase, SearchPhase::Searching(_)),
        "description": description,
        "detail": detail,
        "label": label,
        "state": state,
    })
}

fn empty_copy(phase: SearchPhase) -> serde_json::Value {
    let (title, detail) = match phase {
        SearchPhase::Off => ("Bluetooth is off", "Turn it on to find devices."),
        SearchPhase::Searching(_) => (
            "Looking for devices",
            "Put yours in pairing mode, it shows up here.",
        ),
        SearchPhase::Fresh => ("Nothing here yet", "Search to find devices nearby."),
        SearchPhase::Ended(_) => (
            "Nothing found",
            "Check the device is in pairing mode and close by, then search again.",
        ),
    };
    serde_json::json!({ "detail": detail, "title": title })
}

fn since(ago: Duration) -> String {
    match ago.as_secs() {
        seconds if seconds < MINUTE => "Just now".to_string(),
        seconds if seconds < HOUR => format!("{} min ago", seconds / MINUTE),
        seconds => format!("{} h ago", seconds / HOUR),
    }
}

fn icon_kind(icon: &str) -> Option<&'static str> {
    Some(match icon {
        "audio-headphones" => "Headphones",
        "audio-headset" => "Headset",
        "audio-card" => "Speaker",
        "input-keyboard" => "Keyboard",
        "input-mouse" => "Mouse",
        "input-gaming" => "Game controller",
        "input-tablet" => "Tablet",
        "phone" => "Phone",
        "computer" => "Computer",
        "camera-photo" | "camera-video" => "Camera",
        "printer" => "Printer",
        "video-display" => "Display",
        "multimedia-player" => "Media player",
        "modem" | "network-wireless" => "Network",
        icon if icon.starts_with("audio-") => "Audio",
        icon if icon.starts_with("input-") => "Input device",
        _ => return None,
    })
}

fn class_kind(class: u32) -> Option<&'static str> {
    let minor = (class >> 2) & 0x3f;
    Some(match (class >> 8) & 0x1f {
        0x01 => "Computer",
        0x02 => "Phone",
        0x03 => "Network",
        0x04 => match minor {
            0x01 | 0x02 => "Headset",
            0x05 => "Speaker",
            0x06 => "Headphones",
            _ => "Audio",
        },
        0x05 => match (minor >> 4, minor & 0x0f) {
            (_, 0x01 | 0x02) => "Game controller",
            (0x01, _) => "Keyboard",
            (0x02, _) => "Mouse",
            (0x03, _) => "Keyboard and mouse",
            _ => "Input device",
        },
        0x06 if class & 0x80 != 0 => "Printer",
        0x06 if class & 0x20 != 0 => "Camera",
        0x06 => "Display",
        0x07 => "Wearable",
        0x08 => "Toy",
        _ => return None,
    })
}

fn uuid_kind(uuids: &[String]) -> Option<&'static str> {
    let has = |wanted: &str| uuids.iter().any(|uuid| uuid.eq_ignore_ascii_case(wanted));
    if has(HID_UUID) {
        Some("Input device")
    } else if has(AUDIO_SINK_UUID) {
        Some("Audio")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{class_kind, device_kind, nearby_payload, signal_bars, SearchPhase, HID_UUID};
    use crate::bluetooth::{
        BackendCapabilities, DeviceActionState, DeviceInfo, DeviceIntent, DiscoveryState, Pairing,
    };

    const BLUEZ: BackendCapabilities = BackendCapabilities {
        separate_trust_flag: true,
        audio_reclaim: true,
        settings_pairing: None,
        settings_pane: None,
    };
    const MACOS: BackendCapabilities = BackendCapabilities {
        separate_trust_flag: false,
        audio_reclaim: false,
        settings_pairing: Some("System Settings"),
        settings_pane: Some("x-apple.systempreferences:com.apple.BluetoothSettings"),
    };
    const LIFT: &str = "31CAE991-2662-5D9B-D7B9-8CE2CD5FF04C";

    fn device(suffix: &str, alias: &str, rssi: Option<i16>, icon: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            address: format!("AA:BB:CC:DD:EE:{suffix}"),
            alias: alias.into(),
            paired: false,
            trusted: false,
            connected: false,
            audio_connected: None,
            services_resolved: false,
            icon: icon.map(str::to_string),
            class: None,
            uuids: Vec::new(),
            rssi,
            pairing: Pairing::Backend,
        }
    }

    fn searching_through(devices: &[DeviceInfo]) -> DiscoveryState {
        let mut discovery = DiscoveryState::default();
        discovery.start();
        for device in devices {
            discovery.record(device.address.clone());
        }
        discovery
    }

    fn later(seconds: u64) -> Instant {
        Instant::now() + Duration::from_secs(seconds)
    }

    fn names(payload: &serde_json::Value) -> Vec<(String, String)> {
        payload["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["name"].as_str().unwrap().to_string(),
                    item["group"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn the_search_lead_says_what_enter_does_in_every_phase() {
        let fresh = DiscoveryState::default();
        let searching = searching_through(&[]);
        let mut ended = searching_through(&[]);
        ended.stop();
        let cases = [
            (
                &fresh,
                false,
                0,
                "off",
                "Bluetooth is off",
                "Turn it on to search",
                "can_enable",
            ),
            (
                &fresh,
                true,
                0,
                "rest",
                "Search nearby",
                "Not searched yet",
                "can_start",
            ),
            (
                &searching,
                true,
                18,
                "active",
                "Searching nearby",
                "42 s left",
                "can_stop",
            ),
            (
                &ended,
                true,
                90,
                "rest",
                "Search finished",
                "1 min ago",
                "can_search_again",
            ),
            (
                &ended,
                true,
                5,
                "rest",
                "Search finished",
                "Just now",
                "can_search_again",
            ),
        ];
        for (discovery, powered, after, state, label, detail, verb) in cases {
            let payload = nearby_payload(&[], discovery, None, powered, BLUEZ, later(after));
            let lead = &payload["lead"];
            assert_eq!(
                (
                    lead["state"].as_str(),
                    lead["label"].as_str(),
                    lead["detail"].as_str()
                ),
                (Some(state), Some(label), Some(detail)),
                "{label}"
            );
            let verbs = ["can_enable", "can_start", "can_stop", "can_search_again"];
            for flag in verbs {
                assert_eq!(lead[flag], flag == verb, "{label}: {flag}");
            }
            let description = if powered {
                "finds devices for up to 60 seconds."
            } else {
                "needs bluetooth on."
            };
            assert_eq!(lead["description"], description, "{label}");
        }
    }

    #[test]
    fn each_phase_has_its_own_empty_copy_and_count_summary() {
        let phone = device("01", "Galaxy S21 5G", Some(-61), Some("phone"));
        let mut ended = searching_through(std::slice::from_ref(&phone));
        ended.stop();
        let cases = [
            (
                searching_through(&[]),
                true,
                &[][..],
                "Looking for devices",
                None,
            ),
            (
                DiscoveryState::default(),
                true,
                &[][..],
                "Nothing here yet",
                None,
            ),
            (
                DiscoveryState::default(),
                false,
                &[][..],
                "Bluetooth is off",
                None,
            ),
            (
                searching_through(std::slice::from_ref(&phone)),
                true,
                std::slice::from_ref(&phone),
                "Looking for devices",
                Some("1 so far"),
            ),
            (
                ended,
                true,
                std::slice::from_ref(&phone),
                "Nothing found",
                Some("1 found"),
            ),
        ];
        for (discovery, powered, devices, title, summary) in cases {
            let payload = nearby_payload(devices, &discovery, None, powered, BLUEZ, later(1));
            assert_eq!(payload["empty"]["title"], title);
            assert_eq!(payload["summary"].as_str(), summary, "{title}");
        }
    }

    #[test]
    fn known_kinds_lead_by_signal_and_the_rest_follow_as_other_devices() {
        let devices = [
            device("01", "FMM001_5998242", Some(-83), None),
            device("02", "Galaxy S21 5G", Some(-61), Some("phone")),
            device("03", "EVB-P22081807", Some(-78), None),
            device("04", "Quiet Mouse", None, Some("input-mouse")),
            device("05", "Speaker", Some(-44), Some("audio-card")),
        ];
        let payload = nearby_payload(
            &devices,
            &searching_through(&devices),
            None,
            true,
            BLUEZ,
            later(1),
        );
        let other = "other devices".to_string();
        assert_eq!(
            names(&payload),
            [
                ("Speaker".to_string(), String::new()),
                ("Galaxy S21 5G".to_string(), String::new()),
                ("Quiet Mouse".to_string(), String::new()),
                ("EVB-P22081807".to_string(), other.clone()),
                ("FMM001_5998242".to_string(), other),
            ]
        );
        let items = payload["items"].as_array().unwrap();
        assert_eq!(items[0]["signal"], 4);
        assert_eq!(items[0]["detail"], "Speaker");
        assert_eq!(items[2]["signal"], serde_json::Value::Null);
        assert_eq!(items[3]["detail"], "Unknown kind");
    }

    #[test]
    fn a_device_shows_pairing_failure_and_success_where_it_was_found() {
        let lift = device("01", "LIFT", Some(-44), Some("input-mouse"));
        let mut paired = lift.clone();
        paired.paired = true;
        let mut discovery = searching_through(std::slice::from_ref(&lift));
        let action = |pending, intent| DeviceActionState {
            address: lift.address.clone(),
            intent,
            status: if pending {
                "Pairing"
            } else {
                "the device refused"
            }
            .into(),
            pending,
        };
        let pairing = action(true, DeviceIntent::Pair);
        let refused = action(false, DeviceIntent::Pair);
        let cases = [
            (&lift, None, "", "Mouse", true, false),
            (&lift, Some(&pairing), "Pairing", "Mouse", false, false),
            (
                &lift,
                Some(&refused),
                "Couldn't pair",
                "Put LIFT in pairing mode, then try again",
                false,
                true,
            ),
        ];
        for (current, action, badge, detail, can_pair, can_retry) in cases {
            let payload = nearby_payload(
                std::slice::from_ref(current),
                &discovery,
                action,
                true,
                BLUEZ,
                later(1),
            );
            let item = &payload["items"][0];
            assert_eq!(item["badge"], badge, "{badge}");
            assert_eq!(item["detail"], detail, "{badge}");
            assert_eq!(item["can_pair"], can_pair, "{badge}");
            assert_eq!(item["can_retry"], can_retry, "{badge}");
            assert_eq!(item["action_pending"], badge == "Pairing", "{badge}");
        }
        let before = nearby_payload(
            std::slice::from_ref(&paired),
            &discovery,
            None,
            true,
            BLUEZ,
            later(1),
        );
        assert_eq!(
            before["count"], 0,
            "a device paired before the search stays out"
        );
        discovery.claim(&lift.address);
        let after = nearby_payload(
            std::slice::from_ref(&paired),
            &discovery,
            None,
            true,
            BLUEZ,
            later(1),
        );
        let item = &after["items"][0];
        assert_eq!(item["badge"], "Paired");
        assert_eq!(item["badge_tone"], "success");
        assert_eq!(item["detail"], "Now in bluetooth devices");
        assert_eq!(item["paired"], true);
    }

    #[test]
    fn a_settings_paired_device_points_at_system_settings() {
        let lift = DeviceInfo {
            address: LIFT.into(),
            pairing: Pairing::SystemSettings,
            uuids: vec![HID_UUID.into()],
            ..device("00", "LIFT", Some(-58), None)
        };
        let mut discovery = DiscoveryState::default();
        discovery.start();
        discovery.record_device(lift);
        let no_pane = BackendCapabilities {
            settings_pane: None,
            ..MACOS
        };
        let cases = [
            (MACOS, "", true),
            (no_pane, "Pair in System Settings", false),
        ];
        for (capabilities, badge, opens_settings) in cases {
            let payload = nearby_payload(&[], &discovery, None, true, capabilities, later(1));
            let item = &payload["items"][0];
            assert_eq!(item["badge"], badge);
            assert_eq!(item["can_open_settings"], opens_settings);
            assert_eq!(item["detail"], "Input device · Bluetooth LE");
            assert_eq!(item["can_pair"], false);
            assert_eq!(item["low_energy"], true);
            assert_eq!(item["signal"], 3);
        }
    }

    #[test]
    fn a_settings_twin_of_a_listed_device_is_shown_once() {
        let listed = device("01", "MX KEYS B", Some(-52), Some("input-keyboard"));
        let twin = DeviceInfo {
            address: LIFT.into(),
            pairing: Pairing::SystemSettings,
            ..device("00", " mx keys b ", Some(-60), None)
        };
        let mut discovery = searching_through(std::slice::from_ref(&listed));
        discovery.record_device(twin);
        let payload = nearby_payload(
            std::slice::from_ref(&listed),
            &discovery,
            None,
            true,
            MACOS,
            later(1),
        );
        assert_eq!(payload["count"], 1);
        assert_eq!(payload["items"][0]["address"], listed.address);
    }

    #[test]
    fn a_found_device_stays_after_bluez_drops_its_object() {
        let mut discovery = DiscoveryState::default();
        discovery.start();
        discovery.record_device(device("03", "Luna 2", Some(-42), Some("audio-headphones")));
        let payload = nearby_payload(&[], &discovery, None, true, BLUEZ, later(1));
        assert_eq!(payload["count"], 1);
        assert_eq!(payload["items"][0]["detail"], "Headphones");
    }

    #[test]
    fn turning_bluetooth_off_empties_the_found_list() {
        let phone = device("01", "Phone", Some(-61), Some("phone"));
        let discovery = searching_through(std::slice::from_ref(&phone));
        let payload = nearby_payload(
            std::slice::from_ref(&phone),
            &discovery,
            None,
            false,
            BLUEZ,
            later(1),
        );
        assert_eq!(payload["count"], 0);
        assert_eq!(payload["summary"], serde_json::Value::Null);
        assert_eq!(payload["lead"]["state"], "off");
    }

    #[test]
    fn device_kinds_come_from_the_icon_then_the_class_then_the_services() {
        let kinded = |icon: Option<&str>, class: Option<u32>, uuids: &[&str]| DeviceInfo {
            class,
            uuids: uuids.iter().map(|uuid| (*uuid).to_string()).collect(),
            ..device("01", "x", None, icon)
        };
        let cases = [
            (
                kinded(Some("input-keyboard"), Some(0x0200), &[]),
                Some("Keyboard"),
            ),
            (
                kinded(Some("audio-headphones"), None, &[]),
                Some("Headphones"),
            ),
            (kinded(Some("audio-speakers"), None, &[]), Some("Audio")),
            (
                kinded(Some("unknown-thing"), Some(0x5a020c), &[]),
                Some("Phone"),
            ),
            (kinded(None, Some(0x240404), &[]), Some("Headset")),
            (kinded(None, Some(0x002540), &[]), Some("Keyboard")),
            (kinded(None, Some(0x002580), &[]), Some("Mouse")),
            (kinded(None, Some(0x002508), &[]), Some("Game controller")),
            (
                kinded(None, None, &[&HID_UUID.to_uppercase()]),
                Some("Input device"),
            ),
            (kinded(None, Some(0x001f00), &[]), None),
            (kinded(None, None, &[]), None),
        ];
        for (device, expected) in cases {
            assert_eq!(device_kind(&device), expected, "{device:?}");
        }
        assert_eq!(class_kind(0x000680), Some("Printer"));
    }

    #[test]
    fn signal_bars_follow_the_rssi_thresholds() {
        let cases = [
            (-30, 4),
            (-49, 4),
            (-50, 3),
            (-64, 3),
            (-65, 2),
            (-79, 2),
            (-80, 1),
            (-100, 1),
        ];
        for (rssi, bars) in cases {
            assert_eq!(signal_bars(rssi), bars, "rssi={rssi}");
        }
    }

    #[test]
    fn a_search_counts_down_from_its_window() {
        let discovery = searching_through(&[]);
        assert_eq!(
            discovery.phase(true, later(70)),
            SearchPhase::Searching(Duration::ZERO)
        );
        let payload = nearby_payload(&[], &discovery, None, true, BLUEZ, later(70));
        assert_eq!(payload["lead"]["detail"], "1 s left");
    }
}
