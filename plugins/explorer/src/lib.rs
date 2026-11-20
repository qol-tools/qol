pub mod cli;
pub mod config;
pub mod listing;
pub mod nav;
pub mod ops;
pub mod places;
pub mod props;
pub mod relay;
pub mod thumbs;
pub mod ui;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
