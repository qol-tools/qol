use std::fmt;

use qol_conventions::operations::OperationKey;
use serde::{Deserialize, Deserializer, Serialize};

use super::{EnrollmentFailure, ExpectedAuthority};
use crate::PeerId;

pub const MAX_NEARBY: usize = 16;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum NearbyRequest {
    List {},
    Link {
        expected: ExpectedAuthority,
        peer_id: PeerId,
    },
    Confirm {
        expected: ExpectedAuthority,
        peer_id: PeerId,
        grants: Vec<OperationKey>,
    },
    Decline {
        expected: ExpectedAuthority,
        peer_id: PeerId,
    },
}

impl NearbyRequest {
    pub fn is_mutation(&self) -> bool {
        !matches!(self, Self::List {})
    }

    pub fn action_name(&self) -> &'static str {
        match self {
            Self::List {} => "nearby",
            Self::Link { .. } => "nearby_link",
            Self::Confirm { .. } => "nearby_confirm",
            Self::Decline { .. } => "nearby_decline",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LinkCode(u32);

impl LinkCode {
    pub fn new(value: u32) -> Option<Self> {
        (value < 1_000_000).then_some(Self(value))
    }

    pub fn value(self) -> u32 {
        self.0
    }

    #[cfg(feature = "service")]
    pub(crate) fn reduce(value: u32) -> Self {
        Self(value % 1_000_000)
    }
}

impl fmt::Display for LinkCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03} {:03}", self.0 / 1000, self.0 % 1000)
    }
}

impl<'de> Deserialize<'de> for LinkCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u32::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("link code must be below one million"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NearbyComputer {
    pub peer_id: PeerId,
    #[serde(deserialize_with = "crate::enrollment::deserialize_name")]
    pub name: String,
    pub link: Option<NearbyLink>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NearbyLink {
    pub code: Option<LinkCode>,
    pub state: NearbyState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum NearbyState {
    Connecting {},
    Confirm {},
    WaitingForPeer {},
    Failed { error: EnrollmentFailure },
}
