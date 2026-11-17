pub mod app;
pub mod cli;
mod pages;
mod session;
mod updates;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
