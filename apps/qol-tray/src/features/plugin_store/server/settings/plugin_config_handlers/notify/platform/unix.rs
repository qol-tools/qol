use crate::plugins::action_transport::{dispatch_daemon_action, DaemonActionDispatch};
use std::path::Path;

pub(in crate::features::plugin_store::server::settings::plugin_config_handlers::notify) fn notify_plugin_reload(
    socket_path: &str,
) -> bool {
    matches!(
        dispatch_daemon_action(Path::new(socket_path), "reload"),
        DaemonActionDispatch::Handled { .. }
    )
}
