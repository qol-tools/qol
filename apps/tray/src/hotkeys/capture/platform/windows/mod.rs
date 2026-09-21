use super::super::Binding;
use super::super::{OnFire, RebuildBindings};
use anyhow::{bail, Result};
use crossbeam_channel::Receiver;
use std::sync::Arc;

pub(crate) fn start_recording(_session_id: u64, _events: Arc<crate::daemon::EventBus>) -> bool {
    false
}

pub(crate) fn cancel_recording(_session_id: u64) {}

/// No capture backend re-emits events on Windows; nothing to flush.
pub(crate) fn release_held_keys() {}

pub(crate) fn install(
    _bindings: Vec<Binding>,
    _on_fire: OnFire,
    _reload_rx: Receiver<()>,
    _rebuild: RebuildBindings,
) -> Result<()> {
    bail!("kernel-level hotkey capture is not implemented on Windows")
}
