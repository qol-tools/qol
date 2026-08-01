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
pub struct LightTargetInfo {
    pub target: LightTarget,
    pub name: String,
    pub capabilities: LightCapabilities,
    pub state: LightState,
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
