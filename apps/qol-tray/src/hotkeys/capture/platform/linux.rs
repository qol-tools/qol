//! Linux capture backend.
//!
//! When the `linux_evdev` feature is enabled, reads `/dev/input/event*`
//! keyboards directly, EVIOCGRAB-s each one so X11 / Wayland never see the
//! raw events, and re-emits everything except configured hotkey presses
//! through a single uinput virtual device.
//!
//! Without the `linux_evdev` feature, `install` returns Err — the caller
//! falls back to the `global_hotkey` (XGrabKey) listener.
//!
//! Requires udev permission on `/dev/input/event*` (typically `input` group)
//! and on `/dev/uinput` (typically `input` or a dedicated `uinput` group).
//! The `uinput` kernel module must be loaded.

use super::super::Binding;
use anyhow::Result;

#[cfg(feature = "linux_evdev")]
mod evdev_backend;
#[cfg(feature = "linux_evdev")]
mod matcher;

#[cfg(feature = "linux_evdev")]
pub(crate) fn install(
    bindings: Vec<Binding>,
    on_fire: Box<dyn Fn(&Binding) + Send + Sync>,
) -> Result<()> {
    evdev_backend::install(bindings, on_fire)
}

#[cfg(not(feature = "linux_evdev"))]
pub(crate) fn install(
    _bindings: Vec<Binding>,
    _on_fire: Box<dyn Fn(&Binding) + Send + Sync>,
) -> Result<()> {
    anyhow::bail!("evdev capture not compiled in (rebuild qol-tray with --features linux_evdev)")
}
