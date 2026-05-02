//! Windows capture backend stub.
//!
//! evdev/uinput is Linux-specific. Windows would need an analogous low-level
//! keyboard hook (WH_KEYBOARD_LL) implementation. Until that lands, return
//! Err so the caller falls back to the `global_hotkey` listener — which works
//! on Windows via RegisterHotKey.

use super::super::Binding;
use anyhow::{bail, Result};

pub(crate) fn install(
    _bindings: Vec<Binding>,
    _on_fire: Box<dyn Fn(&Binding) + Send + Sync>,
) -> Result<()> {
    bail!("kernel-level hotkey capture is not implemented on Windows")
}
