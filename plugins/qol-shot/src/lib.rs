pub mod cli;

mod actions;
mod capture_gate;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod capture_status;
mod completion;
mod config;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod daemon;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod daemon_app;
mod frozen_frame;
mod geometry;
mod output;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod pinned;
pub(crate) mod platform;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod preview;
pub mod recording;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod region_selector;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod saved_toast;
pub mod screenshot;
mod settings;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod shortcuts;
mod space;

pub use config::{AudioConfig, Config, CopyCommand, ShortcutsConfig, VideoConfig};
pub use geometry::{backdrop_regions, BackdropCorners, BackdropRegions, Monitor, Rect};

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
