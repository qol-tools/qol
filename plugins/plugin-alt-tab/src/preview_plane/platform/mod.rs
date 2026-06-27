#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{hide_async, live_preview_replacement, show_async};
#[cfg(target_os = "macos")]
pub(crate) use macos::{hide_async, live_preview_replacement, show_async};
#[cfg(target_os = "windows")]
pub(crate) use windows::{hide_async, live_preview_replacement, show_async};

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported {
    use crate::preview_plane::PreviewPlanePayload;

    pub(crate) fn show_async(payload: PreviewPlanePayload) {
        qol_runtime::probe!(
            "PREVIEW_PLANE_SHOW",
            "show_id={} outcome=skipped reason=unsupported_platform items={}",
            payload.show_id,
            payload.items.len()
        );
    }

    pub(crate) fn hide_async(reason: &'static str) {
        qol_runtime::probe!(
            "PREVIEW_PLANE_HIDE",
            "reason={reason} outcome=skipped reason=unsupported_platform"
        );
    }

    pub(crate) fn live_preview_replacement() -> Option<&'static str> {
        None
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use unsupported::{hide_async, live_preview_replacement, show_async};
