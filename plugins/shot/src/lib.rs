pub mod app;
mod capture;
pub mod cli;
mod config;
pub(crate) mod platform;
mod ui;

pub use capture::geometry::{Monitor, Rect};
pub use capture::{recording, screenshot};
pub use config::{AudioConfig, CaptureConfig, Config, CopyCommand, ShortcutsConfig, VideoConfig};
pub use platform::run_internal_mode;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
