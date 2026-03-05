mod log_relay;
mod readiness;
mod spawn;

use super::Plugin;
use anyhow::Result;
use std::process::Child;

pub(super) fn start_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(daemon_config) = spawn::enabled_daemon(plugin) else {
        return Ok(());
    };

    let mut child = spawn::spawn_daemon(plugin, daemon_config)?;
    readiness::wait_for_daemon_ready(plugin, daemon_config, &mut child)?;
    register_daemon(plugin, child);
    Ok(())
}

pub(super) fn stop_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(mut child) = plugin.daemon_process.take() else {
        return Ok(());
    };

    log::info!("Stopping daemon for plugin: {}", plugin.id);
    crate::signal::unregister_daemon_pid(child.id());
    readiness::terminate_daemon(&mut child);
    readiness::wait_for_exit(plugin, &mut child)
}

fn register_daemon(plugin: &mut Plugin, child: Child) {
    let pid = child.id();
    plugin.daemon_process = Some(child);
    #[cfg(unix)]
    crate::os::display::add_ignore_pid(pid);
    crate::signal::register_daemon_pid(pid);
    log::info!("Registered ignore pid {} for plugin {}", pid, plugin.id);
}
