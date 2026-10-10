use std::ffi::{c_int, c_void};

use qol_platform::native::wide::wide_nul;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_ESCAPE, VK_MENU, VK_SHIFT,
};

#[link(name = "shell32")]
extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(appid: *const u16) -> c_int;
}

pub fn is_modifier_held() -> bool {
    key_held(VK_MENU)
}

pub fn is_shift_held() -> bool {
    key_held(VK_SHIFT)
}

pub fn is_escape_held() -> bool {
    key_held(VK_ESCAPE)
}

fn key_held(key: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetAsyncKeyState(i32::from(key)) };
    state as u16 & 0x8000 != 0
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
    let app_id = wide_nul(qol_conventions::SETTINGS_SURFACE_APP_ID);
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
    qol_windowing::platform::windows::monitors()
        .get(u32::from(display) as usize)
        .map_or(1.0, |monitor| monitor.scale)
}
