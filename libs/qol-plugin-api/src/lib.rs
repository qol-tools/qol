pub mod capability;
pub mod manifest;
pub mod restore;

pub use manifest::PluginId;
pub use restore::{ForegroundProc, PaneSnapshot, RestoreClaim};
