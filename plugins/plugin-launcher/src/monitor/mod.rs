mod platform;
mod poller;
pub(crate) mod state;
mod tracker;

pub use state::ActiveMonitor;
pub use tracker::{MonitorConfig, MonitorTracker};
