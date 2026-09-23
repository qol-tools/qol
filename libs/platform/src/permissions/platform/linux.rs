use crate::LinuxDisplayBackend;

use super::{Permission, PermissionState, PermissionsApi};

pub(crate) struct Permissions;

impl PermissionsApi for Permissions {
    fn status(&self, _permission: Permission) -> PermissionState {
        match crate::linux_display_backend() {
            LinuxDisplayBackend::X11 => PermissionState::NotRequired {
                because: "an X11 session gates neither input nor screen capture",
            },
            LinuxDisplayBackend::Wayland => PermissionState::Unknown {
                because: "Wayland grants this per portal request, so there is no standing answer",
            },
            LinuxDisplayBackend::Unknown => PermissionState::Unknown {
                because: "the Linux display backend could not be determined",
            },
        }
    }

    fn request(&self, permission: Permission) -> PermissionState {
        self.status(permission)
    }
}
