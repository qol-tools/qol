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

use crate::chord::{Cap, ModifierToken};

pub(super) const JOINER: Option<&str> = imp::JOINER;

pub(super) fn modifier_cap(modifier: ModifierToken) -> Cap {
    imp::modifier_cap(modifier)
}

pub(super) fn order(modifier: ModifierToken) -> u8 {
    match modifier {
        ModifierToken::Ctrl => 0,
        ModifierToken::Secondary => 1,
        ModifierToken::Shift => 2,
        ModifierToken::Alt => 3,
        ModifierToken::Platform => 4,
    }
}
