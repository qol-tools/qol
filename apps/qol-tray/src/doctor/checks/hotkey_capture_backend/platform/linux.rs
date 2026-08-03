use super::super::CaptureProbe;

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
    }
}

#[cfg(feature = "linux_evdev")]
pub(super) fn capture_probe() -> CaptureProbe {
    CaptureProbe {
        compiled: true,
        device_node_count: input_event_node_count(),
        keyboard_count: evdev::enumerate()
            .map(|(_, device)| device)
            .filter(is_keyboard)
            .count(),
        uinput_writable: uinput_writable(),
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
fn is_keyboard(device: &evdev::Device) -> bool {
    let Some(keys) = device.supported_keys() else {
        return false;
    };
    keys.contains(evdev::KeyCode::KEY_ESC) && keys.contains(evdev::KeyCode::KEY_A)
}

#[cfg(feature = "linux_evdev")]
fn uinput_writable() -> bool {
    fs::OpenOptions::new()
        .write(true)
        .open(Path::new("/dev/uinput"))
        .is_ok()
}
