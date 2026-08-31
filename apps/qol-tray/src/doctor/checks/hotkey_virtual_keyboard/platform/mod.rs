#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
use fallback as implementation;
#[cfg(target_os = "linux")]
use linux as implementation;

pub(super) fn keycode_display_name(code: u16) -> String {
    implementation::keycode_display_name(code)
}

pub(super) fn virtual_keyboard_probe() -> super::VirtualKeyboardScan {
    implementation::virtual_keyboard_probe()
}
