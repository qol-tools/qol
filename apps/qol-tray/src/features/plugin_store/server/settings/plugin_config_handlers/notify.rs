mod platform;

use super::super::super::types::AppState;

pub(super) fn notify_plugin_reload(state: &AppState, plugin_id: &str) {
    platform::notify_plugin_reload(state, plugin_id);
}
