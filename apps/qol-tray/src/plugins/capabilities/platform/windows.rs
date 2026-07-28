use super::PermissionPlatform;
use crate::plugins::capabilities::PermissionStatus;
use crate::plugins::manifest::Capabilities;
use std::collections::HashMap;

pub(super) struct Platform;

impl PermissionPlatform for Platform {
    fn check_plugin_permissions(_capabilities: &Capabilities) -> HashMap<String, PermissionStatus> {
        HashMap::new()
    }

    fn check_permission(_name: &str) -> Option<PermissionStatus> {
        None
    }

    fn request_permission(_name: &str) -> Option<PermissionStatus> {
        None
    }
}
