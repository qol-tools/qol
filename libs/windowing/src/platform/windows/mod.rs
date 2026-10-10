use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::Once;

use crate::{WindowId, WindowOps, WindowRect};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, TRUE};
use windows_sys::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HDC, HMONITOR,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use windows_sys::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow,
    GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindow, IsWindowVisible, IsZoomed, PostMessageW, SetForegroundWindow, SetWindowPos,
    ShowWindow, GWL_EXSTYLE, GW_OWNER, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE,
    SW_RESTORE, WM_CLOSE, WS_EX_TOOLWINDOW,
};

const BASE_DPI: f32 = 96.0;
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

    pub fn try_from_id(id: &WindowId) -> Result<Self, String> {
        Self::from_id(id).ok_or_else(|| format!("Invalid window ID: {}", id.as_str()))
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

    pub fn client_frame(self) -> Option<WindowRect> {
        let mut client = empty_rect();
        if unsafe { GetClientRect(self.0, &mut client) } == 0 {
            return None;
        }
        let mut origin = POINT { x: 0, y: 0 };
        if unsafe { ClientToScreen(self.0, &mut origin) } == 0 {
            return None;
        }
        Some(to_frame(RECT {
            left: origin.x,
            top: origin.y,
            right: origin.x + client.right - client.left,
            bottom: origin.y + client.bottom - client.top,
        }))
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

    pub fn monitor(self) -> Option<Monitor> {
        monitor_info(unsafe { MonitorFromWindow(self.0, MONITOR_DEFAULTTONEAREST) })
    }

    pub fn work_area(self) -> Option<WindowRect> {
        self.monitor().map(|monitor| monitor.work_area)
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

pub struct Win32Windows;

impl WindowOps for Win32Windows {
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
        Ok(top_level_windows()
            .into_iter()
            .filter(|window| window.is_switchable())
            .map(Window::id)
            .collect())
    }

    fn window_geometry(&self, window_id: &WindowId) -> Result<Option<WindowRect>, String> {
        let window = Window::try_from_id(window_id)?;
        if !window.exists() {
            return Ok(None);
        }
        Ok(window.frame())
    }

    fn move_resize(&self, window_id: &WindowId, rect: WindowRect) -> Result<(), String> {
        Window::try_from_id(window_id)?.set_frame(rect)
    }

    fn focus_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(Window::try_from_id(window_id)?.activate())
    }

    fn minimize_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(Window::try_from_id(window_id)?.minimize())
    }

    fn restore_window(&self, window_id: &WindowId) -> Result<bool, String> {
        self.focus_window(window_id)
    }

    fn active_window_id(&self) -> Result<Option<WindowId>, String> {
        Ok(Window::foreground().map(Window::id))
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

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub bounds: WindowRect,
    pub work_area: WindowRect,
    pub device_name: String,
    pub scale: f32,
}

pub fn monitors() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(collect_monitor),
            &mut handles as *mut Vec<HMONITOR> as LPARAM,
        );
    }
    handles.into_iter().filter_map(monitor_info).collect()
}

pub fn monitor_at(x: i32, y: i32) -> Option<Monitor> {
    monitor_info(unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) })
}

pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

pub fn work_areas_left_to_right() -> Vec<WindowRect> {
    let mut areas: Vec<WindowRect> = monitors()
        .into_iter()
        .map(|monitor| monitor.work_area)
        .collect();
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

fn monitor_info(monitor: HMONITOR) -> Option<Monitor> {
    if monitor.is_null() {
        return None;
    }
    let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let read = unsafe { GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()) };
    if read == 0 {
        return None;
    }
    let device = &info.szDevice;
    let end = device
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(device.len());
    Some(Monitor {
        bounds: to_frame(info.monitorInfo.rcMonitor),
        work_area: to_frame(info.monitorInfo.rcWork),
        device_name: String::from_utf16_lossy(&device[..end]),
        scale: monitor_scale(monitor),
    })
}

fn monitor_scale(monitor: HMONITOR) -> f32 {
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    let result = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if result < 0 || dpi_x == 0 {
        return 1.0;
    }
    dpi_x as f32 / BASE_DPI
}

pub struct WindowPixels {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = 2;

pub fn capture_screen_bgra(x: i32, y: i32, width: usize, height: usize) -> Option<Vec<u8>> {
    let columns = i32::try_from(width).ok()?;
    let rows = i32::try_from(height).ok()?;
    let screen = unsafe { GetDC(null_mut()) };
    if screen.is_null() {
        return None;
    }
    let pixels = render_to_dib(width, height, |dc| unsafe {
        BitBlt(dc, 0, 0, columns, rows, screen, x, y, SRCCOPY | CAPTUREBLT) != 0
    });
    unsafe { ReleaseDC(null_mut(), screen) };
    let mut pixels = pixels?;
    make_opaque(&mut pixels);
    Some(pixels)
}

fn make_opaque(bgra: &mut [u8]) {
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel[3] = u8::MAX;
    }
}

fn print_window(hwnd: HWND, width: usize, height: usize) -> Option<Vec<u8>> {
    render_to_dib(width, height, |dc| unsafe {
        PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT) != 0
    })
}

fn render_to_dib(width: usize, height: usize, draw: impl FnOnce(HDC) -> bool) -> Option<Vec<u8>> {
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
    let drawn = draw(dc);
    let pixels =
        unsafe { std::slice::from_raw_parts(bits as *const u8, width * height * 4) }.to_vec();
    unsafe {
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    drawn.then_some(pixels)
}

fn cropped_rgba(bgra: &[u8], source_width: usize, crop: WindowRect) -> WindowPixels {
    let (left, top) = (crop.x.max(0.0) as usize, crop.y.max(0.0) as usize);
    let width = (crop.width.max(0.0) as usize).min(source_width.saturating_sub(left));
    let source_height = bgra.len() / 4 / source_width.max(1);
    let height = (crop.height.max(0.0) as usize).min(source_height.saturating_sub(top));
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in top..top + height {
        let start = (row * source_width + left) * 4;
        for pixel in bgra[start..start + width * 4].as_chunks::<4>().0 {
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

    #[test]
    fn make_opaque_sets_only_the_alpha_channel() {
        let cases: [(Vec<u8>, Vec<u8>); 3] = [
            (vec![], vec![]),
            (vec![1, 2, 3, 0], vec![1, 2, 3, 255]),
            (
                vec![9, 8, 7, 6, 5, 4, 3, 2],
                vec![9, 8, 7, 255, 5, 4, 3, 255],
            ),
        ];
        for (mut bgra, expected) in cases {
            make_opaque(&mut bgra);
            assert_eq!(bgra, expected);
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
