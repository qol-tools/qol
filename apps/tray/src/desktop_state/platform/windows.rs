use qol_runtime::MonitorBounds;
use qol_windowing::platform::windows::{
    cursor_position, monitors, top_level_windows, Monitor, Window,
};
use qol_windowing::WindowId;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
};

use crate::desktop_state::{fingerprint_windows, is_ignored_pid, FocusedWindow, Platform};

pub(super) fn create() -> impl Platform {
    WindowsQueries {
        own_pid: std::process::id(),
    }
}

struct WindowsQueries {
    own_pid: u32,
}

impl Platform for WindowsQueries {
    fn cursor_position(&self) -> Option<(f32, f32)> {
        cursor_position().map(|(x, y)| (x as f32, y as f32))
    }

    fn focused_window_bounds(&self) -> Option<MonitorBounds> {
        self.focused_window().map(|focused| focused.monitor)
    }

    fn focused_window(&self) -> Option<FocusedWindow> {
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid == self.own_pid || is_ignored_pid(pid) {
            return None;
        }
        let id = window as usize as u32;
        let monitor = Window::from_id(&WindowId::from_u32(id))?.monitor()?;
        Some(FocusedWindow {
            id: Some(id),
            monitor: monitor_bounds(&monitor),
        })
    }

    fn physical_monitors(&self) -> Vec<MonitorBounds> {
        monitors().iter().map(monitor_bounds).collect()
    }

    fn window_open(&self, id: u32) -> Option<bool> {
        Some(unsafe { IsWindow(id as usize as HWND) } != 0)
    }

    fn window_list_fingerprint(&self) -> Option<u64> {
        let windows = top_level_windows()
            .into_iter()
            .filter(|window| window.is_switchable())
            .filter_map(|window| window.id().as_u32())
            .map(u64::from);
        let foreground = unsafe { GetForegroundWindow() };
        let focused = (!foreground.is_null()).then_some(foreground as usize as u64);
        Some(fingerprint_windows(windows, focused))
    }
}

fn monitor_bounds(monitor: &Monitor) -> MonitorBounds {
    let bounds = monitor.bounds;
    MonitorBounds {
        x: bounds.x as f32,
        y: bounds.y as f32,
        width: bounds.width as f32,
        height: bounds.height as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::core::w;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    #[test]
    fn opening_a_window_changes_the_window_list_fingerprint() {
        let queries = create();
        let before = queries.window_list_fingerprint();
        let window = unsafe {
            CreateWindowExW(
                0,
                w!("STATIC"),
                w!("qol fingerprint probe"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                0,
                0,
                200,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!window.is_null());
        let after = queries.window_list_fingerprint();
        unsafe { DestroyWindow(window) };

        assert!(before.is_some());
        assert_ne!(before, after);
    }
}
