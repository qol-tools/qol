use super::super::VirtualKeyboardProbe;
use super::super::VirtualKeyboardScan;
use crate::hotkeys::capture::platform::linux::classify::{
    classify, DeviceCapabilities, DeviceClass, SkipReason,
};

pub(super) fn virtual_keyboard_probe() -> VirtualKeyboardScan {
    use std::collections::HashSet;

    let mut devices = Vec::new();
    let mut physical_down: HashSet<u16> = HashSet::new();
    for (path, device) in evdev::enumerate() {
        match classify(&DeviceCapabilities::of(&device)) {
            DeviceClass::Skipped(SkipReason::VirtualKeyboard) => {
                let latched = device
                    .get_key_state()
                    .map(|state| {
                        let mut codes: Vec<u16> = state.iter().map(|code| code.0).collect();
                        codes.sort_unstable();
                        codes
                    })
                    .unwrap_or_default();
                devices.push(VirtualKeyboardProbe {
                    path: path.display().to_string(),
                    latched,
                });
            }
            DeviceClass::Keyboard => {
                if let Ok(state) = device.get_key_state() {
                    physical_down.extend(state.iter().map(|code| code.0));
                }
            }
            DeviceClass::Skipped(_) => {}
        }
    }
    let mut physical_down: Vec<u16> = physical_down.into_iter().collect();
    physical_down.sort_unstable();
    VirtualKeyboardScan {
        devices,
        physical_down,
    }
}

pub(super) fn keycode_display_name(code: u16) -> String {
    crate::hotkeys::capture::platform::linux::evdev_backend::keycode_name(code).to_string()
}
