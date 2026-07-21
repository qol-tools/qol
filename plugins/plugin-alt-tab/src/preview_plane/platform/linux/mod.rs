use crate::preview_plane::PreviewPlanePayload;

mod cinnamon_shell;

pub(crate) fn show_async(payload: PreviewPlanePayload) {
    if let Some(reason) = cinnamon_shell::disabled_reason() {
        qol_runtime::probe!(
            "PREVIEW_PLANE_SHOW",
            "show_id={} outcome=skipped reason={reason} items={}",
            payload.show_id,
            payload.items.len()
        );
        return;
    }
    cinnamon_shell::show_async(payload);
}

pub(crate) fn hide_async(reason: &'static str) {
    if let Some(disabled_reason) = cinnamon_shell::disabled_reason() {
        qol_runtime::probe!(
            "PREVIEW_PLANE_HIDE",
            "reason={reason} outcome=skipped reason={disabled_reason}"
        );
        return;
    }
    cinnamon_shell::hide_async(reason);
}

pub(crate) fn live_preview_replacement() -> Option<&'static str> {
    cinnamon_shell::live_preview_replacement()
}
