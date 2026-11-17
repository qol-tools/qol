mod daemon;
mod daemon_run;

use anyhow::{Context, Result};

pub fn run() -> Result<()> {
    daemon_run::run()
}

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(crate::PLUGIN_ID)
        .context("failed to open settings URL")
}

pub fn toggle_theme() -> Result<crate::theme::ColorScheme> {
    crate::theme::toggle()
}

pub fn kill() -> Result<()> {
    daemon::send_kill();
    Ok(())
}
