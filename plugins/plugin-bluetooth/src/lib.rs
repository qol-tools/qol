pub mod bluetooth;
pub mod cli;
pub mod config;
pub mod platform;
pub mod retry;

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
pub const SETTINGS_SURFACE_ARG: &str = "__qol-settings-surface";

pub fn show_settings() -> anyhow::Result<()> {
    if let Err(error) = platform::run_settings_panel() {
        eprintln!("[bluetooth] native settings unavailable, opening browser: {error:#}");
        return platform::open_settings();
    }
    Ok(())
}
