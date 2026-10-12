use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use qol_gpui::popup_window::WindowGeometrySession;
use qol_windowing::platform::windows::Window;
use qol_windowing::WindowRect;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HC_ACTION, MSG, PM_NOREMOVE, WH_MOUSE_LL, WM_APP, WM_LBUTTONDOWN,
    WM_MBUTTONDOWN, WM_QUIT, WM_RBUTTONDOWN, WM_XBUTTONDOWN,
};

const CLICK_MESSAGE: u32 = WM_APP + 0x51;

pub(crate) struct Monitor {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn start(window_title: String, tx: mpsc::Sender<()>) -> Option<Monitor> {
    let window = qol_gpui::popup_window::window_geometry_session(&window_title)?;
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("qol-click-away".to_string())
        .spawn(move || monitor_loop(window, tx, ready_tx))
        .ok()?;
    match ready_rx.recv() {
        Ok(Some(thread_id)) => Some(Monitor {
            thread_id,
            thread: Some(thread),
        }),
        _ => {
            let _ = thread.join();
            None
        }
    }
}

fn monitor_loop(
    window: WindowGeometrySession,
    tx: mpsc::Sender<()>,
    ready: mpsc::Sender<Option<u32>>,
) {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE) };
    let hook =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), GetModuleHandleW(null()), 0) };
    if hook.is_null() {
        let _ = ready.send(None);
        return;
    }
    let _ = ready.send(Some(unsafe { GetCurrentThreadId() }));
    while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
        if message.message == CLICK_MESSAGE && !pointer_over(&window) {
            let _ = tx.send(());
        }
    }
    unsafe { UnhookWindowsHookEx(hook) };
}

fn pointer_over(window: &WindowGeometrySession) -> bool {
    match (window.bounds(), window.pointer_root()) {
        (Some(bounds), Some(point)) => {
            let region = Window::from_id(&window.id()).and_then(Window::region);
            point_inside(bounds, point) && in_region(region, point)
        }
        _ => false,
    }
}

fn point_inside((left, top, width, height): (i32, i32, u32, u32), (x, y): (i32, i32)) -> bool {
    let (x, y) = (i64::from(x), i64::from(y));
    let (left, top) = (i64::from(left), i64::from(top));
    x >= left && x < left + i64::from(width) && y >= top && y < top + i64::from(height)
}

fn in_region(region: Option<WindowRect>, (x, y): (i32, i32)) -> bool {
    let (x, y) = (f64::from(x), f64::from(y));
    region.is_none_or(|region| {
        x >= region.x
            && x < region.x + region.width
            && y >= region.y
            && y < region.y + region.height
    })
}

fn is_button_press(message: u32) -> bool {
    matches!(
        message,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
    )
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && is_button_press(wparam as u32) {
        unsafe { PostThreadMessageW(GetCurrentThreadId(), CLICK_MESSAGE, 0, 0) };
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL};

    #[test]
    fn only_button_presses_count_as_clicks() {
        let cases = [
            (WM_LBUTTONDOWN, true),
            (WM_RBUTTONDOWN, true),
            (WM_MBUTTONDOWN, true),
            (WM_XBUTTONDOWN, true),
            (WM_LBUTTONUP, false),
            (WM_MOUSEMOVE, false),
            (WM_MOUSEWHEEL, false),
        ];
        for (message, expected) in cases {
            assert_eq!(is_button_press(message), expected, "message={message:#x}");
        }
    }

    #[test]
    fn pointer_inside_counts_left_and_top_edges_only() {
        let bounds = (10, 20, 100, 50);
        let cases = [
            ((10, 20), true),
            ((60, 45), true),
            ((109, 69), true),
            ((110, 45), false),
            ((60, 70), false),
            ((9, 45), false),
            ((60, 19), false),
            ((-5, -5), false),
        ];
        for (point, expected) in cases {
            assert_eq!(point_inside(bounds, point), expected, "point={point:?}");
        }
    }

    #[test]
    fn a_window_region_clips_the_hit_test() {
        let region = WindowRect {
            x: 200.0,
            y: 0.0,
            width: 400.0,
            height: 300.0,
        };
        let cases = [
            (None, (5, 5), true),
            (Some(region), (200, 0), true),
            (Some(region), (599, 299), true),
            (Some(region), (600, 100), false),
            (Some(region), (300, 300), false),
            (Some(region), (5, 5), false),
        ];
        for (region, point, expected) in cases {
            assert_eq!(
                in_region(region, point),
                expected,
                "region={region:?} point={point:?}"
            );
        }
    }

    #[test]
    fn pointer_inside_handles_negative_origin_monitors() {
        let bounds = (-1920, -200, 800, 600);
        assert!(point_inside(bounds, (-1500, 0)));
        assert!(!point_inside(bounds, (-1120, 0)));
        assert!(!point_inside((0, 0, 0, 0), (0, 0)));
    }
}
