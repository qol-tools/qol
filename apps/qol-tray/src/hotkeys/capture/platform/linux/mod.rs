use super::super::Binding;
use super::super::{OnFire, RebuildBindings};
use anyhow::Result;
use crossbeam_channel::Receiver;

#[cfg(feature = "linux_evdev")]
mod evdev_backend;
#[cfg(feature = "linux_evdev")]
mod matcher;
mod recorder;

#[cfg(feature = "linux_evdev")]
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

#[cfg(not(feature = "linux_evdev"))]
pub(crate) fn install(
    _bindings: Vec<Binding>,
    _on_fire: OnFire,
    _reload_rx: Receiver<()>,
    _rebuild: RebuildBindings,
) -> Result<()> {
    anyhow::bail!("evdev capture not compiled in (rebuild qol-tray with --features linux_evdev)")
}
