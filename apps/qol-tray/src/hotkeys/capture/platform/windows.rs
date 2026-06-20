use super::super::Binding;
use super::super::{OnFire, RebuildBindings};
use anyhow::{bail, Result};
use crossbeam_channel::Receiver;

pub(crate) fn install(
    _bindings: Vec<Binding>,
    _on_fire: OnFire,
    _reload_rx: Receiver<()>,
    _rebuild: RebuildBindings,
) -> Result<()> {
    bail!("kernel-level hotkey capture is not implemented on Windows")
}
