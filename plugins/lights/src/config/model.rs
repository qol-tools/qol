use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::model::{DeviceId, GroupId, LightTarget};
use crate::runtime::actions;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginConfig {
    pub backend: BackendConfig,
    pub main_target_type: String,
    pub main_target_id: String,
    #[serde(default)]
    pub devices: HashMap<String, DeviceEntry>,
    #[serde(default = "default_color")]
    pub live_color_hex: String,
    #[serde(default = "default_brightness")]
    pub live_brightness: u8,
    #[serde(default = "default_mirek")]
    pub live_mirek: u16,
    pub presets: PresetConfig,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            backend: BackendConfig::default(),
            main_target_type: "device".into(),
            main_target_id: String::new(),
            devices: HashMap::new(),
            live_color_hex: default_color(),
            live_brightness: default_brightness(),
            live_mirek: default_mirek(),
            presets: PresetConfig::default(),
        }
    }
}

impl PluginConfig {
    pub fn main_target(&self) -> LightTarget {
        target_from_parts(&self.main_target_type, &self.main_target_id)
    }

    pub fn preset_for_action(&self, action: &str) -> Option<PresetSlot> {
        self.presets.preset_for_action(action)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendConfig {
    pub kind: String,
    pub serial_port: String,
    pub channel: u8,
    pub network_key: String,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            kind: "zigbee-direct".into(),
            serial_port: "auto".into(),
            channel: 11,
            network_key: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub ieee_address: String,
    pub name: String,
    pub endpoints: Vec<EndpointEntry>,
    #[serde(default)]
    pub online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_address: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<LightLook>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightLook {
    pub on: bool,
    pub color_hex: String,
    pub brightness: u8,
    pub mirek: u16,
}

impl LightLook {
    pub fn from_live(config: &PluginConfig) -> Self {
        Self {
            on: true,
            color_hex: config.live_color_hex.clone(),
            brightness: config.live_brightness,
            mirek: config.live_mirek,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointEntry {
    pub id: u8,
    pub clusters: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetConfig {
    pub preset_1: PresetSlot,
    pub preset_2: PresetSlot,
    pub preset_3: PresetSlot,
    pub preset_4: PresetSlot,
    pub preset_5: PresetSlot,
    pub preset_6: PresetSlot,
    pub preset_7: PresetSlot,
    pub preset_8: PresetSlot,
}

impl Default for PresetConfig {
    fn default() -> Self {
        Self {
            preset_1: PresetSlot::named("Preset 1"),
            preset_2: PresetSlot::named("Preset 2"),
            preset_3: PresetSlot::named("Preset 3"),
            preset_4: PresetSlot::named("Preset 4"),
            preset_5: PresetSlot::named("Preset 5"),
            preset_6: PresetSlot::named("Preset 6"),
            preset_7: PresetSlot::named("Preset 7"),
            preset_8: PresetSlot::named("Preset 8"),
        }
    }
}

pub const PRESET_COUNT: usize = 8;

impl PresetConfig {
    pub fn slot(&self, number: usize) -> Option<&PresetSlot> {
        Some(match number {
            1 => &self.preset_1,
            2 => &self.preset_2,
            3 => &self.preset_3,
            4 => &self.preset_4,
            5 => &self.preset_5,
            6 => &self.preset_6,
            7 => &self.preset_7,
            8 => &self.preset_8,
            _ => return None,
        })
    }

    pub fn slot_mut(&mut self, number: usize) -> Option<&mut PresetSlot> {
        Some(match number {
            1 => &mut self.preset_1,
            2 => &mut self.preset_2,
            3 => &mut self.preset_3,
            4 => &mut self.preset_4,
            5 => &mut self.preset_5,
            6 => &mut self.preset_6,
            7 => &mut self.preset_7,
            8 => &mut self.preset_8,
            _ => return None,
        })
    }

    pub fn preset_for_action(&self, action: &str) -> Option<PresetSlot> {
        if action == actions::PRESET_1 {
            return Some(self.preset_1.clone());
        }
        if action == actions::PRESET_2 {
            return Some(self.preset_2.clone());
        }
        if action == actions::PRESET_3 {
            return Some(self.preset_3.clone());
        }
        if action == actions::PRESET_4 {
            return Some(self.preset_4.clone());
        }
        if action == actions::PRESET_5 {
            return Some(self.preset_5.clone());
        }
        if action == actions::PRESET_6 {
            return Some(self.preset_6.clone());
        }
        if action == actions::PRESET_7 {
            return Some(self.preset_7.clone());
        }
        if action == actions::PRESET_8 {
            return Some(self.preset_8.clone());
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetSlot {
    pub enabled: bool,
    pub name: String,
    pub power_on: bool,
    pub brightness: u8,
    pub color_hex: String,
    pub mirek: u16,
}

impl PresetSlot {
    pub fn named(name: &str) -> Self {
        Self {
            enabled: false,
            name: name.into(),
            power_on: true,
            brightness: 100,
            color_hex: default_color(),
            mirek: 300,
        }
    }
}

impl Default for PresetSlot {
    fn default() -> Self {
        Self::named("Preset")
    }
}

fn default_color() -> String {
    "#FFFFFF".into()
}

fn default_brightness() -> u8 {
    100
}

fn default_mirek() -> u16 {
    300
}

fn target_from_parts(target_type: &str, target_id: &str) -> LightTarget {
    if target_type == "group" {
        return LightTarget::Group {
            id: GroupId(target_id.to_string()),
        };
    }

    LightTarget::Device {
        id: DeviceId(target_id.to_string()),
    }
}
