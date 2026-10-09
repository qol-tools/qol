use std::ptr::null_mut;
use std::sync::Once;

use qol_windowing::{WindowId, WindowRect};
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, GetProcessTimes, OpenProcess,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, SetForegroundWindow, SetWindowPos, ShowWindow, GWL_EXSTYLE,
    GW_OWNER, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WS_EX_TOOLWINDOW,
};

const SHELL_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

pub(super) fn ensure_dpi_awareness() {
    static AWARE: Once = Once::new();
    AWARE.call_once(|| unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    });
}

#[derive(Clone, Copy)]
pub(super) struct Window(HWND);

impl Window {
    pub(super) fn foreground() -> Option<Self> {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_null())
            .then_some(Self(hwnd))
            .filter(|window| !window.is_shell())
    }

    pub(super) fn from_id(id: &WindowId) -> Option<Self> {
        id.as_u32().map(|handle| Self(handle as usize as HWND))
    }

    pub(super) fn id(self) -> WindowId {
        WindowId::from_u32(self.0 as usize as u32)
    }

    pub(super) fn exists(self) -> bool {
        unsafe { IsWindow(self.0) != 0 }
    }

    pub(super) fn is_minimized(self) -> bool {
        unsafe { IsIconic(self.0) != 0 }
    }

    pub(super) fn is_maximized(self) -> bool {
        unsafe { IsZoomed(self.0) != 0 }
    }

    pub(super) fn is_switchable(self) -> bool {
        let shown = unsafe { IsWindowVisible(self.0) != 0 } || self.is_minimized();
        let owned = unsafe { !GetWindow(self.0, GW_OWNER).is_null() };
        let style = unsafe { GetWindowLongPtrW(self.0, GWL_EXSTYLE) };
        let tool = style & WS_EX_TOOLWINDOW as isize != 0;
        shown && !owned && !tool && !self.is_cloaked() && !self.is_shell()
    }

    fn is_cloaked(self) -> bool {
        let mut cloaked = 0u32;
        let read = unsafe {
            DwmGetWindowAttribute(
                self.0,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        read >= 0 && cloaked != 0
    }

    fn is_shell(self) -> bool {
        let mut buffer = [0u16; 64];
        let length = unsafe { GetClassNameW(self.0, buffer.as_mut_ptr(), buffer.len() as i32) };
        let class = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
        SHELL_CLASSES.contains(&class.as_str())
    }

    pub(super) fn frame(self) -> Option<WindowRect> {
        self.visible_rect().map(to_frame)
    }

    fn visible_rect(self) -> Option<RECT> {
        let mut rect = empty_rect();
        let read = unsafe {
            DwmGetWindowAttribute(
                self.0,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                (&mut rect as *mut RECT).cast(),
                std::mem::size_of::<RECT>() as u32,
            )
        };
        if read >= 0 {
            return Some(rect);
        }
        self.outer_rect()
    }

    fn outer_rect(self) -> Option<RECT> {
        let mut rect = empty_rect();
        (unsafe { GetWindowRect(self.0, &mut rect) } != 0).then_some(rect)
    }

    pub(super) fn set_frame(self, target: WindowRect) -> Result<(), String> {
        if self.is_minimized() || self.is_maximized() {
            unsafe { ShowWindow(self.0, SW_RESTORE) };
        }
        let outer = self.outer_rect().ok_or("Cannot read window geometry")?;
        let visible = self.visible_rect().ok_or("Cannot read window geometry")?;
        let left = visible.left - outer.left;
        let top = visible.top - outer.top;
        let right = outer.right - visible.right;
        let bottom = outer.bottom - visible.bottom;
        let placed = unsafe {
            SetWindowPos(
                self.0,
                null_mut(),
                target.x.round() as i32 - left,
                target.y.round() as i32 - top,
                target.width.round() as i32 + left + right,
                target.height.round() as i32 + top + bottom,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if placed == 0 {
            return Err("Failed to set window geometry".to_string());
        }
        Ok(())
    }

    pub(super) fn maximize(self) {
        unsafe { ShowWindow(self.0, SW_MAXIMIZE) };
    }

    pub(super) fn minimize(self) -> bool {
        unsafe { ShowWindow(self.0, SW_MINIMIZE) };
        self.is_minimized()
    }

    pub(super) fn activate(self) -> bool {
        if self.is_minimized() {
            unsafe { ShowWindow(self.0, SW_RESTORE) };
        }
        unsafe {
            let this_thread = GetCurrentThreadId();
            let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
            let attached = foreground_thread != 0
                && foreground_thread != this_thread
                && AttachThreadInput(this_thread, foreground_thread, TRUE) != 0;
            BringWindowToTop(self.0);
            let activated = SetForegroundWindow(self.0) != 0;
            if attached {
                AttachThreadInput(this_thread, foreground_thread, 0);
            }
            activated
        }
    }

    pub(super) fn work_area(self) -> Option<WindowRect> {
        let monitor = unsafe { MonitorFromWindow(self.0, MONITOR_DEFAULTTONEAREST) };
        monitor_work_area(monitor)
    }

    pub(super) fn pid(self) -> Option<u32> {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(self.0, &mut pid) };
        (pid != 0).then_some(pid)
    }
}

pub(super) fn top_level_windows() -> Vec<Window> {
    let mut windows: Vec<Window> = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_window),
            &mut windows as *mut Vec<Window> as LPARAM,
        );
    }
    windows
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let windows = unsafe { &mut *(data as *mut Vec<Window>) };
    windows.push(Window(hwnd));
    TRUE
}

pub(super) fn work_areas_left_to_right() -> Vec<WindowRect> {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut monitors as *mut Vec<HMONITOR> as LPARAM,
        );
    }
    let mut areas: Vec<WindowRect> = monitors.into_iter().filter_map(monitor_work_area).collect();
    areas.sort_by(|a, b| a.x.total_cmp(&b.x));
    areas
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    TRUE
}

fn monitor_work_area(monitor: HMONITOR) -> Option<WindowRect> {
    if monitor.is_null() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: empty_rect(),
        rcWork: empty_rect(),
        dwFlags: 0,
    };
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then(|| to_frame(info.rcWork))
}

pub(super) fn process_start_ticks(pid: u32) -> Option<u64> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut created = empty_filetime();
    let (mut exited, mut kernel, mut user) = (empty_filetime(), empty_filetime(), empty_filetime());
    let read =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    unsafe { CloseHandle(process) };
    (read != 0)
        .then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

pub(super) fn process_image_name(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let read = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    unsafe { CloseHandle(process) };
    (read != 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

fn to_frame(rect: RECT) -> WindowRect {
    WindowRect {
        x: f64::from(rect.left),
        y: f64::from(rect.top),
        width: f64::from(rect.right - rect.left),
        height: f64::from(rect.bottom - rect.top),
    }
}

fn empty_rect() -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

fn empty_filetime() -> FILETIME {
    FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }
}
