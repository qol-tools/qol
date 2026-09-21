#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(all(target_os = "linux", feature = "linux_evdev"))]
pub(super) use imp::evdev_keycode;
pub(super) use imp::{PhysicalHotkeyState, POLL_INTERVAL, POLL_WHILE_IDLE};

pub(super) fn release_active_grab(
    manager: &global_hotkey::GlobalHotKeyManager,
) -> anyhow::Result<()> {
    imp::release_active_grab(manager)
}
