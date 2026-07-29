use crate::preview_plane::PreviewPlanePayload;

pub(crate) fn prepare() {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_disables_live_preview_replacement() {
        assert_eq!(live_preview_replacement(), None);
    }
}
