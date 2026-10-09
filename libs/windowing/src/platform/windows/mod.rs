use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Once;

use crate::{WindowId, WindowRect};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors,
    GetMonitorInfoW, MonitorFromWindow, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, PostMessageW, SetForegroundWindow, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, GW_OWNER, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE,
    WM_CLOSE, WS_EX_TOOLWINDOW,
};

const SHELL_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

pub fn ensure_dpi_awareness() {
    static AWARE: Once = Once::new();
    AWARE.call_once(|| unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    });
}

#[derive(Clone, Copy)]
pub struct Window(HWND);

impl Window {
    pub fn foreground() -> Option<Self> {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_null())
            .then_some(Self(hwnd))
            .filter(|window| !window.is_shell())
    }

    pub fn from_id(id: &WindowId) -> Option<Self> {
        id.as_u32().map(|handle| Self(handle as usize as HWND))
    }

    pub fn id(self) -> WindowId {
        WindowId::from_u32(self.0 as usize as u32)
    }

    pub fn exists(self) -> bool {
        unsafe { IsWindow(self.0) != 0 }
    }

    pub fn is_minimized(self) -> bool {
        unsafe { IsIconic(self.0) != 0 }
    }

    pub fn is_maximized(self) -> bool {
        unsafe { IsZoomed(self.0) != 0 }
    }

    pub fn is_switchable(self) -> bool {
        let shown = unsafe { IsWindowVisible(self.0) != 0 };
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

    pub fn frame(self) -> Option<WindowRect> {
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

    pub fn set_frame(self, target: WindowRect) -> Result<(), String> {
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

    pub fn maximize(self) {
        unsafe { ShowWindow(self.0, SW_MAXIMIZE) };
    }

    pub fn minimize(self) -> bool {
        unsafe { ShowWindow(self.0, SW_MINIMIZE) };
        self.is_minimized()
    }

    pub fn activate(self) -> bool {
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

    pub fn work_area(self) -> Option<WindowRect> {
        let monitor = unsafe { MonitorFromWindow(self.0, MONITOR_DEFAULTTONEAREST) };
        monitor_work_area(monitor)
    }

    pub fn title(self) -> String {
        let mut buffer = [0u16; 512];
        let length = unsafe { GetWindowTextW(self.0, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }

    pub fn request_close(self) -> bool {
        unsafe { PostMessageW(self.0, WM_CLOSE, 0, 0) != 0 }
    }

    pub fn capture_rgba(self) -> Option<WindowPixels> {
        let outer = self.outer_rect()?;
        let visible = self.visible_rect()?;
        let width = usize::try_from(outer.right - outer.left).ok()?;
        let height = usize::try_from(outer.bottom - outer.top).ok()?;
        let bgra = print_window(self.0, width, height)?;
        let crop = WindowRect {
            x: f64::from(visible.left - outer.left),
            y: f64::from(visible.top - outer.top),
            width: f64::from(visible.right - visible.left),
            height: f64::from(visible.bottom - visible.top),
        };
        Some(cropped_rgba(&bgra, width, crop))
    }

    pub fn pid(self) -> Option<u32> {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(self.0, &mut pid) };
        (pid != 0).then_some(pid)
    }
}

pub fn top_level_windows() -> Vec<Window> {
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

pub fn work_areas_left_to_right() -> Vec<WindowRect> {
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

pub struct WindowPixels {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = 2;

fn print_window(hwnd: HWND, width: usize, height: usize) -> Option<Vec<u8>> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: i32::try_from(width).ok()?,
            biHeight: -i32::try_from(height).ok()?,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [unsafe { std::mem::zeroed() }],
    };
    let dc = unsafe { CreateCompatibleDC(null_mut()) };
    if dc.is_null() {
        return None;
    }
    let mut bits: *mut c_void = null_mut();
    let bitmap = unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        unsafe { DeleteDC(dc) };
        return None;
    }
    let previous = unsafe { SelectObject(dc, bitmap) };
    let printed = unsafe { PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT) } != 0;
    let pixels =
        unsafe { std::slice::from_raw_parts(bits as *const u8, width * height * 4) }.to_vec();
    unsafe {
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    printed.then_some(pixels)
}

fn cropped_rgba(bgra: &[u8], source_width: usize, crop: WindowRect) -> WindowPixels {
    let (left, top) = (crop.x.max(0.0) as usize, crop.y.max(0.0) as usize);
    let width = (crop.width.max(0.0) as usize).min(source_width.saturating_sub(left));
    let source_height = bgra.len() / 4 / source_width.max(1);
    let height = (crop.height.max(0.0) as usize).min(source_height.saturating_sub(top));
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in top..top + height {
        let start = (row * source_width + left) * 4;
        for pixel in bgra[start..start + width * 4].chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    WindowPixels {
        width,
        height,
        rgba,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cropped_rgba_keeps_the_visible_frame() {
        let bgra: Vec<u8> = (0..4 * 3)
            .flat_map(|index| [index as u8, 100, 200, 0])
            .collect();
        let cases = [
            (
                "inner",
                rect(1.0, 1.0, 2.0, 1.0),
                2,
                1,
                vec![200, 100, 5, 255, 200, 100, 6, 255],
            ),
            (
                "clamped",
                rect(3.0, 2.0, 5.0, 5.0),
                1,
                1,
                vec![200, 100, 11, 255],
            ),
        ];
        for (name, crop, width, height, rgba) in cases {
            let pixels = cropped_rgba(&bgra, 4, crop);
            assert_eq!(
                (pixels.width, pixels.height, pixels.rgba),
                (width, height, rgba),
                "{name}"
            );
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> WindowRect {
        WindowRect {
            x,
            y,
            width,
            height,
        }
    }
}
