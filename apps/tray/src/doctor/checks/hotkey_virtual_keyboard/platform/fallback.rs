use super::super::VirtualKeyboardScan;

pub(super) fn virtual_keyboard_probe() -> VirtualKeyboardScan {
    VirtualKeyboardScan {
        compiled: false,
        devices: Vec::new(),
        physical_down: Vec::new(),
    }
}

pub(super) fn keycode_display_name(code: u16) -> String {
    format!("0x{code:02x}")
}
