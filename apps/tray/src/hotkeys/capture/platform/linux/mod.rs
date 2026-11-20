use super::super::Binding;
use super::super::{OnFire, RebuildBindings};
use anyhow::Result;
use crossbeam_channel::Receiver;

pub(crate) mod classify;
pub(crate) mod evdev_backend;
mod heal;
mod matcher;
mod recorder;

pub(crate) const KEEP_REGISTERED_ONE_SHOTS: bool = false;

/// Shown when shortcut recording could not start.
pub(crate) const RECORDING_UNAVAILABLE: &str = "Shortcut recording could not start. Try again.";

pub(crate) fn install(
    bindings: Vec<Binding>,
    on_fire: OnFire,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
) -> Result<()> {
    evdev_backend::install(bindings, on_fire, reload_rx, rebuild)
}

pub(crate) fn start_recording(
    session_id: u64,
    events: std::sync::Arc<crate::daemon::EventBus>,
) -> bool {
    recorder::global().start(session_id, events)
}

pub(crate) fn cancel_recording(session_id: u64) {
    recorder::global().cancel(session_id);
}

pub(crate) fn release_held_keys() {
    evdev_backend::release_held_keys();
}
