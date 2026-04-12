use std::sync::{Arc, Mutex};

use crate::znp::{Device, Endpoint};
use anyhow::Result;

use crate::backend::zigbee::ZigbeeBackend;
use crate::config::model::{PluginConfig, PresetSlot};
use crate::config::store;
use crate::domain::model::{LightCommand, LightTarget, RgbColor};
use crate::runtime::actions;
use crate::service::light_service::LightService;

const BRIGHTNESS_STEP: u8 = 10;
const MIREK_STEP: u16 = 25;
const RELOAD_ACTION: &str = "reload";

#[derive(Clone)]
pub enum DaemonOutcome {
    Handled,
    HandledWithData(serde_json::Value),
    Fallback,
    Error(String),
}

pub struct DaemonState {
    service: Arc<Mutex<Option<LightService<ZigbeeBackend>>>>,
    config: PluginConfig,
    main_target: LightTarget,
    current_brightness: u8,
    current_mirek: u16,
}

impl DaemonState {
    pub fn new() -> Result<Self> {
        let loaded = load_runtime_state()?;
        Ok(Self {
            service: loaded.service,
            config: loaded.config,
            main_target: loaded.main_target,
            current_brightness: loaded.current_brightness,
            current_mirek: loaded.current_mirek,
        })
    }

    pub fn config(&self) -> &PluginConfig {
        &self.config
    }

    pub fn shared_service(&self) -> Arc<Mutex<Option<LightService<ZigbeeBackend>>>> {
        self.service.clone()
    }

    pub fn main_target(&self) -> &LightTarget {
        &self.main_target
    }

    pub fn events(&self) -> crossbeam_channel::Receiver<crate::znp::ZigbeeEvent> {
        self.service
            .lock()
            .unwrap()
            .as_ref()
            .expect("service not initialized")
            .backend()
            .events()
            .clone()
    }

    pub fn handle_action(&mut self, action: &str) -> DaemonOutcome {
        if action == RELOAD_ACTION {
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
            return self.with_service(|svc| svc.backend().permit_join(60));
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
        if let Some(preset) = self.config.preset_for_action(action) {
            return self.apply_preset(&preset);
        }
        if actions::is_run_action(action) {
            return DaemonOutcome::Error(format!(
                "plugin-lights action '{}' is not implemented yet",
                action
            ));
        }

        DaemonOutcome::Fallback
    }

    fn with_service<F>(&self, f: F) -> DaemonOutcome
    where
        F: FnOnce(&mut LightService<ZigbeeBackend>) -> Result<()>,
    {
        let mut guard = self.service.lock().unwrap();
        let Some(svc) = guard.as_mut() else {
            return DaemonOutcome::Error("service not available".into());
        };
        match f(svc) {
            Ok(()) => DaemonOutcome::Handled,
            Err(e) => DaemonOutcome::Error(e.to_string()),
        }
    }

    fn apply(&mut self, command: LightCommand) -> DaemonOutcome {
        let target = self.main_target.clone();
        self.with_service(|svc| svc.apply_command(&target, &command))
    }

    fn adjust_brightness(&mut self, delta: i16) -> DaemonOutcome {
        let new_level = (self.current_brightness as i16 + delta).clamp(0, 100) as u8;
        self.current_brightness = new_level;
        self.apply(LightCommand::SetBrightness { level: new_level })
    }

    fn apply_live_color(&mut self) -> DaemonOutcome {
        let config = match store::load() {
            Ok(config) => config,
            Err(error) => return DaemonOutcome::Error(error.to_string()),
        };
        let color = parse_color(&config.live_color_hex);
        self.apply(LightCommand::SetColor { color })
    }

    fn apply_live_brightness(&mut self) -> DaemonOutcome {
        let config = match store::load() {
            Ok(config) => config,
            Err(error) => return DaemonOutcome::Error(error.to_string()),
        };
        self.current_brightness = config.live_brightness;
        let result = self.apply(LightCommand::SetBrightness {
            level: config.live_brightness,
        });
        if matches!(result, DaemonOutcome::Handled) {
            let color = parse_color(&config.live_color_hex);
            self.apply(LightCommand::SetColor { color });
        }
        result
    }

    fn apply_live_colortemp(&mut self) -> DaemonOutcome {
        let config = match store::load() {
            Ok(config) => config,
            Err(error) => return DaemonOutcome::Error(error.to_string()),
        };
        self.current_mirek = config.live_mirek;
        self.apply(LightCommand::SetColorTemperature {
            mirek: config.live_mirek,
        })
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

        let commands = preset_commands(preset);
        let target = self.main_target.clone();
        let mut guard = self.service.lock().unwrap();
        let Some(svc) = guard.as_mut() else {
            return DaemonOutcome::Error("service not available".into());
        };
        for command in commands {
            if let Err(error) = svc.apply_command(&target, &command) {
                return DaemonOutcome::Error(error.to_string());
            }
        }

        DaemonOutcome::Handled
    }

    fn reload(&mut self) -> DaemonOutcome {
        let config = match store::load() {
            Ok(c) => c,
            Err(e) => return DaemonOutcome::Error(e.to_string()),
        };
        self.main_target = config.main_target();
        self.current_brightness = config.live_brightness;
        self.current_mirek = config.live_mirek;
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

    if config.backend.network_key == "auto" {
        config.backend.network_key = format_key(backend.network_key());
    }

    let live_devices = backend.devices();
    for (key, entry) in config.devices.iter_mut() {
        let nwk = parse_network_address(key).unwrap_or(0);
        entry.online = live_devices
            .iter()
            .any(|device| device.network_address == nwk);
    }
    store::save(&config)?;
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
            let network_address = parse_network_address(key).unwrap_or(0);
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
    RgbColor {
        red: parse_hex_pair(&color_hex[0..2]),
        green: parse_hex_pair(&color_hex[2..4]),
        blue: parse_hex_pair(&color_hex[4..6]),
    }
}

fn parse_hex_pair(value: &str) -> u8 {
    u8::from_str_radix(value, 16).unwrap_or(0)
}
