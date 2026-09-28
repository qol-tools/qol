pub mod admin;
mod authority;
mod authority_error;
pub mod enrollment;
pub mod session;

pub use authority_error::AuthorityError;

pub use authority::{
    AuthorityLifetime, AuthorityProjection, AuthorityStatus, PeerProjection, StoreRevision,
    StoreRevisionError,
};

use std::{fmt, str::FromStr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerId([u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("peer identifier must be canonical unpadded base64url encoding of 32 bytes")]
pub struct PeerIdError;

impl PeerId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[cfg(feature = "service")]
    pub(crate) fn from_spki_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }
}

impl FromStr for PeerId {
    type Err = PeerIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 43 {
            return Err(PeerIdError);
        }
        let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| PeerIdError)?;
        let bytes = decoded.try_into().map_err(|_| PeerIdError)?;
        if URL_SAFE_NO_PAD.encode(bytes) != value {
            return Err(PeerIdError);
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PeerId").field(&self.to_string()).finish()
    }
}

impl Serialize for PeerId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PeerId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

pub mod network;

pub mod operations;
