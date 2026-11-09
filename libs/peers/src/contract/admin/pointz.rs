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
    Serve {
        on: bool,
    },
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
            Self::Serve { .. } => "pointz_serve",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_is_a_named_mutation_on_the_wire() {
        let request = PointzRequest::Serve { on: false };
        assert!(request.is_mutation());
        assert_eq!(request.action_name(), "pointz_serve");
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire, serde_json::json!({"action": "serve", "on": false}));
        assert_eq!(
            serde_json::from_value::<PointzRequest>(wire).unwrap(),
            request
        );
    }
}
