use qol_runtime::MonitorBounds;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
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
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return None;
        }
        Some((point.x as f32, point.y as f32))
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
        let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
        Some(FocusedWindow {
            id: Some(window as usize as u32),
            monitor: monitor_bounds(monitor)?,
        })
    }

    fn physical_monitors(&self) -> Vec<MonitorBounds> {
        let mut monitors: Vec<MonitorBounds> = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(collect_monitor),
                &mut monitors as *mut Vec<MonitorBounds> as LPARAM,
            );
        }
        monitors
    }

    fn window_open(&self, id: u32) -> Option<bool> {
        Some(unsafe { IsWindow(id as usize as HWND) } != 0)
    }
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data as *mut Vec<MonitorBounds>) };
    if let Some(bounds) = monitor_bounds(monitor) {
        monitors.push(bounds);
    }
    TRUE
}

fn monitor_bounds(monitor: HMONITOR) -> Option<MonitorBounds> {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    let rect = info.rcMonitor;
    Some(MonitorBounds {
        x: rect.left as f32,
        y: rect.top as f32,
        width: (rect.right - rect.left) as f32,
        height: (rect.bottom - rect.top) as f32,
    })
}
