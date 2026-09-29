use std::{fmt, str::FromStr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PointzDeviceId([u8; 16]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("PointZ device identifier must be canonical unpadded base64url encoding of 16 bytes")]
pub struct PointzDeviceIdError;

impl PointzDeviceId {
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl FromStr for PointzDeviceId {
    type Err = PointzDeviceIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 22 {
            return Err(PointzDeviceIdError);
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| PointzDeviceIdError)?;
        let bytes = decoded.try_into().map_err(|_| PointzDeviceIdError)?;
        if URL_SAFE_NO_PAD.encode(bytes) != value {
            return Err(PointzDeviceIdError);
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for PointzDeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl fmt::Debug for PointzDeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PointzDeviceId")
            .field(&self.to_string())
            .finish()
    }
}

impl Serialize for PointzDeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PointzDeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointzImportSource {
    Fresh,
    Legacy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointzImport {
    pub source: PointzImportSource,
    pub imported: u32,
    pub dropped: u32,
    pub seed_replaced: bool,
    pub devices_unreadable: bool,
}

impl PointzImport {
    pub const FRESH: Self = Self {
        source: PointzImportSource::Fresh,
        imported: 0,
        dropped: 0,
        seed_replaced: false,
        devices_unreadable: false,
    };

    pub fn phones_must_pair_again(&self) -> bool {
        self.seed_replaced || self.devices_unreadable || self.dropped > 0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointzDevice {
    pub device_id: PointzDeviceId,
    pub name: String,
    pub paired_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointzProjection {
    pub server_id: String,
    pub import: PointzImport,
    pub devices: Vec<PointzDevice>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointzPlugin {
    Absent,
    Legacy,
    Compatible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointzPairing {
    pub open: bool,
    pub code: Option<String>,
    pub seconds_remaining: u32,
}

impl PointzPairing {
    pub const CLOSED: Self = Self {
        open: false,
        code: None,
        seconds_remaining: 0,
    };
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointzSocket {
    Discovery,
    Command,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PointzTransport {
    Stopped {},
    Running { dropped: u64 },
    PortBusy { socket: PointzSocket },
    Failed { socket: PointzSocket },
}
