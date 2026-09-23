use super::{Permission, PermissionState, PermissionsApi};

const NOT_INSPECTED: &str = "qol-platform cannot inspect input or screen capture grants here";

pub(crate) struct Permissions;

impl PermissionsApi for Permissions {
    fn status(&self, _permission: Permission) -> PermissionState {
        PermissionState::Unknown {
            because: NOT_INSPECTED,
        }
    }

    fn request(&self, permission: Permission) -> PermissionState {
        self.status(permission)
    }
}
