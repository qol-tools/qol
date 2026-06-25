pub mod cli;

mod actions;
mod config;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod daemon;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod daemon_app;
mod geometry;
mod output;
pub(crate) mod platform;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod preview;
pub mod recording;
pub mod screenshot;
mod settings;

pub use config::{AudioConfig, Config, VideoConfig};
pub use geometry::{Monitor, Rect};

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
