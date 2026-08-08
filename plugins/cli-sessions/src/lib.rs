use anyhow::Context as _;

pub mod cli;
pub mod config;
pub mod daemon;
pub mod diagnostics;
mod doctor;
pub mod host;
pub mod session;
pub mod signal;
pub mod storage;
pub mod strategy;
pub mod ui;

pub use diagnostics::{anomaly, snapshot};
pub use session::{git, registry, service, status, tool};
pub use storage::{paths, persist};
pub use ui::{nav, notify, placement, selection};

pub fn show_settings() -> anyhow::Result<()> {
    let runtime = qol_gpui::settings_panel::SettingsRuntime::empty();
    if let Err(error) = qol_gpui::settings_panel::run_plugin_settings(
        crate::storage::paths::PLUGIN_ID,
        "CLI Sessions Settings",
        crate::config::contract(),
        runtime,
    ) {
        eprintln!("[cli-sessions] native settings unavailable, opening browser: {error:#}");
        return qol_apps::desktop_integration::open_plugin_settings(
            crate::storage::paths::PLUGIN_ID,
        )
        .context("failed to open CLI Sessions settings URL");
    }
    Ok(())
}
