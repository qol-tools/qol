use super::super::Binding;
use anyhow::{bail, Result};

pub(crate) fn install(
    _bindings: Vec<Binding>,
    _on_fire: Box<dyn Fn(&Binding) + Send + Sync>,
) -> Result<()> {
    bail!("kernel-level hotkey capture is not implemented on macOS")
}
