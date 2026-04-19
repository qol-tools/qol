use anyhow::Result;

use crate::cursor::CursorEffect;

/// Platform abstraction for cursor effects.
///
/// One implementation per OS lives in `linux.rs`/`macos.rs`/`windows.rs`.
/// Stubs return typed `Err` rather than panicking — the host (qol-tray)
/// surfaces the error as a "not supported" toast.
pub trait CursorPlatform {
    /// Build the OS-specific cursor effect engine. The returned effect's
    /// `run` is responsible for surfacing platform-not-supported errors,
    /// which keeps construction infallible and matches existing call sites.
    fn create_effect(&self) -> Box<dyn CursorEffect>;

    /// Install signal handlers that route SIGTERM/SIGINT into the
    /// shared external-stop flag. Stubs are a no-op.
    fn install_signal_handlers(&self);

    /// Open the plugin's settings UI in the user's preferred way.
    fn open_settings(&self) -> Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;
