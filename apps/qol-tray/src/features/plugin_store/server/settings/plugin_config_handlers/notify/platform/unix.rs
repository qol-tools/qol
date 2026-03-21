use crate::plugins::action_transport::{dispatch_daemon_action, DaemonActionDispatch};
use std::path::Path;

pub(super) fn notify_plugin_reload(socket_path: &str) -> bool {
    matches!(
        dispatch_daemon_action(Path::new(socket_path), "reload"),
        DaemonActionDispatch::Handled
    )
}
