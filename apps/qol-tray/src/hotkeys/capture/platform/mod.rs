//! Per-OS hotkey capture backends. Each `<os>.rs` exports `pub(crate) fn install`
//! conforming to the same signature; this module wires the active one in via a
//! cfg-aliased re-export so business code calls a single symbol regardless of OS.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::install;
#[cfg(target_os = "macos")]
pub(crate) use macos::install;
#[cfg(target_os = "windows")]
pub(crate) use windows::install;
