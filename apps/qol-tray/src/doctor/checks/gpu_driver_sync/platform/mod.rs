#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingUpdate {
    pub name: String,
    pub new_version: String,
}

#[cfg(all(target_os = "linux", test))]
pub(super) use linux::parse_upgradable;
#[cfg(target_os = "linux")]
pub(super) use linux::{
    apply_held_update, guard_armed, guard_supported, held_nvidia_packages, hold_driver_packages,
    loaded_version, notify_held_updates, notify_mismatch, on_disk_version, pending_nvidia_updates,
    unhold_driver_packages, watch_supported,
};
#[cfg(target_os = "macos")]
pub(super) use macos::{
    apply_held_update, guard_armed, guard_supported, held_nvidia_packages, hold_driver_packages,
    loaded_version, notify_held_updates, notify_mismatch, on_disk_version, pending_nvidia_updates,
    unhold_driver_packages, watch_supported,
};
#[cfg(target_os = "windows")]
pub(super) use windows::{
    apply_held_update, guard_armed, guard_supported, held_nvidia_packages, hold_driver_packages,
    loaded_version, notify_held_updates, notify_mismatch, on_disk_version, pending_nvidia_updates,
    unhold_driver_packages, watch_supported,
};
