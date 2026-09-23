#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::{grant, restore_rule, rules_dir};
#[cfg(target_os = "linux")]
pub(super) use linux::{grant, restore_rule, rules_dir};
#[cfg(target_os = "macos")]
pub(super) use macos::{grant, restore_rule, rules_dir};
#[cfg(target_os = "windows")]
pub(super) use windows::{grant, restore_rule, rules_dir};

pub(super) const RULES_DIR: &str = "/etc/udev/rules.d";
