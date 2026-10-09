use std::ffi::{c_int, c_void};
use std::ptr::{null, null_mut};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

const BASE_DPI: f32 = 96.0;

#[link(name = "shell32")]
extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(appid: *const u16) -> c_int;
}

pub fn is_modifier_held() -> bool {
    false
}

pub fn is_shift_held() -> bool {
    false
}

pub fn is_escape_held() -> bool {
    false
}

pub fn set_accessory_policy() {}

pub fn ghost_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

pub fn ghost_window_decorations(_transparent: bool) -> gpui::WindowDecorations {
    gpui::WindowDecorations::Client
}

pub fn adjust_ghost_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    bounds
}

pub fn should_poll_focus() -> bool {
    false
}

pub fn has_process_focus() -> bool {
    true
}

pub fn square_window_corners(window: &mut gpui::Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let preference = DWMWCP_DONOTROUND;
    let _ = unsafe {
        DwmSetWindowAttribute(
            handle.hwnd.get() as HWND,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&preference as *const i32).cast::<c_void>(),
            std::mem::size_of::<i32>() as u32,
        )
    };
}

pub fn start_window_move(window: &mut gpui::Window) {
    window.start_window_move();
}

pub fn settings_surface_taskbar_identity() -> super::SettingsSurfaceTaskbarIdentity {
    super::SettingsSurfaceTaskbarIdentity {
        app_id: qol_conventions::SETTINGS_SURFACE_APP_ID,
        display_name: qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME,
        icon: super::TaskbarIconSource::WindowClassResource,
    }
}

pub fn apply_settings_surface_identity(_window: &mut gpui::Window) {
    let mut app_id: Vec<u16> = qol_conventions::SETTINGS_SURFACE_APP_ID
        .encode_utf16()
        .collect();
    app_id.push(0);
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr());
    }
}

pub(crate) fn native_scale_for(_window: &gpui::Window) -> f32 {
    1.0
}

pub(crate) fn native_readback_position(_title: &str) -> Option<(i32, i32)> {
    None
}

pub(crate) fn readback_matches(
    _readback: Option<(i32, i32)>,
    _native: crate::window::NativeDesktopBounds,
) -> bool {
    true
}

pub(crate) fn display_scale_factor(display: gpui::DisplayId) -> f32 {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(collect_monitor),
            &mut monitors as *mut Vec<HMONITOR> as LPARAM,
        );
    }
    monitors
        .get(u32::from(display) as usize)
        .and_then(|monitor| monitor_scale(*monitor))
        .unwrap_or(1.0)
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

fn monitor_scale(monitor: HMONITOR) -> Option<f32> {
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    let result = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    (result >= 0 && dpi_x > 0).then(|| dpi_x as f32 / BASE_DPI)
}
