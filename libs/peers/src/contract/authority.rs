use std::{fmt, str::FromStr};

use qol_conventions::operations::OperationKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::PeerId;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct StoreRevision(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("store revision must be a canonical decimal u64 string")]
pub struct StoreRevisionError;

impl StoreRevision {
    pub const INITIAL: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for StoreRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for StoreRevision {
    type Err = StoreRevisionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(StoreRevisionError);
        }
        value.parse().map(Self).map_err(|_| StoreRevisionError)
    }
}

impl Serialize for StoreRevision {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for StoreRevision {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

pub const MAX_NAME_BYTES: usize = 256;

pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.trim() == name
        && !name.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityLifetime {
    Session,
    Persistent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityStatus {
    Ready,
    Faulted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerProjection {
    pub peer_id: PeerId,
    pub name: String,
    pub grants: Vec<OperationKey>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityProjection {
    pub peer_id: PeerId,
    pub name: String,
    pub lifetime: AuthorityLifetime,
    pub revision: StoreRevision,
    pub status: AuthorityStatus,
    pub peers: Vec<PeerProjection>,
    pub tombstones: Vec<PeerId>,
}
