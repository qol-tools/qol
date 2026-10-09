pub mod app;
pub mod cli;
pub mod config;
pub mod device;
pub mod doctor;
pub mod input;
pub mod levels;
pub mod mute;
pub mod output;
pub mod volume;

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
