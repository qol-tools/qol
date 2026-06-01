//! Platform abstraction for OS-wide theming.
//!
//! No working implementation exists yet on any OS — the trait is here so that
//! when a real implementation lands (likely Linux first via GTK/Qt/icon-theme
//! mechanisms, see `theme/mod.rs` for notes), the strategy-pattern boundary is
//! already in place. Until then every OS returns a typed "not implemented"
//! error.
#![allow(dead_code)]

use anyhow::Result;

pub trait ThemePlatform {
    /// Apply the named theme system-wide. Stubs return `Err`.
    fn apply_theme(&self, name: &str) -> Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

// The theme module has no business-code consumers yet; mark the re-export as
// allowed so clippy doesn't trip until a caller wires it up.
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub use linux::Platform;
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
pub use macos::Platform;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
pub use windows::Platform;
