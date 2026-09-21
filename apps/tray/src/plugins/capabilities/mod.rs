mod platform;

use crate::plugins::manifest::Capabilities;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Fixable,
    RequiresLogout,
    Denied,
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionStatus {
    pub state: PermissionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

pub fn check_plugin_permissions(capabilities: &Capabilities) -> HashMap<String, PermissionStatus> {
    platform::check_plugin_permissions(capabilities)
}

pub fn check_permission(name: &str) -> Option<PermissionStatus> {
    platform::check_permission(name)
}

pub fn request_permission(name: &str) -> Option<PermissionStatus> {
    platform::request_permission(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_state_serializes_as_snake_case() {
        let cases = [
            (PermissionState::Granted, "\"granted\""),
            (PermissionState::Fixable, "\"fixable\""),
            (PermissionState::RequiresLogout, "\"requires_logout\""),
            (PermissionState::Denied, "\"denied\""),
        ];
        for (state, expected) in cases {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, expected, "state: {:?}", state);
        }
    }

    #[test]
    fn permission_status_omits_null_hint() {
        let status = PermissionStatus {
            state: PermissionState::Granted,
            hint: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"state":"granted"}"#);
    }

    #[test]
    fn permission_status_includes_hint_when_present() {
        let status = PermissionStatus {
            state: PermissionState::Fixable,
            hint: Some(String::from("foo")),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"state":"fixable","hint":"foo"}"#);
    }
}
