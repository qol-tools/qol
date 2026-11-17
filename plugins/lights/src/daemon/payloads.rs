use serde_json::{json, Value};

use super::input::display_hex;
use super::pairing::{PairingPhase, PAIRING_WINDOW};
use crate::config::model::{LightLook, PresetConfig, PRESET_COUNT};

const PAIRED: &str = "Paired";
const NOW_IN_LIGHTS: &str = "Now in lights";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightRow {
    pub ieee: String,
    pub name: String,
    pub network_address: u16,
    pub look: LightLook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DongleStatus {
    Ready { port: String, channel: u8 },
    Missing(String),
    Offline(String),
}

impl DongleStatus {
    pub fn status(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "Connected",
            Self::Missing(_) => "Not found",
            Self::Offline(_) => "Offline",
        }
    }

    pub fn ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

pub fn lights_payload(lights: &[LightRow], main_target_id: &str, has_presets: bool) -> Value {
    let sole = lights.len() == 1;
    let items = sorted(lights)
        .into_iter()
        .map(|light| {
            json!({
                "brightness": light.look.brightness,
                "color": display_hex(&light.look.color_hex),
                "detail": look_detail(light.look.on, light.look.brightness),
                "has_presets": has_presets,
                "id": light.ieee,
                "ieee": light.ieee,
                "main": is_main(light, main_target_id, sole),
                "mirek": light.look.mirek,
                "name": light.name,
                "on": light.look.on,
            })
        })
        .collect::<Vec<_>>();
    json!({ "count": items.len(), "items": items })
}

pub fn presets_payload(presets: &PresetConfig) -> Value {
    let items = (1..=PRESET_COUNT)
        .filter_map(|number| presets.slot(number).map(|slot| (number, slot)))
        .filter(|(_, slot)| slot.enabled)
        .map(|(number, slot)| {
            json!({
                "brightness": slot.brightness,
                "color": display_hex(&slot.color_hex),
                "detail": look_detail(slot.power_on, slot.brightness),
                "id": format!("preset_{number}"),
                "mirek": slot.mirek,
                "name": slot.name,
                "power_on": slot.power_on,
            })
        })
        .collect::<Vec<_>>();
    json!({ "count": items.len(), "items": items })
}

fn look_detail(on: bool, brightness: u8) -> String {
    if on {
        return format!("On · {brightness}%");
    }
    "Off".to_string()
}

pub fn find_lights_payload(phase: PairingPhase, joined: &[LightRow]) -> Value {
    let items = match phase {
        PairingPhase::NoDongle => Vec::new(),
        _ => joined
            .iter()
            .map(|light| {
                json!({
                    "badge": PAIRED,
                    "badge_tone": "success",
                    "detail": NOW_IN_LIGHTS,
                    "id": light.ieee,
                    "ieee": light.ieee,
                    "name": light.name,
                    "paired": true,
                })
            })
            .collect(),
    };
    let count = items.len();
    let summary = match phase {
        _ if count == 0 => None,
        PairingPhase::NoDongle => None,
        PairingPhase::Searching(_) => Some(format!("{count} so far")),
        PairingPhase::Fresh | PairingPhase::Ended => Some(format!("{count} paired")),
    };
    json!({
        "count": count,
        "empty": find_empty(phase),
        "items": items,
        "lead": find_lead(phase),
        "searching": matches!(phase, PairingPhase::Searching(_)),
        "summary": summary,
    })
}

pub fn dongle_payload(status: &DongleStatus) -> Value {
    let (state, label, detail, title, empty_detail) = match status {
        DongleStatus::Ready { port, channel } => (
            "rest",
            "Connected".to_string(),
            format!("{port} · Zigbee channel {channel}"),
            "Dongle ready",
            "Lights pair and take commands through it.",
        ),
        DongleStatus::Missing(message) => (
            "off",
            "No dongle found".to_string(),
            message.clone(),
            "No dongle",
            "Plug in your light dongle, then find it.",
        ),
        DongleStatus::Offline(message) => (
            "off",
            "Dongle offline".to_string(),
            message.clone(),
            "Dongle offline",
            "Check the dongle is plugged in, then find it again.",
        ),
    };
    json!({
        "count": 0,
        "empty": { "detail": empty_detail, "title": title },
        "items": [],
        "lead": {
            "can_find": !status.ready(),
            "detail": detail,
            "label": label,
            "state": state,
        },
        "status": status.status(),
    })
}

fn find_lead(phase: PairingPhase) -> Value {
    let (state, label, detail) = match phase {
        PairingPhase::NoDongle => (
            "off",
            "No dongle",
            "Plug in your light dongle to search".to_string(),
        ),
        PairingPhase::Searching(left) => (
            "active",
            "Searching for lights",
            format!("{} s left", (left.as_secs_f32().ceil() as u64).max(1)),
        ),
        PairingPhase::Fresh => ("rest", "Search for lights", "Not searched yet".to_string()),
        PairingPhase::Ended => (
            "rest",
            "Search finished",
            "Search again to pair more".to_string(),
        ),
    };
    let description = match phase {
        PairingPhase::NoDongle => "needs the light dongle.".to_string(),
        _ => format!(
            "lets new lights join for up to {} seconds.",
            PAIRING_WINDOW.as_secs()
        ),
    };
    json!({
        "can_find": phase == PairingPhase::NoDongle,
        "can_search_again": phase == PairingPhase::Ended,
        "can_start": phase == PairingPhase::Fresh,
        "can_stop": matches!(phase, PairingPhase::Searching(_)),
        "description": description,
        "detail": detail,
        "label": label,
        "state": state,
    })
}

fn find_empty(phase: PairingPhase) -> Value {
    let (title, detail) = match phase {
        PairingPhase::NoDongle => ("No dongle", "Plug in your light dongle to find lights."),
        PairingPhase::Searching(_) => (
            "Looking for lights",
            "Reset your light to pairing mode, it shows up here.",
        ),
        PairingPhase::Fresh => ("Nothing here yet", "Search to pair a new light."),
        PairingPhase::Ended => (
            "Nothing joined",
            "Check the light is in pairing mode and close to the dongle, then search again.",
        ),
    };
    json!({ "detail": detail, "title": title })
}

fn sorted(lights: &[LightRow]) -> Vec<&LightRow> {
    let mut sorted = lights.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.ieee.cmp(&right.ieee))
    });
    sorted
}

fn is_main(light: &LightRow, main_target_id: &str, sole: bool) -> bool {
    if main_target_id.is_empty() {
        return sole;
    }
    main_target_id.eq_ignore_ascii_case(&light.ieee)
        || main_target_id.eq_ignore_ascii_case(&network_label(light.network_address))
}

fn network_label(network_address: u16) -> String {
    format!("0x{network_address:04X}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn light(ieee: &str, name: &str, network_address: u16) -> LightRow {
        LightRow {
            ieee: ieee.into(),
            name: name.into(),
            network_address,
            look: LightLook {
                on: true,
                color_hex: "#ffffff".into(),
                brightness: 100,
                mirek: 300,
            },
        }
    }

    #[test]
    fn lights_sort_by_name_and_flag_the_main_light() {
        let lights = [
            light("00:00:00:00:00:00:00:02", "Shelf", 0x1A2B),
            light("00:00:00:00:00:00:00:01", "desk", 0x0001),
        ];
        let cases = [
            ("", [false, false]),
            ("00:00:00:00:00:00:00:02", [false, true]),
            ("0x0001", [true, false]),
            ("0x1a2b", [false, true]),
            ("missing", [false, false]),
        ];
        for (main_target_id, expected) in cases {
            let payload = lights_payload(&lights, main_target_id, false);
            let items = payload["items"].as_array().unwrap();
            assert_eq!(payload["count"], 2);
            assert_eq!(items[0]["name"], "desk");
            for (item, main) in items.iter().zip(expected) {
                assert_eq!(item["main"], main, "target {main_target_id}");
            }
        }
    }

    #[test]
    fn a_light_row_carries_its_look_and_no_badge() {
        let mut shelf = light("00:00:00:00:00:00:00:02", "Shelf", 0x1A2B);
        shelf.look = LightLook {
            on: false,
            color_hex: "ffaa00".into(),
            brightness: 35,
            mirek: 410,
        };
        let desk = light("00:00:00:00:00:00:00:01", "Desk", 1);

        let payload = lights_payload(&[desk, shelf], "", true);

        assert_eq!(
            payload["items"][0],
            json!({
                "brightness": 100,
                "color": "#FFFFFF",
                "detail": "On · 100%",
                "has_presets": true,
                "id": "00:00:00:00:00:00:00:01",
                "ieee": "00:00:00:00:00:00:00:01",
                "main": false,
                "mirek": 300,
                "name": "Desk",
                "on": true,
            })
        );
        let item = &payload["items"][1];
        assert_eq!(item["on"], false);
        assert_eq!(item["detail"], "Off");
        assert_eq!(item["color"], "#FFAA00");
        assert_eq!(item["brightness"], 35);
        assert_eq!(item["mirek"], 410);
        for dropped in ["accent", "badge", "badge_tone"] {
            assert!(item.get(dropped).is_none(), "{dropped}");
        }
    }

    #[test]
    fn every_light_row_says_whether_any_preset_exists() {
        for has_presets in [true, false] {
            let payload = lights_payload(&[light("AA", "Desk", 1)], "", has_presets);
            assert_eq!(payload["items"][0]["has_presets"], has_presets);
        }
    }

    #[test]
    fn a_sole_light_is_the_main_light_when_none_is_chosen() {
        let payload = lights_payload(&[light("AA", "Desk", 1)], "", false);
        assert_eq!(payload["items"][0]["main"], true);
    }

    #[test]
    fn presets_list_only_enabled_slots_in_slot_order() {
        let mut presets = PresetConfig::default();
        for number in [5, 2] {
            let slot = presets.slot_mut(number).unwrap();
            slot.enabled = true;
            slot.color_hex = "#00ff00".into();
        }
        let slot = presets.slot_mut(5).unwrap();
        slot.power_on = false;
        slot.name = "Night".into();
        presets.slot_mut(2).unwrap().brightness = 40;

        let payload = presets_payload(&presets);

        assert_eq!(payload["count"], 2);
        assert!(payload.get("lead").is_none());
        assert_eq!(
            payload["items"],
            json!([
                {
                    "brightness": 40,
                    "color": "#00FF00",
                    "detail": "On · 40%",
                    "id": "preset_2",
                    "mirek": 300,
                    "name": "Preset 2",
                    "power_on": true,
                },
                {
                    "brightness": 100,
                    "color": "#00FF00",
                    "detail": "Off",
                    "id": "preset_5",
                    "mirek": 300,
                    "name": "Night",
                    "power_on": false,
                },
            ])
        );
    }

    #[test]
    fn no_enabled_preset_is_an_empty_list() {
        let payload = presets_payload(&PresetConfig::default());
        assert_eq!(payload, json!({ "count": 0, "items": [] }));
    }

    #[test]
    fn find_lead_offers_one_verb_per_phase() {
        let cases = [
            (PairingPhase::NoDongle, "off", "can_find"),
            (
                PairingPhase::Searching(Duration::from_millis(14_200)),
                "active",
                "can_stop",
            ),
            (PairingPhase::Fresh, "rest", "can_start"),
            (PairingPhase::Ended, "rest", "can_search_again"),
        ];
        let verbs = ["can_find", "can_stop", "can_start", "can_search_again"];
        for (phase, state, verb) in cases {
            let lead = &find_lights_payload(phase, &[])["lead"];
            assert_eq!(lead["state"], state, "{phase:?}");
            for candidate in verbs {
                assert_eq!(lead[candidate], candidate == verb, "{phase:?} {candidate}");
            }
        }
        let searching =
            find_lights_payload(PairingPhase::Searching(Duration::from_millis(14_200)), &[]);
        assert_eq!(searching["lead"]["detail"], "15 s left");
    }

    #[test]
    fn joined_lights_show_as_paired_until_the_dongle_goes() {
        let joined = [light("AA", "Device 0001", 1)];
        let cases = [
            (PairingPhase::Searching(PAIRING_WINDOW), 1, Some("1 so far")),
            (PairingPhase::Ended, 1, Some("1 paired")),
            (PairingPhase::NoDongle, 0, None),
        ];
        for (phase, count, summary) in cases {
            let payload = find_lights_payload(phase, &joined);
            assert_eq!(payload["count"], count, "{phase:?}");
            assert_eq!(payload["summary"], json!(summary), "{phase:?}");
        }
        let item = &find_lights_payload(PairingPhase::Ended, &joined)["items"][0];
        assert_eq!(item["paired"], true);
        assert_eq!(item["badge"], PAIRED);
        assert_eq!(item["detail"], NOW_IN_LIGHTS);
    }

    #[test]
    fn dongle_lead_offers_find_only_while_the_dongle_is_missing() {
        let cases = [
            (
                DongleStatus::Ready {
                    port: "/dev/ttyUSB0".into(),
                    channel: 11,
                },
                "Connected",
                "/dev/ttyUSB0 · Zigbee channel 11",
                false,
            ),
            (
                DongleStatus::Missing("No light dongle found.".into()),
                "No dongle found",
                "No light dongle found.",
                true,
            ),
            (
                DongleStatus::Offline("permission denied".into()),
                "Dongle offline",
                "permission denied",
                true,
            ),
        ];
        for (status, label, detail, can_find) in cases {
            let payload = dongle_payload(&status);
            assert_eq!(
                payload["status"],
                match label {
                    "Connected" => "Connected",
                    "No dongle found" => "Not found",
                    _ => "Offline",
                }
            );
            assert_eq!(payload["lead"]["label"], label);
            assert_eq!(payload["lead"]["detail"], detail);
            assert_eq!(payload["lead"]["can_find"], can_find);
            assert_eq!(payload["items"], json!([]));
        }
    }
}
