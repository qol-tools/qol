use super::super::CaptureProbe;

pub(super) fn capture_probe() -> CaptureProbe {
    CaptureProbe {
        compiled: false,
        device_node_count: 0,
        keyboard_count: 0,
        uinput_writable: false,
        skipped: Vec::new(),
    }
}
