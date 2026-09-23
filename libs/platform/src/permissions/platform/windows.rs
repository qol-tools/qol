use super::{Permission, PermissionState, PermissionsApi};

pub(crate) struct Permissions;

impl PermissionsApi for Permissions {
    fn status(&self, _permission: Permission) -> PermissionState {
        PermissionState::NotRequired {
            because: "Windows gates neither global input hooks nor screen capture",
        }
    }

    fn request(&self, permission: Permission) -> PermissionState {
        self.status(permission)
    }
}
