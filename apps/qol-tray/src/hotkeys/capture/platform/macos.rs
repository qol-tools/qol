//! macOS capture backend stub.
//!
//! evdev/uinput is Linux-specific. macOS would need an analogous Quartz event
//! tap (CGEventTap) implementation. Until that lands, return Err so the caller
//! falls back to the `global_hotkey` listener — which works on macOS via
//! Carbon RegisterEventHotKey.

use super::super::Binding;
use anyhow::{bail, Result};

pub(crate) fn install(
    _bindings: Vec<Binding>,
    _on_fire: Box<dyn Fn(&Binding) + Send + Sync>,
) -> Result<()> {
    bail!("kernel-level hotkey capture is not implemented on macOS")
}
