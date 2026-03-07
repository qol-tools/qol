use anyhow::{Context, Result};
use std::process::{Command, Stdio};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-os-themes/";

pub fn run() -> Result<()> {
    // TODO: shake-to-grow daemon (X11 only)
    //
    // Algorithm:
    //   1. Poll XQueryPointer in a tight loop, accumulate velocity over a rolling window (~150ms)
    //   2. On velocity threshold breach, call XFixesSetCursorName or swap Xcursor image to a
    //      pre-scaled variant (e.g. 2-3x default cursor size)
    //   3. After ~600ms of low velocity, animate back to normal size
    //
    // Relevant APIs:
    //   - XFixesGetCursorImage / XFixesSetCursor (libXfixes)
    //   - XcursorImageCreate / XcursorLibraryLoadCursor (libXcursor)
    //   - XQueryPointer for position sampling
    //
    // Wayland: no universal cursor size override API exists across compositors.
    // wlr-output-management and KDE's DBus interfaces are compositor-specific.
    // Wayland support is deferred indefinitely.
    todo!("shake-to-grow not yet implemented")
}

pub fn open_settings() -> Result<()> {
    Command::new("xdg-open")
        .arg(SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}
