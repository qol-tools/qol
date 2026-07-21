pub mod cli;

mod actions;
mod capture_gate;
mod capture_status;
mod completion;
mod config;
mod daemon;
pub mod daemon_app;
mod frozen_frame;
mod geometry;
mod output;
mod pinned;
pub(crate) mod platform;
mod preview;
pub mod recording;
mod region_selector;
mod saved_toast;
pub mod screenshot;
mod settings;
mod settings_panel;
mod shortcuts;
mod space;

pub use config::{AudioConfig, Config, CopyCommand, ShortcutsConfig, VideoConfig};
pub use geometry::{backdrop_regions, BackdropCorners, BackdropRegions, Monitor, Rect};

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
