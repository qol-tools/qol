pub mod cli;
pub mod config;
pub mod daemon;
pub mod display_color;
pub mod host_night_light;
pub mod hotkeys;
pub mod monitor;
pub mod platform;
pub mod session;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
