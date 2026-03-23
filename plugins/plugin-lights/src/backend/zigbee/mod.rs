use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::Receiver;
use crate::znp::zcl;
use crate::znp::{ControllerConfig, Device, ZigbeeController, ZigbeeEvent};

use crate::backend::LightBackend;
use crate::config::model::BackendConfig;
use crate::domain::model::{
    BackendConnectionStatus, BackendHealth, DeviceId, LightCapabilities, LightCommand, LightState,
    LightTarget, LightTargetInfo,
};

pub struct ZigbeeBackend {
    controller: ZigbeeController,
    state_cache: HashMap<u16, LightState>,
    resolved_network_key: [u8; 16],
}

impl ZigbeeBackend {
    pub fn open(config: &BackendConfig, persisted_devices: Vec<Device>) -> Result<Self> {
        let serial_port = resolve_serial_port(&config.serial_port)?;
        let network_key = resolve_network_key(&config.network_key)?;

        let controller_config = ControllerConfig {
            serial_port,
            network_key,
            channel: config.channel,
            ..ControllerConfig::default()
        };

        let controller = ZigbeeController::open(controller_config, persisted_devices)
            .context("failed to open zigbee controller")?;

        Ok(Self {
            controller,
            state_cache: HashMap::new(),
            resolved_network_key: network_key,
        })
    }

    pub fn network_key(&self) -> &[u8; 16] {
        &self.resolved_network_key
    }

    pub fn permit_join(&self, duration_secs: u8) -> Result<()> {
        self.controller.permit_join(duration_secs)
    }

    pub fn events(&self) -> &Receiver<ZigbeeEvent> {
        self.controller.events()
    }

    pub fn devices(&self) -> Vec<Device> {
        self.controller.devices()
    }
}

impl LightBackend for ZigbeeBackend {
    fn kind(&self) -> &'static str {
        "zigbee-direct"
    }

    fn health(&self) -> BackendHealth {
        BackendHealth {
            status: BackendConnectionStatus::Connected,
            summary: "zigbee coordinator connected".into(),
        }
    }

    fn list_targets(&self) -> Result<Vec<LightTargetInfo>> {
        let devices = self.controller.devices();
        let targets = devices
            .iter()
            .map(|device| {
                let id = format!("0x{:04X}", device.network_address);
                LightTargetInfo {
                    target: LightTarget::Device {
                        id: DeviceId(id.clone()),
                    },
                    name: id,
                    capabilities: capabilities_from_device(device),
                    state: self
                        .state_cache
                        .get(&device.network_address)
                        .cloned()
                        .unwrap_or_default(),
                }
            })
            .collect();
        Ok(targets)
    }

    fn apply_command(&mut self, target: &LightTarget, command: &LightCommand) -> Result<()> {
        let (nwk_addr, device) = resolve_target(target, &self.controller)?;

        match command {
            LightCommand::TurnOn => {
                let ep = device
                    .endpoint_for_cluster(zcl::CLUSTER_ON_OFF)
                    .ok_or_else(|| anyhow!("device has no on/off cluster"))?;
                self.controller.send_cluster_command(
                    nwk_addr,
                    ep,
                    zcl::CLUSTER_ON_OFF,
                    zcl::on_off::on(),
                )
            }
            LightCommand::TurnOff => {
                let ep = device
                    .endpoint_for_cluster(zcl::CLUSTER_ON_OFF)
                    .ok_or_else(|| anyhow!("device has no on/off cluster"))?;
                self.controller.send_cluster_command(
                    nwk_addr,
                    ep,
                    zcl::CLUSTER_ON_OFF,
                    zcl::on_off::off(),
                )
            }
            LightCommand::Toggle => {
                let ep = device
                    .endpoint_for_cluster(zcl::CLUSTER_ON_OFF)
                    .ok_or_else(|| anyhow!("device has no on/off cluster"))?;
                self.controller.send_cluster_command(
                    nwk_addr,
                    ep,
                    zcl::CLUSTER_ON_OFF,
                    zcl::on_off::toggle(),
                )
            }
            LightCommand::SetBrightness { level } => {
                let ep = device
                    .endpoint_for_cluster(zcl::CLUSTER_LEVEL)
                    .ok_or_else(|| anyhow!("device has no level cluster"))?;
                let zcl_level = (*level as u16 * 254 / 100) as u8;
                self.controller.send_cluster_command(
                    nwk_addr,
                    ep,
                    zcl::CLUSTER_LEVEL,
                    zcl::level::move_to_level(zcl_level, 10),
                )
            }
            LightCommand::SetColor { color } => {
                let ep = device
                    .endpoint_for_cluster(zcl::CLUSTER_COLOR)
                    .ok_or_else(|| anyhow!("device has no color cluster"))?;
                let (cx, cy) = rgb_to_cie_xy(color.red, color.green, color.blue);
                self.controller.send_cluster_command(
                    nwk_addr,
                    ep,
                    zcl::CLUSTER_COLOR,
                    zcl::color::move_to_color(cx, cy, 10),
                )
            }
            LightCommand::SetColorTemperature { mirek } => {
                let ep = device
                    .endpoint_for_cluster(zcl::CLUSTER_COLOR)
                    .ok_or_else(|| anyhow!("device has no color cluster"))?;
                self.controller.send_cluster_command(
                    nwk_addr,
                    ep,
                    zcl::CLUSTER_COLOR,
                    zcl::color::move_to_color_temp(*mirek, 10),
                )
            }
        }
    }
}

fn resolve_serial_port(configured: &str) -> Result<String> {
    if configured == "auto" {
        return crate::znp::detect_sonoff()
            .ok_or_else(|| anyhow!("no Sonoff dongle detected; set serial_port manually"));
    }
    Ok(configured.to_string())
}

fn resolve_network_key(configured: &str) -> Result<[u8; 16]> {
    if configured == "auto" {
        let mut key = [0u8; 16];
        getrandom::getrandom(&mut key)
            .map_err(|e| anyhow!("failed to generate random network key: {}", e))?;
        return Ok(key);
    }

    let parts: Vec<&str> = configured.split(':').collect();
    if parts.len() != 16 {
        return Err(anyhow!(
            "network_key must be 16 colon-separated hex bytes, got {} parts",
            parts.len()
        ));
    }

    let mut key = [0u8; 16];
    for (i, part) in parts.iter().enumerate() {
        key[i] = u8::from_str_radix(part, 16)
            .with_context(|| format!("invalid hex byte '{}' at position {}", part, i))?;
    }
    Ok(key)
}

fn capabilities_from_device(device: &Device) -> LightCapabilities {
    let has_cluster = |id: u16| {
        device
            .endpoints
            .iter()
            .any(|ep| ep.input_clusters.contains(&id))
    };

    LightCapabilities {
        supports_power: has_cluster(zcl::CLUSTER_ON_OFF),
        supports_brightness: has_cluster(zcl::CLUSTER_LEVEL),
        supports_color: has_cluster(zcl::CLUSTER_COLOR),
        supports_color_temperature: has_cluster(zcl::CLUSTER_COLOR),
        min_mirek: if has_cluster(zcl::CLUSTER_COLOR) {
            Some(153)
        } else {
            None
        },
        max_mirek: if has_cluster(zcl::CLUSTER_COLOR) {
            Some(500)
        } else {
            None
        },
    }
}

fn rgb_to_cie_xy(r: u8, g: u8, b: u8) -> (u16, u16) {
    let gamma = |v: f64| {
        if v > 0.04045 {
            ((v + 0.055) / 1.055).powf(2.4)
        } else {
            v / 12.92
        }
    };

    let rf = gamma(r as f64 / 255.0);
    let gf = gamma(g as f64 / 255.0);
    let bf = gamma(b as f64 / 255.0);

    let x = rf * 0.4124 + gf * 0.3576 + bf * 0.1805;
    let y = rf * 0.2126 + gf * 0.7152 + bf * 0.0722;
    let z = rf * 0.0193 + gf * 0.1192 + bf * 0.9505;

    let sum = x + y + z;
    if sum < f64::EPSILON {
        return (0, 0);
    }

    let cx = x / sum;
    let cy = y / sum;

    ((cx * 65535.0) as u16, (cy * 65535.0) as u16)
}

fn resolve_target(target: &LightTarget, controller: &ZigbeeController) -> Result<(u16, Device)> {
    let hex_id = match target {
        LightTarget::Device { id } => &id.0,
        LightTarget::Group { .. } => return Err(anyhow!("group targets not supported yet")),
    };

    let nwk_addr = parse_hex_address(hex_id)?;
    let devices = controller.devices();
    let device = devices
        .iter()
        .find(|d| d.network_address == nwk_addr)
        .ok_or_else(|| anyhow!("device {} not found", hex_id))?
        .clone();

    Ok((nwk_addr, device))
}

fn parse_hex_address(hex_id: &str) -> Result<u16> {
    let stripped = hex_id.strip_prefix("0x").unwrap_or(hex_id);
    u16::from_str_radix(stripped, 16).with_context(|| format!("invalid hex address '{}'", hex_id))
}
