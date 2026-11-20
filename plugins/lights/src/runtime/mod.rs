pub mod actions;

use anyhow::{anyhow, Context, Result};

use crate::PLUGIN_ID;

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open settings URL")
}

pub fn entrypoint(args: Vec<String>) -> Result<()> {
    if qol_plugin_daemon::daemon::launched_as_daemon(&args) {
        return crate::daemon::run_from_env();
    }
    let Some(action) = args.first().map(String::as_str) else {
        return open_settings();
    };

    run_action(action)
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
