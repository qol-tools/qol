pub mod actions;

use anyhow::{anyhow, Result};

pub fn entrypoint(args: Vec<String>) -> Result<()> {
    if qol_plugin_daemon::daemon::launched_as_daemon(&args) {
        return crate::daemon::run_from_env();
    }
    let Some(action) = args.first().map(String::as_str) else {
        return crate::platform::open_settings();
    };

    run_action(action)
}

pub fn run_action(action: &str) -> Result<()> {
    if action == actions::SETTINGS {
        return crate::platform::open_settings();
    }
    if actions::is_supported_action(action) {
        return crate::daemon::execute_action_once(action);
    }
    Err(anyhow!("unknown action: {}", action))
}
