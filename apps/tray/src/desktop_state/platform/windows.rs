use qol_runtime::MonitorBounds;
use qol_windowing::platform::windows::{cursor_position, monitors, Monitor, Window};
use qol_windowing::WindowId;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
};

use crate::desktop_state::{is_ignored_pid, FocusedWindow, Platform};

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
