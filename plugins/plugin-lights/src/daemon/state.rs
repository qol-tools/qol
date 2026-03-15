use anyhow::Result;
use zigbee_znp::{Device, Endpoint};

use crate::backend::zigbee::ZigbeeBackend;
use crate::config::model::{PluginConfig, PresetSlot};
use crate::config::store;
use crate::domain::model::{LightCommand, LightTarget, RgbColor};
use crate::runtime::actions;
use crate::service::light_service::LightService;

const BRIGHTNESS_STEP: u8 = 10;
const MIREK_STEP: u16 = 25;

pub enum DaemonOutcome {
    Handled,
    Fallback,
    Error(String),
}

pub struct DaemonState {
    service: LightService<ZigbeeBackend>,
    config: PluginConfig,
    main_target: LightTarget,
    current_brightness: u8,
    current_mirek: u16,
}

impl DaemonState {
    pub fn new() -> Result<Self> {
        let mut config = store::load()?;
        let main_target = config.main_target();
        let persisted_devices = load_persisted_devices(&config);

        let backend = ZigbeeBackend::open(&config.backend, persisted_devices)?;

        if config.backend.network_key == "auto" {
            config.backend.network_key = format_key(backend.network_key());
            store::save(&config)?;
        }

        let service = LightService::new(backend);
        Ok(Self {
            service,
            config,
            main_target,
            current_brightness: 50,
            current_mirek: 300,
        })
    }

    pub fn handle_action(&mut self, action: &str) -> DaemonOutcome {
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
            return match self.service.backend().permit_join(60) {
                Ok(()) => DaemonOutcome::Handled,
                Err(error) => DaemonOutcome::Error(error.to_string()),
            };
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

    fn apply(&mut self, command: LightCommand) -> DaemonOutcome {
        match self
            .service
            .apply_command(&self.main_target, &command)
        {
            Ok(()) => DaemonOutcome::Handled,
            Err(error) => DaemonOutcome::Error(error.to_string()),
        }
    }

    fn adjust_brightness(&mut self, delta: i16) -> DaemonOutcome {
        let new_level = (self.current_brightness as i16 + delta).clamp(0, 100) as u8;
        self.current_brightness = new_level;
        self.apply(LightCommand::SetBrightness { level: new_level })
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
        for command in commands {
            let result = self.service.apply_command(&self.main_target, &command);
            if let Err(error) = result {
                return DaemonOutcome::Error(error.to_string());
            }
        }

        DaemonOutcome::Handled
    }
}

fn load_persisted_devices(config: &PluginConfig) -> Vec<Device> {
    config
        .devices
        .values()
        .filter_map(|entry| {
            let ieee_address = parse_ieee_address(&entry.ieee_address).ok()?;
            let endpoints = entry
                .endpoints
                .iter()
                .map(|ep| Endpoint {
                    id: ep.id,
                    input_clusters: ep.clusters.clone(),
                })
                .collect();
            Some(Device {
                network_address: 0,
                ieee_address,
                endpoints,
            })
        })
        .collect()
}

fn parse_ieee_address(hex: &str) -> Result<[u8; 8], String> {
    let parts: Vec<&str> = hex.split(':').collect();
    if parts.len() != 8 {
        return Err(format!("expected 8 colon-separated hex bytes, got {}", parts.len()));
    }
    let mut addr = [0u8; 8];
    for (i, part) in parts.iter().enumerate() {
        addr[i] = u8::from_str_radix(part, 16)
            .map_err(|_| format!("invalid hex byte '{}' at position {}", part, i))?;
    }
    Ok(addr)
}

fn format_key(key: &[u8; 16]) -> String {
    key.iter()
        .map(|b| format!("{:02X}", b))
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
        LightCommand::SetColorTemperature { mirek: preset.mirek },
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
