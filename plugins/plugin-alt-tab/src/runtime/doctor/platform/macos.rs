use super::{Inspection, PermissionInspection};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
}

pub(in crate::runtime::doctor) fn inspect() -> Inspection {
    Inspection {
        platform: "macos",
        supported: true,
        backend: "appkit-windowserver",
        display_ready: true,
        display_env_set: false,
        wayland_env_set: false,
        session_type: None,
    }
}

pub(in crate::runtime::doctor) fn inspect_permissions() -> PermissionInspection {
    PermissionInspection {
        platform: "macos",
        supported: true,
        accessibility_trusted: Some(unsafe { AXIsProcessTrusted() }),
        screen_recording_trusted: Some(unsafe { CGPreflightScreenCaptureAccess() }),
    }
}
