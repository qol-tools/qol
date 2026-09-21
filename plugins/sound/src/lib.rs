pub mod app;
pub mod cli;
pub mod config;
pub mod doctor;
pub mod output;
pub mod volume;

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
pub const BINARY_NAME: &str = "plugin-sound";
