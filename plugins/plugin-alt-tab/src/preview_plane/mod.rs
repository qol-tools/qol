use serde::Serialize;

mod platform;

const DEFAULT_TTL_MS: u64 = 30_000;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PreviewPlanePayload {
    pub(crate) show_id: String,
    pub(crate) ttl_ms: u64,
    pub(crate) backdrop: bool,
    pub(crate) chrome: bool,
    pub(crate) items: Vec<PreviewPlaneItem>,
}

impl PreviewPlanePayload {
    pub(crate) fn new(show_id: impl Into<String>, items: Vec<PreviewPlaneItem>) -> Self {
        Self {
            show_id: show_id.into(),
            ttl_ms: DEFAULT_TTL_MS,
            backdrop: false,
            chrome: false,
            items,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PreviewPlaneItem {
    pub(crate) wid: u32,
    pub(crate) selected: bool,
    pub(crate) title: String,
    pub(crate) rect: PreviewPlaneRect,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct PreviewPlaneRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

pub(crate) fn show_async(payload: PreviewPlanePayload) {
    platform::show_async(payload);
}

pub(crate) fn hide_async(reason: &'static str) {
    platform::hide_async(reason);
}

pub(crate) fn live_preview_replacement() -> Option<&'static str> {
    platform::live_preview_replacement()
}
