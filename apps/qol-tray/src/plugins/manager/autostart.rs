use crate::plugins::resolver::PluginSource;
use crate::plugins::Plugin;
use std::path::Path;

const DEV_DAEMON_AUTOSTART_MARKER: &str = ".qol-tray-dev-autostart";

pub(super) fn start_plugin_daemons<'a, I>(plugins: I)
where
    I: IntoIterator<Item = &'a mut Plugin>,
{
    let mut expected_lifelines = Vec::new();
    for plugin in plugins {
        if !should_start_daemon(plugin) {
            continue;
        }
        if daemon_enabled(plugin) {
            expected_lifelines.push(plugin.id.as_str().to_string());
        }
        start_daemon(plugin);
    }
    audit_host_death_lifelines(expected_lifelines);
}

fn audit_host_death_lifelines(expected: Vec<String>) {
    if expected.is_empty() {
        return;
    }
    std::thread::Builder::new()
        .name("qol-lifeline-audit".into())
        .spawn(move || {
            let missing = super::lifeline_facade::settle_missing_lifelines(&expected);
            if missing.is_empty() {
                log::debug!(
                    "host-death watchdog: all {} daemon(s) armed a lifeline",
                    expected.len()
                );
                return;
            }
            for id in missing {
                log::error!(
                    "host-death watchdog: daemon '{id}' did NOT arm a host-death lifeline and \
                     will orphan (leak) if qol-tray is force-quit. Its daemon entry must call \
                     qol_runtime::spawn_host_death_watchdog (qol_plugin_daemon::daemon::start_listener \
                     does this automatically)."
                );
            }
        })
        .ok();
}

fn should_start_daemon(plugin: &Plugin) -> bool {
    let daemon_enabled = daemon_enabled(plugin);
    if !daemon_enabled {
        return true;
    }
    should_autostart_daemon_for_source(
        plugin.id.as_str(),
        &plugin.path,
        daemon_enabled,
        Some(&plugin.source),
    )
}

fn daemon_enabled(plugin: &Plugin) -> bool {
    plugin
        .manifest
        .daemon
        .as_ref()
        .is_some_and(|daemon| daemon.enabled)
}

fn start_daemon(plugin: &mut Plugin) {
    if let Err(error) = plugin.start_daemon() {
        log::error!("Failed to start daemon for plugin {}: {}", plugin.id, error);
    }
}

fn should_autostart_daemon_for_source(
    plugin_id: &str,
    plugin_path: &Path,
    daemon_enabled: bool,
    source: Option<&PluginSource>,
) -> bool {
    if !daemon_enabled {
        return true;
    }
    if !source.is_some_and(PluginSource::is_live_source) {
        return true;
    }

    let marker_path = plugin_path.join(DEV_DAEMON_AUTOSTART_MARKER);
    if marker_path.is_file() {
        return true;
    }

    log::warn!(
        "Daemon autostart blocked for dev-linked plugin {} at {}. Create {} to opt in.",
        plugin_id,
        plugin_path.display(),
        marker_path.display()
    );
    false
}

#[cfg(test)]
mod tests {
    use super::{should_autostart_daemon_for_source, DEV_DAEMON_AUTOSTART_MARKER};
    use crate::plugins::resolver::PluginSource;

    #[test]
    fn allows_installed_plugin_daemon_autostart() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            true,
            Some(&PluginSource::Installed),
        ));
    }

    #[test]
    fn allows_dev_linked_plugin_when_daemon_disabled() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            false,
            Some(&PluginSource::DevLinked),
        ));
    }

    #[test]
    fn blocks_dev_linked_plugin_daemon_autostart_without_marker() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(!should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            true,
            Some(&PluginSource::DevLinked),
        ));
    }

    #[test]
    fn allows_dev_linked_plugin_daemon_autostart_with_marker() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join(DEV_DAEMON_AUTOSTART_MARKER), "").unwrap();
        assert!(should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            true,
            Some(&PluginSource::DevLinked),
        ));
    }
}
