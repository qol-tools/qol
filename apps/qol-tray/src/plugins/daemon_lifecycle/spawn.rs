use crate::plugins::manifest::DaemonConfig;
use crate::plugins::Plugin;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub(super) fn enabled_daemon(plugin: &Plugin) -> Option<&DaemonConfig> {
    plugin
        .manifest
        .daemon
        .as_ref()
        .filter(|daemon| daemon.enabled)
}

pub(super) fn spawn_daemon(
    plugin: &Plugin,
    daemon_config: &DaemonConfig,
    daemon_listener: Option<&super::DaemonListener>,
    runtime_config: Option<&mut crate::plugins::config::RuntimeConfigContext>,
) -> Result<(Child, u64)> {
    let profile_guard = materialize_runtime_config_with_context(plugin, runtime_config)?;
    let consumed_generation = profile_guard.generation();
    let daemon_path = daemon_path(plugin, daemon_config)?;
    let mut command = daemon_command(plugin, daemon_config, &daemon_path, daemon_listener);
    #[cfg(feature = "dev")]
    let relay_patterns = configure_log_relay(plugin, &mut command);
    #[cfg(not(feature = "dev"))]
    {
        command.stdout(Stdio::inherit());
        command.stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let version = plugin.manifest.plugin.version.clone();
        let commit = plugin.manifest.build.commit.clone();
        crate::logging::relay::attach_with_prod_log(
            plugin.id.as_str(),
            &version,
            commit.as_deref(),
            child.stderr.take(),
        );
        Ok((child, consumed_generation))
    }
    #[cfg(feature = "dev")]
    {
        let mut child = command.spawn()?;
        crate::logging::relay::attach(
            plugin.id.as_str(),
            child.stdout.take(),
            child.stderr.take(),
            relay_patterns,
        );
        Ok((child, consumed_generation))
    }
}

fn materialize_runtime_config(
    plugin: &Plugin,
) -> Result<crate::plugins::config::ProfileConfigReadGuard> {
    let profile_guard = crate::plugins::config::profile_config_read_guard();
    crate::plugins::PluginConfigManager::new()?
        .materialize_runtime_config_for_manifest(plugin.id.as_str(), &plugin.manifest)?;
    Ok(profile_guard)
}

fn materialize_runtime_config_with_context(
    plugin: &Plugin,
    runtime_config: Option<&mut crate::plugins::config::RuntimeConfigContext>,
) -> Result<crate::plugins::config::ProfileConfigReadGuard> {
    let Some(runtime_config) = runtime_config else {
        return materialize_runtime_config(plugin);
    };
    runtime_config.prepare_runtime_config_for_spawn(plugin.id.as_str(), &plugin.manifest)
}

fn daemon_path(plugin: &Plugin, daemon_config: &DaemonConfig) -> Result<PathBuf> {
    super::super::resolve_plugin_command_path_for_source(
        &plugin.path,
        &daemon_config.command,
        Some(&plugin.source),
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "Daemon executable not found for command {:?} in {:?}",
            daemon_config.command,
            plugin.path
        )
    })
}

fn daemon_command(
    plugin: &Plugin,
    daemon_config: &DaemonConfig,
    daemon_path: &Path,
    daemon_listener: Option<&super::DaemonListener>,
) -> Command {
    let mut command = Command::new(daemon_path);
    command.current_dir(&plugin.path).stdin(Stdio::null());
    command.env(qol_conventions::ENV_PLUGIN_ID, plugin.id.as_str());
    command.env("QOL_TRAY_PLUGIN_DIR", &plugin.path);
    crate::features::theme::apply_accent_env(&mut command);
    crate::features::theme::apply_theme_name_env(&mut command);
    apply_log_env(&mut command);
    apply_daemon_env(&mut command, daemon_config);
    apply_process_group(&mut command);
    if let Some(daemon_listener) = daemon_listener {
        super::listener::apply_to_command(daemon_listener, &mut command);
    }
    command
}

fn apply_daemon_env(command: &mut Command, daemon_config: &DaemonConfig) {
    if let Some(socket) = daemon_config.socket.as_deref() {
        command.env(
            qol_conventions::ENV_DAEMON_SOCKET,
            crate::dev_generation::daemon_socket_path(socket),
        );
    }
    command.env(qol_conventions::ENV_DAEMON_REPLACE_EXISTING, "1");
    command.env(
        qol_conventions::ENV_STATE_SOCKET,
        crate::dev_generation::state_socket_path(),
    );
    if let Some(token) = crate::features::plugin_store::server::security::current_token() {
        command.env(qol_conventions::ENV_HTTP_TOKEN, token);
    }
    command.env_remove("XMODIFIERS");
}

fn apply_log_env(command: &mut Command) {
    #[cfg(feature = "dev")]
    command.env("RUST_LOG", "debug");

    #[cfg(not(feature = "dev"))]
    command.env("RUST_LOG", "warn");
}

fn apply_process_group(command: &mut Command) {
    super::platform::configure_process_group(command);
}

#[cfg(feature = "dev")]
fn configure_log_relay(plugin: &Plugin, command: &mut Command) -> Vec<String> {
    let log_control = crate::logging::load_plugin_control_from_shared_config(plugin.id.as_str());
    if log_control.muted {
        command.stdout(Stdio::null()).stderr(Stdio::null());
        return Vec::new();
    }
    // Never Stdio::inherit() here: the tray's own stdio is a pipe into qol
    // dev, which dies at every generation handoff. A daemon inheriting it
    // EPIPE-panics on its next write, and inside extern-C frames (gpui's
    // launch callback, event-tap callbacks) that panic cannot unwind and
    // aborts the daemon. The piped relay is read by the tray, which outlives
    // the handoff, so daemon writes always succeed.
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    log_control.suppress_patterns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{PluginId, PluginManifest};
    use serde_json::json;
    use std::ffi::OsStr;
    use std::path::Path;
    use tempfile::TempDir;

    fn daemon_config() -> crate::plugins::manifest::DaemonConfig {
        crate::plugins::manifest::DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: None,
            port: None,
            extra_ports: Vec::new(),
        }
    }

    fn minimal_plugin(root: &std::path::Path) -> Plugin {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
id = "plugin-launcher"
uid = "uid-launcher"
name = "Launcher"
description = ""
version = "1.0.0"

[menu]
label = "Launcher"
items = []
"#,
        )
        .unwrap();
        Plugin::new(
            PluginId::new("plugin-launcher"),
            manifest,
            root.join("plugin-launcher"),
        )
    }

    #[test]
    fn daemon_command_injects_saved_theme_accent() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        crate::features::theme::save_selected_accent_key("blue").unwrap();
        let plugin = minimal_plugin(root.path());
        let command = daemon_command(&plugin, &daemon_config(), Path::new("/bin/true"), None);

        let (_, value) = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(qol_conventions::ENV_THEME_ACCENT))
            .expect("daemon spawns must inherit the selected tray accent");
        assert_eq!(value, Some(OsStr::new("blue")));
    }

    #[test]
    fn apply_daemon_env_clears_xmodifiers_to_disable_gpui_xim_client() {
        let mut command = Command::new("/bin/true");
        command.env("XMODIFIERS", "@im=ibus");
        apply_daemon_env(&mut command, &daemon_config());

        let entry = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("XMODIFIERS"));
        let (_, value) = entry.expect(
            "XMODIFIERS must appear in get_envs as an explicit removal so children do not inherit it",
        );
        assert!(
            value.is_none(),
            "XMODIFIERS must be cleared, not overridden with a value",
        );
    }

    #[test]
    fn materialize_runtime_config_uses_loaded_manifest_uid_before_spawn() {
        let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let store = crate::features::profile::ProfileScopeStore::from_active().unwrap();
        let expected = json!({"display": {"transparent_background": true}});
        let profile_path = store.core_plugin_configs_dir().join("uid-alt-tab.json");
        std::fs::create_dir_all(profile_path.parent().unwrap()).unwrap();
        std::fs::write(&profile_path, serde_json::to_string(&expected).unwrap()).unwrap();
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
id = "plugin-alt-tab"
uid = "uid-alt-tab"
name = "Alt Tab"
description = ""
version = "1.0.0"

[menu]
label = "Alt Tab"
items = []
"#,
        )
        .unwrap();
        let plugin = Plugin::new(
            PluginId::new("plugin-alt-tab"),
            manifest,
            root.path().join("plugin-alt-tab"),
        );

        materialize_runtime_config(&plugin).unwrap();

        let runtime = crate::paths::plugins_dir()
            .unwrap()
            .join("plugin-alt-tab")
            .join("config.json");
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(runtime).unwrap()).unwrap();
        assert_eq!(value, expected);
    }
}
