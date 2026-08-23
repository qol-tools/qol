#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod text;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

use crate::chord::ModifierToken;

trait ChordStyle {
    fn modifier_label(&self, modifier: ModifierToken) -> &'static str;
    fn join(&self, mods: &[&str], key: &str) -> String;
}

pub(super) fn modifier_label(modifier: ModifierToken) -> &'static str {
    imp::Platform.modifier_label(modifier)
}

pub(super) fn join(mods: &[&str], key: &str) -> String {
    imp::Platform.join(mods, key)
}
