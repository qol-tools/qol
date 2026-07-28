pub mod app;
mod capture;
pub mod cli;
mod config;
pub(crate) mod platform;
mod ui;

pub use app as daemon_app;
pub use capture::geometry::{backdrop_regions, BackdropCorners, BackdropRegions, Monitor, Rect};
pub use capture::{recording, screenshot};
pub use config::{
    AudioConfig, CaptureConfig, Config, CopyCommand, SavedFeedback, ShortcutsConfig, VideoConfig,
};

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
