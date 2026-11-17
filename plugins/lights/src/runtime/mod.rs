pub mod actions;

use anyhow::{anyhow, Context, Result};

use crate::PLUGIN_ID;

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open settings URL")
}

pub fn entrypoint(args: Vec<String>) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return daemon_or_settings();
    };

    run_action(action)
}

fn daemon_or_settings() -> Result<()> {
    if std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return crate::daemon::run_from_env();
    }

    open_settings()
}

pub fn run_action(action: &str) -> Result<()> {
    if action == actions::SETTINGS {
        return open_settings();
    }
    if actions::is_supported_action(action) {
        return crate::daemon::execute_action_once(action);
    }
    Err(anyhow!("unknown action: {}", action))
}
