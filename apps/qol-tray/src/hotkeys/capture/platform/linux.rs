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
