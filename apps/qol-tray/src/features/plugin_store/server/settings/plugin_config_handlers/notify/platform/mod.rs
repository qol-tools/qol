#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

use super::super::super::super::types::AppState;

pub(super) fn notify_plugin_reload(state: &AppState, plugin_id: &str) {
    #[cfg(unix)]
    return unix::notify_plugin_reload(state, plugin_id);

    #[cfg(not(unix))]
    fallback::notify_plugin_reload(state, plugin_id)
}
