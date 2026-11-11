use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::znp::{Device, Endpoint};
use anyhow::Result;
use serde_json::Value;

use crate::backend::zigbee::ZigbeeBackend;
use crate::config::model::{PluginConfig, PresetSlot};
use crate::config::store;
use crate::domain::model::{DeviceId, LightCommand, LightTarget, RgbColor};
use crate::runtime::actions;
use crate::service::light_service::LightService;

use super::input::{self, BRIGHTNESS_RANGE, MIREK_RANGE};
use super::looks::{self, LightAddress, LightLooks, SharedLooks};
use super::pairing::{self, SharedPairing};
use super::persist::{self, Change};
use super::presets;

const BRIGHTNESS_STEP: u8 = 10;
const MIREK_STEP: u16 = 25;
const NOT_PAIRED: &str = "That light is not paired.";

#[derive(Clone)]
pub enum DaemonOutcome {
    Handled,
    HandledWithData(serde_json::Value),
    Fallback,
    Error(String),
}

pub struct DaemonState {
    // Option so reload() can take() the old service, closing the serial port
    // before opening a new one. The WS thread and device monitor hold Arc clones,
    // so replacing self.service alone wouldn't drop the inner — take() does.
    service: Arc<Mutex<Option<LightService<ZigbeeBackend>>>>,
    config: PluginConfig,
    main_target: SharedTarget,
    looks: SharedLooks,
    pairing: SharedPairing,
    current_brightness: u8,
    current_mirek: u16,
}

pub type SharedTarget = Arc<Mutex<LightTarget>>;

pub fn current_target(target: &SharedTarget) -> LightTarget {
    target
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

struct Delivery {
    ieee: Option<String>,
    applied: usize,
    failure: Option<String>,
}

fn deliver(
    svc: &mut LightService<ZigbeeBackend>,
    target: &LightTarget,
    commands: &[LightCommand],
    strict: bool,
) -> Delivery {
    let ieee = looks::ieee_of_target(target, &looks::light_addresses(&svc.backend().devices()));
    if strict && ieee.is_none() {
        return Delivery {
            ieee,
            applied: 0,
            failure: Some(NOT_PAIRED.into()),
        };
    }
    let mut applied = 0;
    for command in commands {
        if let Err(error) = svc.apply_command(target, command) {
            return Delivery {
                ieee,
                applied,
                failure: Some(error.to_string()),
            };
        }
        applied += 1;
    }
    Delivery {
        ieee,
        applied,
        failure: None,
    }
}

impl DaemonState {
    pub fn new() -> Result<Self> {
        let loaded = load_runtime_state()?;
        Ok(Self {
            service: loaded.service,
            looks: Arc::new(Mutex::new(LightLooks::new(&loaded.config))),
            config: loaded.config,
            main_target: Arc::new(Mutex::new(loaded.main_target)),
            pairing: SharedPairing::default(),
            current_brightness: loaded.current_brightness,
            current_mirek: loaded.current_mirek,
        })
    }

    pub fn shared_service(&self) -> Arc<Mutex<Option<LightService<ZigbeeBackend>>>> {
        self.service.clone()
    }

    pub fn shared_main_target(&self) -> SharedTarget {
        self.main_target.clone()
    }

    pub fn shared_looks(&self) -> SharedLooks {
        self.looks.clone()
    }

    pub fn shared_pairing(&self) -> SharedPairing {
        self.pairing.clone()
    }

    pub fn presets(&self) -> &crate::config::model::PresetConfig {
        &self.config.presets
    }

    pub fn events(&self) -> crossbeam_channel::Receiver<crate::znp::ZigbeeEvent> {
        self.service
            .lock()
            .expect("service lock poisoned during events()")
            .as_ref()
            .expect("service not initialized")
            .backend()
            .events()
            .clone()
    }

    pub fn handle_action(&mut self, action: &str) -> DaemonOutcome {
        self.handle_input_action(action, &Value::Null)
    }

    pub fn handle_input_action(&mut self, action: &str, input: &Value) -> DaemonOutcome {
        if action == actions::RELOAD {
            return self.reload();
        }
        if action == actions::TOGGLE_MAIN {
            return self.apply(LightCommand::Toggle);
        }
        if action == actions::ON_MAIN {
            return self.apply(LightCommand::TurnOn);
        }
        if action == actions::OFF_MAIN {
            return self.apply(LightCommand::TurnOff);
        }
        if action == actions::BRIGHTER_MAIN {
            return self.adjust_brightness(BRIGHTNESS_STEP as i16);
        }
        if action == actions::DIMMER_MAIN {
            return self.adjust_brightness(-(BRIGHTNESS_STEP as i16));
        }
        if action == actions::WARMER_MAIN {
            return self.adjust_mirek(MIREK_STEP as i32);
        }
        if action == actions::COOLER_MAIN {
            return self.adjust_mirek(-(MIREK_STEP as i32));
        }
        if action == actions::PAIR {
            let window = pairing::PAIRING_WINDOW.as_secs() as u8;
            let outcome = self.with_service(|svc| svc.backend().permit_join(window));
            if matches!(outcome, DaemonOutcome::Handled) {
                pairing::update(&self.pairing, |session| session.start(Instant::now()));
            }
            return outcome;
        }
        if action == actions::STOP_PAIR {
            let outcome = self.with_service(|svc| svc.backend().permit_join(0));
            if matches!(outcome, DaemonOutcome::Handled) {
                pairing::update(&self.pairing, |session| session.stop(Instant::now()));
            }
            return outcome;
        }
        if action == actions::SET_COLOR_MAIN {
            return self.apply_live_color();
        }
        if action == actions::SET_BRIGHTNESS_MAIN {
            return self.apply_live_brightness();
        }
        if action == actions::SET_COLORTEMP_MAIN {
            return self.apply_live_colortemp();
        }
        if let Some(outcome) = self.handle_light_action(action, input) {
            return outcome;
        }
        if presets::is_preset_action(action) {
            return self.edit_presets(action, input);
        }
        if let Some(preset) = self.config.preset_for_action(action) {
            return self.apply_preset(&preset);
        }
        if actions::is_run_action(action) {
            return DaemonOutcome::Error(format!(
                "{} action '{}' is not implemented yet",
                store::PLUGIN_ID,
                action
            ));
        }

        DaemonOutcome::Fallback
    }

    fn handle_light_action(&mut self, action: &str, input: &Value) -> Option<DaemonOutcome> {
        let outcome = match action {
            actions::LIGHT_ON => self.light_command(input, |_| Ok(vec![LightCommand::TurnOn])),
            actions::LIGHT_OFF => self.light_command(input, |_| Ok(vec![LightCommand::TurnOff])),
            actions::SET_LIGHT_COLOR => self.light_command(input, |input| {
                let color = parse_color(&input::color(input)?);
                Ok(vec![LightCommand::SetColor { color }])
            }),
            actions::SET_LIGHT_BRIGHTNESS => self.light_command(input, |input| {
                let level = input::number(input, "Brightness", BRIGHTNESS_RANGE)? as u8;
                Ok(vec![LightCommand::SetBrightness { level }])
            }),
            actions::SET_LIGHT_COLORTEMP => self.light_command(input, |input| {
                let mirek = input::number(input, "Colour temperature", MIREK_RANGE)?;
                Ok(vec![LightCommand::SetColorTemperature { mirek }])
            }),
            actions::REMOVE_LIGHT => self.remove_light(input),
            _ => return None,
        };
        Some(outcome)
    }

    fn light_command(
        &self,
        input: &Value,
        build: impl FnOnce(&Value) -> Result<Vec<LightCommand>, String>,
    ) -> DaemonOutcome {
        let ieee = match input::ieee(input) {
            Ok(ieee) => ieee,
            Err(message) => return DaemonOutcome::Error(message),
        };
        let commands = match build(input) {
            Ok(commands) => commands,
            Err(message) => return DaemonOutcome::Error(message),
        };
        let target = LightTarget::Device { id: DeviceId(ieee) };
        self.send(&target, &commands, true)
    }

    fn remove_light(&mut self, input: &Value) -> DaemonOutcome {
        let ieee = match input::ieee(input) {
            Ok(ieee) => ieee,
            Err(message) => return DaemonOutcome::Error(message),
        };
        let Ok(address) = parse_ieee_address(&ieee) else {
            return DaemonOutcome::Error("That is not a light address.".into());
        };
        let Some(network_address) = self.locate(&ieee) else {
            return DaemonOutcome::Error(NOT_PAIRED.into());
        };
        if let DaemonOutcome::Error(message) =
            self.with_service(|svc| svc.backend().remove_device(&address))
        {
            log::warn!(
                "light {ieee} did not answer its leave request ({message}), forgetting it anyway"
            );
        }
        self.forget_light(&ieee, network_address);
        DaemonOutcome::Handled
    }

    fn locate(&self, ieee: &str) -> Option<Option<u16>> {
        let paired = self
            .addresses()
            .into_iter()
            .find(|(known, _)| known == ieee);
        let configured = self
            .config
            .devices
            .values()
            .find(|entry| looks::normalize(&entry.ieee_address) == ieee);
        match (paired, configured) {
            (Some((_, network)), _) => Some(network),
            (None, Some(entry)) => Some(entry.network_address),
            (None, None) => None,
        }
    }

    fn forget_light(&mut self, ieee: &str, network_address: Option<u16>) {
        looks::lock(&self.looks).forget(ieee);
        self.config
            .devices
            .retain(|_, entry| looks::normalize(&entry.ieee_address) != ieee);
        let named = !self.config.main_target_id.is_empty()
            && looks::names_light(&self.config.main_target_id, ieee, network_address);
        if named {
            self.config.main_target_id.clear();
            *self
                .main_target
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = self.config.main_target();
        }
        persist::submit(Change::Forget {
            ieee: ieee.to_string(),
            network_address,
        });
    }

    fn addresses(&self) -> Vec<LightAddress> {
        if let Ok(guard) = self.service.lock() {
            if let Some(svc) = guard.as_ref() {
                return looks::light_addresses(&svc.backend().devices());
            }
        }
        looks::config_addresses(&self.config.devices)
    }

    fn edit_presets(&mut self, action: &str, input: &Value) -> DaemonOutcome {
        let mut edited = self.config.presets.clone();
        let target = current_target(&self.main_target);
        let addresses = self.addresses();
        let result = presets::apply(&mut edited, action, input, |ieee| {
            looks::look_for(ieee, &target, &addresses, &looks::lock(&self.looks))
        });
        match result {
            Ok(()) => {
                self.config.presets = edited.clone();
                persist::submit(Change::Presets(edited));
                DaemonOutcome::Handled
            }
            Err(message) => DaemonOutcome::Error(message),
        }
    }

    fn with_service_value<T>(
        &self,
        f: impl FnOnce(&mut LightService<ZigbeeBackend>) -> T,
    ) -> Result<T, String> {
        let Ok(mut guard) = self.service.lock() else {
            return Err("service lock poisoned".into());
        };
        let Some(svc) = guard.as_mut() else {
            return Err("service not available".into());
        };
        Ok(f(svc))
    }

    fn with_service<F>(&self, f: F) -> DaemonOutcome
    where
        F: FnOnce(&mut LightService<ZigbeeBackend>) -> Result<()>,
    {
        match self.with_service_value(f) {
            Ok(Ok(())) => DaemonOutcome::Handled,
            Ok(Err(error)) => DaemonOutcome::Error(error.to_string()),
            Err(message) => DaemonOutcome::Error(message),
        }
    }

    fn send(&self, target: &LightTarget, commands: &[LightCommand], strict: bool) -> DaemonOutcome {
        let delivery = match self.with_service_value(|svc| deliver(svc, target, commands, strict)) {
            Ok(delivery) => delivery,
            Err(message) => return DaemonOutcome::Error(message),
        };
        if let Some(ieee) = &delivery.ieee {
            looks::record(&self.looks, ieee, &commands[..delivery.applied]);
        }
        delivery
            .failure
            .map_or(DaemonOutcome::Handled, DaemonOutcome::Error)
    }

    fn apply(&mut self, command: LightCommand) -> DaemonOutcome {
        let target = current_target(&self.main_target);
        self.send(&target, &[command], false)
    }

    fn adjust_brightness(&mut self, delta: i16) -> DaemonOutcome {
        let new_level = (self.current_brightness as i16 + delta).clamp(0, 100) as u8;
        self.current_brightness = new_level;
        self.apply(LightCommand::SetBrightness { level: new_level })
    }

    fn apply_live_color(&mut self) -> DaemonOutcome {
        let config = match self.fresh_config() {
            Ok(config) => config,
            Err(outcome) => return outcome,
        };
        let color = parse_color(&config.live_color_hex);
        self.apply(LightCommand::SetColor { color })
    }

    fn apply_live_brightness(&mut self) -> DaemonOutcome {
        let config = match self.fresh_config() {
            Ok(config) => config,
            Err(outcome) => return outcome,
        };
        self.current_brightness = config.live_brightness;
        let brightness_result = self.apply(LightCommand::SetBrightness {
            level: config.live_brightness,
        });
        if !matches!(brightness_result, DaemonOutcome::Handled) {
            return brightness_result;
        }
        let color = parse_color(&config.live_color_hex);
        self.apply(LightCommand::SetColor { color })
    }

    fn apply_live_colortemp(&mut self) -> DaemonOutcome {
        let config = match self.fresh_config() {
            Ok(config) => config,
            Err(outcome) => return outcome,
        };
        self.current_mirek = config.live_mirek;
        self.apply(LightCommand::SetColorTemperature {
            mirek: config.live_mirek,
        })
    }

    fn fresh_config(&self) -> Result<PluginConfig, DaemonOutcome> {
        let config = store::load().map_err(|error| DaemonOutcome::Error(error.to_string()))?;
        looks::lock(&self.looks).set_fallback(&config);
        Ok(config)
    }

    fn adjust_mirek(&mut self, delta: i32) -> DaemonOutcome {
        let new_mirek = (self.current_mirek as i32 + delta).clamp(153, 500) as u16;
        self.current_mirek = new_mirek;
        self.apply(LightCommand::SetColorTemperature { mirek: new_mirek })
    }

    fn apply_preset(&mut self, preset: &PresetSlot) -> DaemonOutcome {
        if !preset.enabled {
            return DaemonOutcome::Error(format!("preset '{}' is disabled", preset.name));
        }

        let target = current_target(&self.main_target);
        self.send(&target, &preset_commands(preset), false)
    }

    // Config-only reload: re-reads config.json without re-opening the coordinator.
    // The coordinator stays connected; only target, brightness, etc. update.
    // A full re-init (close serial + reopen) is handled by the Unavailable→reload
    // path in dispatch_action, not here.
    fn reload(&mut self) -> DaemonOutcome {
        let config = match store::load() {
            Ok(c) => c,
            Err(e) => return DaemonOutcome::Error(e.to_string()),
        };
        *self
            .main_target
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = config.main_target();
        self.current_brightness = config.live_brightness;
        self.current_mirek = config.live_mirek;
        looks::lock(&self.looks).adopt(&config);
        self.config = config;
        DaemonOutcome::Handled
    }
}

struct LoadedRuntimeState {
    service: Arc<Mutex<Option<LightService<ZigbeeBackend>>>>,
    config: PluginConfig,
    main_target: LightTarget,
    current_brightness: u8,
    current_mirek: u16,
}

fn load_runtime_state() -> Result<LoadedRuntimeState> {
    let mut config = store::load()?;
    let main_target = config.main_target();
    let persisted_devices = load_persisted_devices(&config);
    let backend = ZigbeeBackend::open(&config.backend, persisted_devices)?;

    // Only write config on first boot (auto → resolved key). Never on subsequent
    // startups — the frontend also writes this file, and a startup save would
    // overwrite fields the frontend manages (e.g. live_color_hex) with stale values.
    if config.backend.network_key == "auto" {
        config.backend.network_key = format_key(backend.network_key());
        let _ = store::save(&config);
    }
    let current_brightness = config.live_brightness;
    let current_mirek = config.live_mirek;

    Ok(LoadedRuntimeState {
        service: Arc::new(Mutex::new(Some(LightService::new(backend)))),
        config,
        main_target,
        current_brightness,
        current_mirek,
    })
}

fn load_persisted_devices(config: &PluginConfig) -> Vec<Device> {
    config
        .devices
        .iter()
        .filter_map(|(key, entry)| {
            let ieee_address = parse_ieee_address(&entry.ieee_address).ok()?;
            let network_address = entry
                .network_address
                .or_else(|| parse_network_address(key))
                .unwrap_or(0);
            let endpoints = entry
                .endpoints
                .iter()
                .map(|endpoint| Endpoint {
                    id: endpoint.id,
                    input_clusters: endpoint.clusters.clone(),
                })
                .collect();
            Some(Device {
                network_address,
                ieee_address,
                endpoints,
            })
        })
        .collect()
}

fn parse_network_address(key: &str) -> Option<u16> {
    let stripped = key.strip_prefix("0x").or_else(|| key.strip_prefix("0X"))?;
    u16::from_str_radix(stripped, 16).ok()
}

fn parse_ieee_address(hex: &str) -> Result<[u8; 8], String> {
    let parts: Vec<&str> = hex.split(':').collect();
    if parts.len() != 8 {
        return Err(format!(
            "expected 8 colon-separated hex bytes, got {}",
            parts.len()
        ));
    }
    let mut addr = [0u8; 8];
    for (index, part) in parts.iter().enumerate() {
        addr[index] = u8::from_str_radix(part, 16)
            .map_err(|_| format!("invalid hex byte '{}' at position {}", part, index))?;
    }
    Ok(addr)
}

fn format_key(key: &[u8; 16]) -> String {
    key.iter()
        .map(|byte| format!("{:02X}", byte))
        .collect::<Vec<_>>()
        .join(":")
}

fn preset_commands(preset: &PresetSlot) -> Vec<LightCommand> {
    vec![
        power_command(preset.power_on),
        LightCommand::SetBrightness {
            level: preset.brightness,
        },
        LightCommand::SetColor {
            color: parse_color(&preset.color_hex),
        },
        LightCommand::SetColorTemperature {
            mirek: preset.mirek,
        },
    ]
}

fn power_command(power_on: bool) -> LightCommand {
    if power_on {
        return LightCommand::TurnOn;
    }

    LightCommand::TurnOff
}

fn parse_color(color_hex: &str) -> RgbColor {
    let hex = color_hex.strip_prefix('#').unwrap_or(color_hex);
    RgbColor {
        red: parse_hex_pair(hex.get(0..2).unwrap_or("00")),
        green: parse_hex_pair(hex.get(2..4).unwrap_or("00")),
        blue: parse_hex_pair(hex.get(4..6).unwrap_or("00")),
    }
}

fn parse_hex_pair(value: &str) -> u8 {
    u8::from_str_radix(value, 16).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{DeviceEntry, EndpointEntry};

    fn entry(network_address: Option<u16>) -> DeviceEntry {
        DeviceEntry {
            ieee_address: "00:11:22:33:44:55:66:77".into(),
            name: "Desk".into(),
            endpoints: vec![EndpointEntry {
                id: 11,
                clusters: vec![6, 8],
            }],
            online: true,
            network_address,
            look: None,
        }
    }

    #[test]
    fn persisted_devices_keep_their_network_address() {
        let cases = [
            ("00:11:22:33:44:55:66:77", Some(0x4A1F), 0x4A1F),
            ("0x1234", None, 0x1234),
            ("0x1234", Some(0x4A1F), 0x4A1F),
            ("00:11:22:33:44:55:66:77", None, 0x0000),
        ];
        for (key, stored, expected) in cases {
            let mut config = PluginConfig::default();
            config.devices.insert(key.into(), entry(stored));
            let devices = load_persisted_devices(&config);
            assert_eq!(devices.len(), 1, "key {key}");
            assert_eq!(devices[0].network_address, expected, "key {key}");
            assert_eq!(
                devices[0].ieee_address,
                [0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]
            );
        }
    }

    #[test]
    fn device_entries_written_before_the_network_address_field_still_load() {
        let entry: DeviceEntry = serde_json::from_value(serde_json::json!({
            "ieee_address": "00:11:22:33:44:55:66:77",
            "name": "Desk",
            "endpoints": [],
        }))
        .unwrap();
        assert_eq!(entry.network_address, None);
        let written = serde_json::to_value(entry).unwrap();
        assert!(written.get("network_address").is_none());
    }
}
