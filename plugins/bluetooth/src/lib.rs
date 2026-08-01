pub mod bluetooth;
pub mod cli;
pub mod config;
pub mod hostfix;
pub mod platform;
mod settings;

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
pub const SETTINGS_SURFACE_ARG: &str = "__qol-settings-surface";

pub fn show_settings() -> anyhow::Result<()> {
    if let Err(error) = settings::run_panel() {
        eprintln!("[bluetooth] native settings unavailable, opening browser: {error:#}");
        return settings::open_browser();
    }
    Ok(())
}
