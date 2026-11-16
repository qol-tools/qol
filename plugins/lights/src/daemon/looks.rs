use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::format_ieee;
use super::input::{hex_string, BRIGHTNESS_RANGE, MIREK_RANGE};
use super::persist::{self, Change};
use crate::config::model::{DeviceEntry, LightLook, PluginConfig};
use crate::domain::model::{LightCommand, LightTarget};
use crate::znp::Device;

pub type SharedLooks = Arc<Mutex<LightLooks>>;
pub type LightAddress = (String, Option<u16>);

#[derive(Debug, Clone)]
pub struct LightLooks {
    looks: HashMap<String, LightLook>,
    fallback: LightLook,
}

impl Default for LightLooks {
    fn default() -> Self {
        Self::new(&PluginConfig::default())
    }
}

impl LightLooks {
    pub fn new(config: &PluginConfig) -> Self {
        let mut looks = Self {
            looks: HashMap::new(),
            fallback: LightLook::from_live(config),
        };
        looks.adopt(config);
        looks
    }

    pub fn adopt(&mut self, config: &PluginConfig) {
        let mut remembered = std::mem::take(&mut self.looks);
        for entry in config.devices.values() {
            let key = normalize(&entry.ieee_address);
            let look = remembered.remove(&key).or_else(|| entry.look.clone());
            if let Some(look) = look {
                self.looks.insert(key, look);
            }
        }
        self.set_fallback(config);
    }

    pub fn set_fallback(&mut self, config: &PluginConfig) {
        self.fallback = LightLook::from_live(config);
    }

    pub fn look(&self, ieee: &str) -> LightLook {
        self.looks
            .get(&normalize(ieee))
            .cloned()
            .unwrap_or_else(|| self.fallback.clone())
    }

    pub fn apply(&mut self, ieee: &str, commands: &[LightCommand]) {
        let mut look = self.look(ieee);
        for command in commands {
            apply_command(&mut look, command);
        }
        self.looks.insert(normalize(ieee), look);
    }

    pub fn forget(&mut self, ieee: &str) {
        self.looks.remove(&normalize(ieee));
    }

    pub fn snapshot(&self) -> HashMap<String, LightLook> {
        self.looks.clone()
    }
}

pub fn lock(looks: &SharedLooks) -> MutexGuard<'_, LightLooks> {
    looks.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn record(looks: &SharedLooks, ieee: &str, commands: &[LightCommand]) {
    if commands.is_empty() {
        return;
    }
    let mut guard = lock(looks);
    guard.apply(ieee, commands);
    persist::submit(Change::Looks(guard.snapshot()));
}

pub fn apply_command(look: &mut LightLook, command: &LightCommand) {
    match command {
        LightCommand::TurnOn => look.on = true,
        LightCommand::TurnOff => look.on = false,
        LightCommand::Toggle => look.on = !look.on,
        LightCommand::SetBrightness { level } => {
            look.brightness = clamp(u16::from(*level), BRIGHTNESS_RANGE) as u8
        }
        LightCommand::SetColor { color } => {
            look.color_hex = hex_string(color.red, color.green, color.blue)
        }
        LightCommand::SetColorTemperature { mirek } => look.mirek = clamp(*mirek, MIREK_RANGE),
    }
}

pub fn write_into(devices: &mut HashMap<String, DeviceEntry>, looks: &HashMap<String, LightLook>) {
    for entry in devices.values_mut() {
        if let Some(look) = looks.get(&normalize(&entry.ieee_address)) {
            entry.look = Some(look.clone());
        }
    }
}

pub fn light_addresses(devices: &[Device]) -> Vec<LightAddress> {
    devices
        .iter()
        .map(|device| {
            (
                format_ieee(&device.ieee_address),
                Some(device.network_address),
            )
        })
        .collect()
}

pub fn config_addresses(devices: &HashMap<String, DeviceEntry>) -> Vec<LightAddress> {
    let mut addresses = devices
        .values()
        .map(|entry| (normalize(&entry.ieee_address), entry.network_address))
        .collect::<Vec<_>>();
    addresses.sort();
    addresses
}

pub fn ieee_of_target(target: &LightTarget, lights: &[LightAddress]) -> Option<String> {
    let LightTarget::Device { id } = target else {
        return None;
    };
    let id = id.0.trim();
    if id.is_empty() {
        return match lights {
            [(ieee, _)] => Some(ieee.clone()),
            _ => None,
        };
    }
    lights
        .iter()
        .find(|(ieee, network)| names_light(id, ieee, *network))
        .map(|(ieee, _)| ieee.clone())
}

pub fn look_for(
    ieee: Option<&str>,
    main_target: &LightTarget,
    lights: &[LightAddress],
    looks: &LightLooks,
) -> Result<LightLook, String> {
    let ieee = match ieee {
        Some(ieee) => {
            let ieee = normalize(ieee);
            if !lights.iter().any(|(known, _)| *known == ieee) {
                return Err("That light is not paired.".into());
            }
            ieee
        }
        None => match ieee_of_target(main_target, lights) {
            Some(ieee) => ieee,
            None => return Ok(looks.fallback.clone()),
        },
    };
    Ok(looks.look(&ieee))
}

pub fn names_light(id: &str, ieee: &str, network_address: Option<u16>) -> bool {
    let id = id.trim();
    id.eq_ignore_ascii_case(ieee)
        || network_address.is_some_and(|nwk| id.eq_ignore_ascii_case(&format!("0x{nwk:04X}")))
}

pub fn normalize(ieee: &str) -> String {
    ieee.trim().to_ascii_uppercase()
}

fn clamp(value: u16, range: (u16, u16)) -> u16 {
    value.clamp(range.0, range.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{DeviceId, RgbColor};

    const DESK: &str = "00:11:22:33:44:55:66:77";
    const SHELF: &str = "00:11:22:33:44:55:66:88";

    fn entry(ieee: &str, look: Option<LightLook>) -> DeviceEntry {
        DeviceEntry {
            ieee_address: ieee.into(),
            name: "Light".into(),
            endpoints: Vec::new(),
            online: true,
            network_address: Some(0x1A2B),
            look,
        }
    }

    fn config_with(devices: Vec<DeviceEntry>) -> PluginConfig {
        let mut config = PluginConfig {
            live_color_hex: "#112233".into(),
            live_brightness: 40,
            live_mirek: 200,
            ..PluginConfig::default()
        };
        for device in devices {
            config.devices.insert(device.ieee_address.clone(), device);
        }
        config
    }

    fn dim() -> LightLook {
        LightLook {
            on: false,
            color_hex: "#FF0000".into(),
            brightness: 5,
            mirek: 400,
        }
    }

    fn device_target(id: &str) -> LightTarget {
        LightTarget::Device {
            id: DeviceId(id.into()),
        }
    }

    #[test]
    fn a_light_with_no_stored_look_starts_on_with_the_live_values() {
        let looks = LightLooks::new(&config_with(vec![entry(DESK, None)]));

        assert_eq!(
            looks.look(DESK),
            LightLook {
                on: true,
                color_hex: "#112233".into(),
                brightness: 40,
                mirek: 200,
            }
        );
    }

    #[test]
    fn a_stored_look_wins_over_the_live_values() {
        let looks = LightLooks::new(&config_with(vec![
            entry(DESK, Some(dim())),
            entry(SHELF, None),
        ]));

        assert_eq!(looks.look(DESK), dim());
        assert_eq!(looks.look(&DESK.to_lowercase()), dim());
        assert!(looks.look(SHELF).on);
    }

    #[test]
    fn commands_change_the_look_they_name() {
        let start = LightLook {
            on: true,
            color_hex: "#112233".into(),
            brightness: 40,
            mirek: 200,
        };
        let with = |change: fn(&mut LightLook)| {
            let mut look = start.clone();
            change(&mut look);
            look
        };
        let cases = [
            (LightCommand::TurnOff, with(|look| look.on = false)),
            (LightCommand::TurnOn, start.clone()),
            (LightCommand::Toggle, with(|look| look.on = false)),
            (
                LightCommand::SetBrightness { level: 0 },
                with(|look| look.brightness = 1),
            ),
            (
                LightCommand::SetBrightness { level: 70 },
                with(|look| look.brightness = 70),
            ),
            (
                LightCommand::SetColor {
                    color: RgbColor {
                        red: 255,
                        green: 170,
                        blue: 0,
                    },
                },
                with(|look| look.color_hex = "#FFAA00".into()),
            ),
            (
                LightCommand::SetColorTemperature { mirek: 9000 },
                with(|look| look.mirek = 500),
            ),
            (
                LightCommand::SetColorTemperature { mirek: 100 },
                with(|look| look.mirek = 153),
            ),
        ];
        for (command, expected) in cases {
            let mut looks = LightLooks::new(&config_with(vec![entry(DESK, None)]));
            looks.apply(DESK, std::slice::from_ref(&command));
            assert_eq!(looks.look(DESK), expected, "{command:?}");
        }
    }

    #[test]
    fn toggling_twice_returns_to_the_start() {
        let mut looks = LightLooks::new(&config_with(vec![entry(DESK, None)]));
        looks.apply(DESK, &[LightCommand::Toggle, LightCommand::Toggle]);
        assert!(looks.look(DESK).on);
    }

    #[test]
    fn a_reload_keeps_the_looks_of_lights_it_already_knows() {
        let mut looks = LightLooks::new(&config_with(vec![entry(DESK, None)]));
        looks.apply(DESK, &[LightCommand::TurnOff]);

        let mut reloaded = config_with(vec![entry(DESK, Some(dim())), entry(SHELF, Some(dim()))]);
        reloaded.live_brightness = 90;
        looks.adopt(&reloaded);

        assert!(!looks.look(DESK).on);
        assert_eq!(looks.look(DESK).brightness, 40);
        assert_eq!(looks.look(SHELF), dim());
        assert_eq!(looks.look("00:00:00:00:00:00:00:01").brightness, 90);
    }

    #[test]
    fn a_reload_drops_looks_of_lights_that_left_the_config() {
        let mut looks = LightLooks::new(&config_with(vec![entry(DESK, None)]));
        looks.apply(DESK, &[LightCommand::TurnOff]);

        looks.adopt(&config_with(vec![]));

        assert!(looks.snapshot().is_empty());
    }

    #[test]
    fn forgetting_a_light_returns_it_to_the_live_values() {
        let mut looks = LightLooks::new(&config_with(vec![entry(DESK, Some(dim()))]));
        looks.forget(DESK);
        assert!(looks.look(DESK).on);
    }

    #[test]
    fn looks_are_written_only_onto_the_devices_that_have_one() {
        let mut devices = config_with(vec![entry(DESK, None), entry(SHELF, None)]).devices;
        let mut looks = LightLooks::default();
        looks.apply(DESK, &[LightCommand::TurnOff]);

        write_into(&mut devices, &looks.snapshot());

        assert_eq!(devices[DESK].look.as_ref().map(|look| look.on), Some(false));
        assert!(devices[SHELF].look.is_none());
    }

    #[test]
    fn a_target_resolves_to_its_light() {
        let lights = vec![
            (DESK.to_string(), Some(0x0001)),
            (SHELF.to_string(), Some(0x1A2B)),
        ];
        let cases = [
            (device_target(""), None),
            (device_target(SHELF), Some(SHELF)),
            (device_target(&SHELF.to_lowercase()), Some(SHELF)),
            (device_target("0x1a2b"), Some(SHELF)),
            (device_target("0x0001"), Some(DESK)),
            (device_target("missing"), None),
            (
                LightTarget::Group {
                    id: crate::domain::model::GroupId("all".into()),
                },
                None,
            ),
        ];
        for (target, expected) in cases {
            assert_eq!(
                ieee_of_target(&target, &lights).as_deref(),
                expected,
                "{target:?}"
            );
        }
    }

    #[test]
    fn an_empty_target_means_the_sole_light() {
        let lights = vec![(DESK.to_string(), None)];
        assert_eq!(
            ieee_of_target(&device_target(""), &lights).as_deref(),
            Some(DESK)
        );
    }

    #[test]
    fn a_named_light_gives_its_own_look_and_an_unknown_one_is_refused() {
        let looks = LightLooks::new(&config_with(vec![
            entry(DESK, Some(dim())),
            entry(SHELF, None),
        ]));
        let lights = vec![(DESK.to_string(), None), (SHELF.to_string(), None)];
        let main = device_target(SHELF);

        assert_eq!(
            look_for(Some(&DESK.to_lowercase()), &main, &lights, &looks),
            Ok(dim())
        );
        assert_eq!(
            look_for(None, &main, &lights, &looks),
            Ok(looks.look(SHELF))
        );
        assert_eq!(
            look_for(Some("AA:AA:AA:AA:AA:AA:AA:AA"), &main, &lights, &looks),
            Err("That light is not paired.".to_string())
        );
    }

    #[test]
    fn with_no_main_light_to_resolve_the_live_look_is_used() {
        let looks = LightLooks::new(&config_with(vec![entry(DESK, Some(dim()))]));
        let lights = vec![(DESK.to_string(), None), (SHELF.to_string(), None)];

        let look = look_for(None, &device_target(""), &lights, &looks).unwrap();

        assert_eq!(look.brightness, 40);
        assert!(look.on);
    }

    #[test]
    fn config_addresses_come_back_in_a_stable_order() {
        let devices = config_with(vec![entry(SHELF, None), entry(DESK, None)]).devices;
        let addresses = config_addresses(&devices);
        assert_eq!(addresses[0].0, DESK);
        assert_eq!(addresses[1].0, SHELF);
    }
}
