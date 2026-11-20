pub mod cli;
pub mod config;
pub mod core;
pub mod daemon;
mod doctor;
pub mod ui;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
