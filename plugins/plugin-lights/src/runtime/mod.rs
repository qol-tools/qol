pub mod actions;

use anyhow::{anyhow, Result};

pub fn entrypoint(args: Vec<String>) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return daemon_or_settings();
    };

    run_action(action)
}

fn daemon_or_settings() -> Result<()> {
    if std::env::var_os("QOL_TRAY_DAEMON_SOCKET").is_some() {
        return crate::daemon::run_from_env();
    }

    crate::platform::open_settings()
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
