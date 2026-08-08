use super::PendingUpdate;
use anyhow::{bail, Result};

pub(crate) fn watch_supported() -> bool {
    false
}

pub(crate) fn loaded_version() -> Option<String> {
    None
}

pub(crate) fn on_disk_version() -> Option<String> {
    None
}

pub(crate) fn notify_mismatch(_loaded: &str, _on_disk: &str) {}

pub(crate) fn guard_supported() -> bool {
    false
}

pub(crate) fn guard_armed() -> bool {
    false
}

pub(crate) fn held_nvidia_packages() -> Vec<String> {
    Vec::new()
}

pub(crate) fn pending_nvidia_updates() -> Vec<PendingUpdate> {
    Vec::new()
}

pub(crate) fn notify_held_updates(_packages: &[String]) {}

pub(crate) fn hold_driver_packages() -> Result<()> {
    bail!("gpu driver guard is Linux-only")
}

pub(crate) fn unhold_driver_packages() -> Result<()> {
    bail!("gpu driver guard is Linux-only")
}

pub(crate) fn apply_held_update(_packages: &[String]) -> Result<()> {
    bail!("gpu driver guard is Linux-only")
}
