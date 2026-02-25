use gpui::*;

pub(crate) fn open_keepalive(cx: &mut App) {
    qol_plugin_api::keepalive::open_keepalive(cx, None);
}
