use std::ptr::null_mut;

use qol_windowing::platform::windows::{cursor_position, monitors};
use qol_windowing::WindowRect;
use windows_sys::Win32::Foundation::{BOOL, FALSE, HWND, LPARAM, POINT, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateRectRgn, DeleteObject, SetWindowRgn,
};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, GWL_EXSTYLE, HWND_NOTOPMOST, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};

use super::PopupPresentation;

pub use super::fallback::*;

const BASE_DPI: f64 = 96.0;

pub struct Platform;

impl PopupPresentation for Platform {
    fn present_topmost(title: &str) {
        if let Some(hwnd) = find_window(title) {
            place(hwnd, HWND_TOPMOST, SWP_NOACTIVATE);
        }
    }

    fn restore_composite(_title: &str) {}
}

struct TitleSearch {
    pid: u32,
    title: Vec<u16>,
    found: HWND,
}

fn find_window(title: &str) -> Option<HWND> {
    let mut search = TitleSearch {
        pid: unsafe { GetCurrentProcessId() },
        title: title.encode_utf16().collect(),
        found: null_mut(),
    };
    unsafe {
        EnumWindows(Some(match_title), &mut search as *mut TitleSearch as LPARAM);
    }
    (!search.found.is_null()).then_some(search.found)
}

unsafe extern "system" fn match_title(hwnd: HWND, data: LPARAM) -> BOOL {
    let search = unsafe { &mut *(data as *mut TitleSearch) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid != search.pid {
        return TRUE;
    }
    let mut buffer = vec![0u16; search.title.len() + 2];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied as usize == search.title.len() && buffer[..search.title.len()] == search.title[..] {
        search.found = hwnd;
        return FALSE;
    }
    TRUE
}

fn place(hwnd: HWND, after: HWND, flags: u32) -> bool {
    unsafe { SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | flags) != 0 }
}

fn window_scale(hwnd: HWND) -> f64 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / BASE_DPI
    }
}

fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    (unsafe { GetWindowRect(hwnd, &mut rect) } != 0).then_some(rect)
}

fn raise_foreground(hwnd: HWND) {
    unsafe {
        let this_thread = GetCurrentThreadId();
        let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
        let attached = foreground_thread != 0
            && foreground_thread != this_thread
            && AttachThreadInput(this_thread, foreground_thread, TRUE) != 0;
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(this_thread, foreground_thread, FALSE);
        }
    }
}

fn show_window(title: &str, topmost: bool, focus: bool) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    let after = if topmost {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };
    let placed = place(hwnd, after, SWP_SHOWWINDOW | SWP_NOACTIVATE);
    if focus {
        raise_foreground(hwnd);
    }
    placed
}

fn apply_tool_window_style(hwnd: HWND) {
    let current = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let wanted = (current | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
    if wanted == current {
        return;
    }
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

fn move_window(hwnd: HWND, x: i32, y: i32) -> bool {
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        ) != 0
    }
}

fn gpui_to_native(hwnd: HWND, gpui_x: f64, gpui_y: f64) -> (i32, i32) {
    let scale = window_scale(hwnd);
    (
        (gpui_x * scale).round() as i32,
        (gpui_y * scale).round() as i32,
    )
}

#[derive(Clone)]
pub struct WindowGeometrySession {
    hwnd: isize,
}

impl WindowGeometrySession {
    fn handle(&self) -> HWND {
        self.hwnd as HWND
    }

    pub fn set_bounds(&self, x: i32, y: i32, width: u32, height: u32) {
        unsafe {
            SetWindowPos(
                self.handle(),
                null_mut(),
                x,
                y,
                width as i32,
                height as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub fn set_position(&self, x: i32, y: i32) {
        move_window(self.handle(), x, y);
    }

    pub fn reposition(&self, x: i32, y: i32) -> bool {
        let (x, y) = gpui_to_native(self.handle(), f64::from(x), f64::from(y));
        move_window(self.handle(), x, y)
    }

    pub fn pointer_root(&self) -> Option<(i32, i32)> {
        cursor_position()
    }

    pub fn bounds(&self) -> Option<(i32, i32, u32, u32)> {
        let rect = window_rect(self.handle())?;
        Some((
            rect.left,
            rect.top,
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        ))
    }

    pub fn pointer_on(&self) -> Option<crate::popup_window::PointerOnWindow> {
        None
    }

    pub fn set_input_region(&self, x: i16, y: i16, width: u16, height: u16) -> bool {
        let hwnd = self.handle();
        let scale = window_scale(hwnd);
        let native = |value: f64| (value * scale).round() as i32;
        let left = native(f64::from(x));
        let top = native(f64::from(y));
        apply_input_region(
            hwnd,
            left,
            top,
            native(f64::from(x) + f64::from(width)) - left,
            native(f64::from(y) + f64::from(height)) - top,
        )
    }

    pub fn anchor_content(&self, _right: bool, _bottom: bool) {}
}

pub fn window_geometry_session(title: &str) -> Option<WindowGeometrySession> {
    find_window(title).map(|hwnd| WindowGeometrySession {
        hwnd: hwnd as isize,
    })
}

pub fn work_area_within(monitor: gpui::Bounds<gpui::Pixels>) -> Option<gpui::Bounds<gpui::Pixels>> {
    let center = monitor.center();
    let (x, y) = (f64::from(center.x), f64::from(center.y));
    let found = monitors().into_iter().find(|candidate| {
        let bounds = candidate.bounds;
        (bounds.x..bounds.x + bounds.width).contains(&x)
            && (bounds.y..bounds.y + bounds.height).contains(&y)
    })?;
    work_area_in(monitor, found.bounds, found.work_area)
}

fn work_area_in(
    monitor: gpui::Bounds<gpui::Pixels>,
    full: WindowRect,
    work: WindowRect,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    if full.width <= 0.0 {
        return None;
    }
    let scale = f64::from(monitor.size.width) / full.width;
    let inset = |native: f64| gpui::px((native * scale) as f32);
    Some(gpui::Bounds::from_corners(
        gpui::point(
            monitor.origin.x + inset(work.x - full.x),
            monitor.origin.y + inset(work.y - full.y),
        ),
        gpui::point(
            monitor.right() - inset(full.x + full.width - work.x - work.width),
            monitor.bottom() - inset(full.y + full.height - work.y - work.height),
        ),
    ))
}

pub fn set_input_region_by_title(title: &str, x: i16, y: i16, width: u16, height: u16) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    apply_input_region(
        hwnd,
        i32::from(x),
        i32::from(y),
        i32::from(width),
        i32::from(height),
    )
}

fn apply_input_region(hwnd: HWND, left: i32, top: i32, width: i32, height: i32) -> bool {
    let region = unsafe { CreateRectRgn(left, top, left + width, top + height) };
    if region.is_null() {
        return false;
    }
    if unsafe { SetWindowRgn(hwnd, region, TRUE) } == 0 {
        unsafe { DeleteObject(region) };
        return false;
    }
    true
}

pub fn window_position_by_title(title: &str) -> Option<(i32, i32)> {
    let rect = window_rect(find_window(title)?)?;
    Some((rect.left, rect.top))
}

pub fn pointer_over_window_by_title(title: &str) -> bool {
    let Some(rect) = find_window(title).and_then(window_rect) else {
        return false;
    };
    cursor_position()
        .is_some_and(|(x, y)| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
}

pub fn reposition_window_by_title(title: &str, gpui_x: f64, gpui_y: f64) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    let (x, y) = gpui_to_native(hwnd, gpui_x, gpui_y);
    move_window(hwnd, x, y)
}

pub fn sync_window_layout(
    title: &str,
    window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let scale = scale_at(origin).unwrap_or_else(|| window.scale_factor());
    crate::window::resize_or_sync_scale(window, size, Some(scale));
    sync_window_layout_by_title(title, gpui::point(origin.x * scale, origin.y * scale), size)
}

fn scale_at(origin: gpui::Point<gpui::Pixels>) -> Option<f32> {
    let (x, y) = (origin.x.to_f64(), origin.y.to_f64());
    monitors()
        .into_iter()
        .find(|candidate| {
            let bounds = candidate.bounds;
            (bounds.x..bounds.x + bounds.width).contains(&x)
                && (bounds.y..bounds.y + bounds.height).contains(&y)
        })
        .map(|candidate| candidate.scale)
}

pub fn sync_window_layout_by_title(
    title: &str,
    origin: gpui::Point<gpui::Pixels>,
    _size: gpui::Size<gpui::Pixels>,
) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    let (frame_x, frame_y) = client_offset(hwnd).unwrap_or((0, 0));
    move_window(
        hwnd,
        origin.x.to_f64().round() as i32 - frame_x,
        origin.y.to_f64().round() as i32 - frame_y,
    )
}

fn client_offset(hwnd: HWND) -> Option<(i32, i32)> {
    let frame = window_rect(hwnd)?;
    let mut client = POINT { x: 0, y: 0 };
    (unsafe { ClientToScreen(hwnd, &mut client) } != 0)
        .then_some((client.x - frame.left, client.y - frame.top))
}

pub fn focus_window_by_title(title: &str) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    raise_foreground(hwnd);
    true
}

pub fn hide_window_by_title(title: &str) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    unsafe { ShowWindow(hwnd, SW_HIDE) };
    true
}

pub fn hide_invisible(title: &str) -> bool {
    hide_window_by_title(title)
}

pub fn park_window_by_title(title: &str) -> bool {
    hide_window_by_title(title)
}

pub fn show_window_by_title(title: &str) -> bool {
    show_window(title, true, true)
}

pub fn show_window_passive_by_title(title: &str) -> bool {
    show_window(title, true, false)
}

pub fn show_window_interactive_by_title(title: &str) -> bool {
    show_window(title, true, false)
}

pub fn show_toast_window_by_title(title: &str) -> bool {
    show_window(title, true, false)
}

pub fn show_normal_window_by_title(title: &str) -> bool {
    show_window(title, false, true)
}

pub fn configure_popup_window(title: &str) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    apply_tool_window_style(hwnd);
    true
}

pub fn configure_overlay_window(title: &str) -> bool {
    let Some(hwnd) = find_window(title) else {
        return false;
    };
    apply_tool_window_style(hwnd);
    place(hwnd, HWND_TOPMOST, SWP_NOACTIVATE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> WindowRect {
        WindowRect {
            x: f64::from(left),
            y: f64::from(top),
            width: f64::from(right - left),
            height: f64::from(bottom - top),
        }
    }

    #[test]
    fn work_area_keeps_the_monitor_units() {
        let cases = [
            (
                "bottom taskbar, physical",
                gpui::bounds(
                    gpui::point(gpui::px(0.0), gpui::px(0.0)),
                    gpui::size(gpui::px(1280.0), gpui::px(800.0)),
                ),
                rect(0, 0, 1280, 800),
                rect(0, 0, 1280, 752),
                (0.0, 0.0, 1280.0, 752.0),
            ),
            (
                "bottom taskbar, logical at 150%",
                gpui::bounds(
                    gpui::point(gpui::px(0.0), gpui::px(0.0)),
                    gpui::size(gpui::px(1280.0), gpui::px(720.0)),
                ),
                rect(0, 0, 1920, 1080),
                rect(0, 0, 1920, 1008),
                (0.0, 0.0, 1280.0, 672.0),
            ),
            (
                "left taskbar on a second monitor",
                gpui::bounds(
                    gpui::point(gpui::px(1920.0), gpui::px(0.0)),
                    gpui::size(gpui::px(1920.0), gpui::px(1080.0)),
                ),
                rect(1920, 0, 3840, 1080),
                rect(1968, 0, 3840, 1080),
                (1968.0, 0.0, 1872.0, 1080.0),
            ),
        ];
        for (name, monitor, full, work, (x, y, width, height)) in cases {
            let area = work_area_in(monitor, full, work).expect(name);
            assert_eq!(
                (
                    f32::from(area.origin.x),
                    f32::from(area.origin.y),
                    f32::from(area.size.width),
                    f32::from(area.size.height),
                ),
                (x, y, width, height),
                "{name}"
            );
        }
    }

    #[test]
    fn work_area_refuses_an_empty_monitor() {
        let monitor = gpui::bounds(
            gpui::point(gpui::px(0.0), gpui::px(0.0)),
            gpui::size(gpui::px(0.0), gpui::px(0.0)),
        );
        assert!(work_area_in(monitor, rect(0, 0, 0, 0), rect(0, 0, 0, 0)).is_none());
    }
}
