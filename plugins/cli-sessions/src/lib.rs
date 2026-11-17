pub mod attention;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod diagnostics;
mod doctor;
pub mod fork_watch;
pub mod host;
pub mod park;
mod parking;
pub mod session;
pub mod signal;
pub mod storage;
pub mod ui;

pub use diagnostics::{anomaly, snapshot};
pub use session::{git, registry, service, status, tool};
pub use storage::{paths, persist};
pub use ui::{collapse, nav, notify, placement, selection};

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
