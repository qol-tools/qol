use super::super::CaptureProbe;

#[cfg(feature = "linux_evdev")]
use crate::hotkeys::capture::platform::linux::classify::{
    classify, DeviceCapabilities, DeviceClass, SkipReason,
};
#[cfg(feature = "linux_evdev")]
use std::fs;
#[cfg(feature = "linux_evdev")]
use std::path::Path;

#[cfg(not(feature = "linux_evdev"))]
pub(super) fn capture_probe() -> CaptureProbe {
    CaptureProbe {
        compiled: false,
        device_node_count: 0,
        keyboard_count: 0,
        uinput_writable: false,
        skipped: Vec::new(),
    }
}

#[cfg(feature = "linux_evdev")]
pub(super) fn capture_probe() -> CaptureProbe {
    let mut keyboard_count = 0;
    let mut skipped = Vec::new();
    for (_, device) in evdev::enumerate() {
        let caps = DeviceCapabilities::of(&device);
        match classify(&caps) {
            DeviceClass::Keyboard => keyboard_count += 1,
            DeviceClass::Skipped(reason) => {
                if matches!(
                    reason,
                    SkipReason::NoKeyboardKeys | SkipReason::VirtualKeyboard
                ) {
                    continue;
                }
                skipped.push((caps.name, reason.to_string()));
            }
        }
    }
    CaptureProbe {
        compiled: true,
        device_node_count: input_event_node_count(),
        keyboard_count,
        uinput_writable: uinput_writable(),
        skipped,
    }
}

#[cfg(feature = "linux_evdev")]
fn input_event_node_count() -> usize {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("event"))
        })
        .count()
}

#[cfg(feature = "linux_evdev")]
fn uinput_writable() -> bool {
    fs::OpenOptions::new()
        .write(true)
        .open(Path::new("/dev/uinput"))
        .is_ok()
}
