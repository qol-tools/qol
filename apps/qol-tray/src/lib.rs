pub mod commands;
pub mod credentials;
pub mod daemon;
#[cfg(unix)]
pub(crate) mod desktop_state;
#[cfg(feature = "dev")]
pub mod dev;
pub mod doctor;
pub mod features;
pub(crate) mod file_io;
pub mod hotkeys;
pub mod housekeeping;
pub mod installer;
pub mod logging;
pub mod menu;
pub mod migrations_startup;
pub mod mode;
pub mod paths;
pub mod plugins;
pub mod process_utils;
pub mod profile;
#[cfg(unix)]
pub mod runtime;
pub mod shortcuts;
#[cfg(unix)]
pub(crate) mod signal;
pub mod sync;
#[cfg(test)]
pub(crate) mod test_support;
pub mod tray;
pub mod updates;
pub mod version;
