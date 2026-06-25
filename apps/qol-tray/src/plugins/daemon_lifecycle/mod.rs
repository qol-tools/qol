mod spawn;

use super::Plugin;
use anyhow::Result;
use std::process::Child;
use std::time::Duration;

const DAEMON_STOP_GRACE: Duration = Duration::from_secs(2);

pub(super) fn start_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(daemon_config) = spawn::enabled_daemon(plugin) else {
        return Ok(());
    };

    let child = spawn::spawn_daemon(plugin, daemon_config)?;
    register_daemon(plugin, child);
    Ok(())
}

pub(super) fn stop_daemon(plugin: &mut Plugin) -> Result<()> {
    let Some(mut child) = plugin.daemon_process.take() else {
        return Ok(());
    };

    log::info!("Stopping daemon for plugin: {}", plugin.id);
    super::daemon_tracker::registry::unregister(
        &crate::paths::runtime_pids_dir(),
        plugin.id.as_str(),
        child.id(),
    );
    if let Err(error) = crate::process_utils::terminate_owned(&mut child, DAEMON_STOP_GRACE) {
        log::warn!("Error reaping daemon for {}: {}", plugin.id, error);
    }
    Ok(())
}

fn register_daemon(plugin: &mut Plugin, child: Child) {
    let pid = child.id();
    plugin.daemon_process = Some(child);
    #[cfg(unix)]
    crate::desktop_state::add_ignore_pid(pid);
    super::daemon_tracker::registry::register(
        &crate::paths::runtime_pids_dir(),
        plugin.id.as_str(),
        pid,
    );
    log::info!("Registered ignore pid {} for plugin {}", pid, plugin.id);
}
