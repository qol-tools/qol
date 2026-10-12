use super::security::{process_elevated, uipi_allows};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
    WH_KEYBOARD_LL, WH_MOUSE_LL,
};

pub type HookProc = unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseHook {
    Required,
    Optional,
}

#[derive(Debug)]
pub struct LowLevelHooks {
    keyboard: HHOOK,
    mouse: HHOOK,
}

impl LowLevelHooks {
    pub fn arm(keyboard: HookProc, mouse: HookProc, need: MouseHook) -> Result<Self, String> {
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0) };
        if keyboard.is_null() {
            return Err(format!(
                "failed to install the Windows keyboard hook: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), module, 0) };
        if mouse.is_null() {
            let error = format!(
                "failed to install the Windows mouse hook: {}",
                std::io::Error::last_os_error()
            );
            if need == MouseHook::Required {
                unsafe { UnhookWindowsHookEx(keyboard) };
                return Err(error);
            }
            log::warn!("{error}");
        }
        Ok(Self { keyboard, mouse })
    }

    pub fn unhook(self) {
        if !self.mouse.is_null() {
            unsafe { UnhookWindowsHookEx(self.mouse) };
        }
        unsafe { UnhookWindowsHookEx(self.keyboard) };
    }
}

pub fn last_input_tick() -> Option<u32> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    (unsafe { GetLastInputInfo(&mut info) } != 0).then_some(info.dwTime)
}

pub fn foreground_reaches_hooks(own: Option<bool>) -> bool {
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return true;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    pid == 0 || uipi_allows(own, process_elevated(pid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::CallNextHookEx;

    unsafe extern "system" fn pass(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    #[test]
    fn both_hooks_arm_and_unhook() {
        let hooks = LowLevelHooks::arm(pass, pass, MouseHook::Required).unwrap();
        assert!(!hooks.keyboard.is_null());
        assert!(!hooks.mouse.is_null());
        hooks.unhook();
    }

    #[test]
    fn the_session_reports_a_last_input_tick() {
        assert!(last_input_tick().is_some());
    }

    #[test]
    fn an_elevated_hook_owner_is_always_reached() {
        assert!(foreground_reaches_hooks(Some(true)));
    }
}
