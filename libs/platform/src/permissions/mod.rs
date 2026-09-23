mod platform;

use serde::{Deserialize, Serialize};

use platform::{Permissions, PermissionsApi};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    InputCapture,
    ScreenCapture,
}

impl Permission {
    pub fn label(self) -> &'static str {
        match self {
            Self::InputCapture => "input capture",
            Self::ScreenCapture => "screen capture",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionState {
    Granted,
    Denied { grant_at: &'static str },
    NotRequired { because: &'static str },
    Unknown { because: &'static str },
}

impl PermissionState {
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Granted | Self::NotRequired { .. })
    }

    pub fn granted(self) -> Option<bool> {
        match self {
            Self::Granted => Some(true),
            Self::Denied { .. } => Some(false),
            Self::NotRequired { .. } | Self::Unknown { .. } => None,
        }
    }
}

pub fn permission_status(permission: Permission) -> PermissionState {
    Permissions.status(permission)
}

/// Asking is what registers the binary with macOS: a process that only ever
/// reads the status never appears in System Settings, so the user has nothing
/// to enable, and the grant belongs to whichever executable asked. `ScreenCapture`
/// blocks until the dialog is answered.
pub fn request_permission(permission: Permission) -> PermissionState {
    Permissions.request(permission)
}
