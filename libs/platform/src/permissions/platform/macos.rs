use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};

use super::{Permission, PermissionState, PermissionsApi};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

pub(crate) struct Permissions;

impl PermissionsApi for Permissions {
    fn status(&self, permission: Permission) -> PermissionState {
        state(
            permission,
            match permission {
                Permission::InputCapture => unsafe { AXIsProcessTrusted() },
                Permission::ScreenCapture => unsafe { CGPreflightScreenCaptureAccess() },
            },
        )
    }

    fn request(&self, permission: Permission) -> PermissionState {
        state(
            permission,
            match permission {
                Permission::InputCapture => {
                    let options = prompt_options();
                    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
                }
                Permission::ScreenCapture => unsafe { CGRequestScreenCaptureAccess() },
            },
        )
    }
}

fn state(permission: Permission, granted: bool) -> PermissionState {
    if granted {
        return PermissionState::Granted;
    }
    PermissionState::Denied {
        grant_at: match permission {
            Permission::InputCapture => "System Settings > Privacy & Security > Accessibility",
            Permission::ScreenCapture => {
                "System Settings > Privacy & Security > Screen & System Audio Recording"
            }
        },
    }
}

fn prompt_options() -> CFDictionary<CFType, CFType> {
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())])
}
