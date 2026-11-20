pub mod app;
pub mod cli;
pub mod config;
pub mod discovery;
mod doctor;
pub mod flow;
pub mod launch;
pub mod ui;

pub use launch::{action_for_modifiers, action_hint, action_label, LaunchAction};
pub use qol_frecency as frecency;
pub use qol_gpui::monitor;
pub use qol_gpui::window::open_window_with_focus;
pub use qol_search::{fuzzy_match, FuzzyMatch};

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
