use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GroupId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LightTarget {
    Device { id: DeviceId },
    Group { id: GroupId },
}

impl LightTarget {
    pub fn main_device() -> Self {
        Self::Device {
            id: DeviceId("main".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightCapabilities {
    pub supports_power: bool,
    pub supports_brightness: bool,
    pub supports_color: bool,
    pub supports_color_temperature: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_mirek: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mirek: Option<u16>,
}

impl LightCapabilities {
    pub fn rgb_cct() -> Self {
        Self {
            supports_power: true,
            supports_brightness: true,
            supports_color: true,
            supports_color_temperature: true,
            min_mirek: Some(153),
            max_mirek: Some(500),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RgbColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LightState {
    pub power: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<RgbColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirek: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LightCommand {
    TurnOn,
    TurnOff,
    Toggle,
    SetBrightness { level: u8 },
    SetColor { color: RgbColor },
    SetColorTemperature { mirek: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub target: LightTarget,
    #[serde(default)]
    pub commands: Vec<LightCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightTargetInfo {
    pub target: LightTarget,
    pub name: String,
    pub capabilities: LightCapabilities,
    pub state: LightState,
}

impl LightTargetInfo {
    pub fn main_rgb_cct() -> Self {
        Self {
            target: LightTarget::main_device(),
            name: "Main Light".into(),
            capabilities: LightCapabilities::rgb_cct(),
            state: LightState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendHealth {
    pub status: BackendConnectionStatus,
    pub summary: String,
}

impl BackendHealth {
    pub fn degraded(summary: impl Into<String>) -> Self {
        Self {
            status: BackendConnectionStatus::Degraded,
            summary: summary.into(),
        }
    }
}
