pub mod daemon;
#[cfg(unix)]
pub(crate) mod desktop_state;
#[cfg(feature = "dev")]
pub mod dev;
pub mod doctor;
pub mod features;
pub(crate) mod file_io;
pub mod hotkeys;
pub mod installer;
pub mod logging;
pub mod menu;
pub mod paths;
pub mod plugins;
pub mod process_utils;
#[cfg(unix)]
pub mod runtime;
pub mod shortcuts;
pub mod signal;
pub mod tray;
pub mod updates;
pub mod version;
