use crate::plugins::manifest::DaemonConfig;
use crate::plugins::Plugin;
use std::process::Command;

#[derive(Debug)]
pub(in crate::plugins) struct DaemonListener;

pub(in crate::plugins::daemon_lifecycle) fn bind_for_plugin(
    _plugin: &Plugin,
    _daemon_config: &DaemonConfig,
) -> Option<DaemonListener> {
    None
}

pub(in crate::plugins::daemon_lifecycle) fn refresh_for_respawn(
    _daemon_listener: DaemonListener,
    _plugin: &Plugin,
    _daemon_config: &DaemonConfig,
) -> Option<DaemonListener> {
    None
}

pub(in crate::plugins::daemon_lifecycle) fn apply_to_command(
    _daemon_listener: &DaemonListener,
    _command: &mut Command,
) {
}
