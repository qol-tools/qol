use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

pub fn key_held(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

#[cfg(test)]
mod tests {
    use super::key_held;

    #[test]
    fn an_unassigned_virtual_key_is_never_held() {
        for vk in [0x07, 0x0A, 0xE8] {
            assert!(!key_held(vk), "{vk:#04x}");
        }
    }
}
