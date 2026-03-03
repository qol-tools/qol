use gpui::*;

use super::LAUNCHER_APP_ID;

pub(crate) fn open_keepalive_window(cx: &mut App) {
    qol_plugin_api::keepalive::open_keepalive(cx, Some(LAUNCHER_APP_ID));
}
