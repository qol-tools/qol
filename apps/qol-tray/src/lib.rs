pub mod commands;
pub mod daemon;
pub(crate) mod desktop_state;
#[cfg(feature = "dev")]
pub mod dev;
pub mod dev_generation;
pub mod doctor;
pub mod features;
pub mod hotkeys;
pub mod installer;
pub mod logging;
pub mod menu;
pub mod migrations;
pub mod paths;
pub mod plugins;
pub mod process;
pub mod profile;
pub mod runtime;
pub mod settings_surface;
pub mod shortcuts;
pub(crate) mod signal;
pub mod sync;
#[cfg(test)]
mod testing;
pub mod tray;
pub mod updates;

pub use commands::{local_http, net};
pub use daemon::reconcile;
pub use features::github_auth::credentials;
pub use installer::{housekeeping, mode};
pub use logging::{console_guard, probe};
pub use migrations as migrations_startup;
pub(crate) use paths::file_io;
pub use plugins::config::drain as config_drain;
pub use plugins::lifeline_handoff;
pub use process as process_utils;
#[cfg(test)]
pub(crate) use testing as test_support;
pub use updates::version;
