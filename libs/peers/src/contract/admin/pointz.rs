use serde::{Deserialize, Serialize};

use super::{ExpectedAuthority, PageCursor};
use crate::pointz::{PointzDeviceId, PointzImport, PointzPairing, PointzPlugin, PointzTransport};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PointzRequest {
    Status {},
    Devices {
        cursor: PageCursor,
    },
    BeginPairing {},
    CancelPairing {},
    Remove {
        expected: ExpectedAuthority,
        device_id: PointzDeviceId,
    },
}

impl PointzRequest {
    pub fn is_mutation(&self) -> bool {
        !matches!(self, Self::Status {} | Self::Devices { .. })
    }

    pub fn action_name(&self) -> &'static str {
        match self {
            Self::Status {} => "pointz_status",
            Self::Devices { .. } => "pointz_devices",
            Self::BeginPairing {} => "pointz_begin_pairing",
            Self::CancelPairing {} => "pointz_cancel_pairing",
            Self::Remove { .. } => "pointz_remove",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointzStatus {
    pub plugin: PointzPlugin,
    pub authority: Option<ExpectedAuthority>,
    pub migration: Option<PointzImport>,
    pub server_id: Option<String>,
    pub device_count: u32,
    pub pairing: PointzPairing,
    pub transport: PointzTransport,
}
