pub mod apply;
pub mod cli;
pub mod daemon;
pub mod detect;
pub mod fixes;
pub mod platform;
pub mod state;

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
