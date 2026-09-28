use std::{fmt, str::FromStr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::PeerId;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionNonce([u8; 16]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("session nonce must be canonical base64url encoding of 16 bytes")]
pub struct SessionNonceError;

impl SessionNonce {
    #[cfg(feature = "service")]
    pub(crate) fn from_random(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for SessionNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl FromStr for SessionNonce {
    type Err = SessionNonceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 22 {
            return Err(SessionNonceError);
        }
        let bytes: [u8; 16] = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| SessionNonceError)?
            .try_into()
            .map_err(|_| SessionNonceError)?;
        if URL_SAFE_NO_PAD.encode(bytes) != value {
            return Err(SessionNonceError);
        }
        Ok(Self(bytes))
    }
}

impl Serialize for SessionNonce {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SessionNonce {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionGeneration {
    pub local: SessionNonce,
    pub remote: SessionNonce,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedSession {
    pub local_peer: PeerId,
    pub remote_peer: PeerId,
    pub generation: SessionGeneration,
}
