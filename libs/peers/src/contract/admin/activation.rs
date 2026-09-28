use std::{fmt, str::FromStr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActivationId([u8; 16]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("activation identifier must be canonical unpadded base64url encoding of 16 bytes")]
pub struct ActivationIdError;

impl ActivationId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for ActivationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl FromStr for ActivationId {
    type Err = ActivationIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 22 {
            return Err(ActivationIdError);
        }
        let bytes: [u8; 16] = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| ActivationIdError)?
            .try_into()
            .map_err(|_| ActivationIdError)?;
        if URL_SAFE_NO_PAD.encode(bytes) != value {
            return Err(ActivationIdError);
        }
        Ok(Self(bytes))
    }
}

impl Serialize for ActivationId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ActivationId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
