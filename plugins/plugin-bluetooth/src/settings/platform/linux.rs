use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

pub(crate) fn run_panel() -> Result<()> {
    let runtime = qol_gpui::settings_panel::SettingsRuntime::new(crate::platform::settings_query)
        .with_input_action(crate::platform::settings_action)
        .poll_every(Duration::from_millis(500));
    qol_gpui::settings_panel::run_plugin_settings(
        crate::PLUGIN_ID,
        "Bluetooth Settings",
        crate::config::contract(),
        runtime,
    )
}

pub(crate) fn spawn_panel() -> Result<()> {
    let executable = std::env::current_exe().context("failed to locate Bluetooth executable")?;
    let mut command = Command::new(executable);
    command.arg(crate::SETTINGS_SURFACE_ARG);
    qol_process::spawn_detached(&mut command).context("failed to launch native Bluetooth settings")
}
