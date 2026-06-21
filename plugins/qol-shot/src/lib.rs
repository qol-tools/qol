pub mod cli;

mod config;
mod geometry;
mod output;
pub(crate) mod platform;
pub mod recording;
pub mod screenshot;
mod settings;

pub use config::{AudioConfig, Config, VideoConfig};
pub use geometry::{Monitor, Rect};

pub const PLUGIN_ID: &str = "qol-shot";
