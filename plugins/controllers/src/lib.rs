pub mod app;
pub mod cli;
pub mod detection;
pub mod fixes;
pub mod platform;

pub use app as daemon;
pub use detection as detect;
pub use fixes::{apply, state};

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
