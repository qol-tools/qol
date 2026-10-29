use std::ffi::c_void;

use qol_platform::native::input::key_held;
use qol_platform::native::wide::wide_nul;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{BOOL, HWND};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, VK_ESCAPE, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SystemParametersInfoW, HTCAPTION, SPI_GETCLIENTAREAANIMATION, WM_NCLBUTTONDOWN,
};

const PROPERTY_STORE: GUID = GUID::from_u128(0x886d8eeb_8cf2_4446_8d02_cdba1dbdcf99);
const APP_USER_MODEL: GUID = GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3);
const RELAUNCH_COMMAND: u32 = 2;
const RELAUNCH_ICON_RESOURCE: u32 = 3;
const RELAUNCH_DISPLAY_NAME_RESOURCE: u32 = 4;
const APP_ID: u32 = 5;
const VT_LPWSTR: u16 = 31;

#[repr(C)]
struct PropertyKey {
    fmtid: GUID,
    pid: u32,
}

#[repr(C)]
struct StringVariant {
    vt: u16,
    reserved: [u16; 3],
    value: *const u16,
    padding: usize,
}

#[repr(C)]
struct PropertyStoreVtbl {
    before_release: [usize; 2],
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    before_set_value: [usize; 3],
    set_value:
        unsafe extern "system" fn(*mut c_void, *const PropertyKey, *const StringVariant) -> HRESULT,
    commit: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

#[link(name = "shell32")]
extern "system" {
    fn SHGetPropertyStoreForWindow(
        hwnd: HWND,
        riid: *const GUID,
        store: *mut *mut c_void,
    ) -> HRESULT;
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

pub fn reduce_motion() -> bool {
    let mut animate: BOOL = 1;
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animate as *mut BOOL).cast(),
            0,
        )
    };
    read != 0 && animate == 0
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

pub fn square_window_corners(window: &mut gpui::Window, _framed: bool) {
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
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    unsafe {
        ReleaseCapture();
        PostMessageW(
            handle.hwnd.get() as HWND,
            WM_NCLBUTTONDOWN,
            HTCAPTION as usize,
            0,
        );
    }
}

pub fn settings_surface_taskbar_identity() -> super::SettingsSurfaceTaskbarIdentity {
    super::SettingsSurfaceTaskbarIdentity {
        app_id: qol_conventions::SETTINGS_SURFACE_APP_ID,
        display_name: qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME,
        icon: super::TaskbarIconSource::WindowClassResource,
    }
}

pub fn apply_settings_surface_identity(window: &mut gpui::Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let exe = exe.display();
    let properties = [
        (RELAUNCH_COMMAND, format!("\"{exe}\"")),
        (RELAUNCH_ICON_RESOURCE, format!("{exe},-1")),
        (
            RELAUNCH_DISPLAY_NAME_RESOURCE,
            qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME.to_owned(),
        ),
        (APP_ID, qol_conventions::SETTINGS_SURFACE_APP_ID.to_owned()),
    ];
    let mut store: *mut c_void = std::ptr::null_mut();
    unsafe {
        if SHGetPropertyStoreForWindow(handle.hwnd.get() as HWND, &PROPERTY_STORE, &mut store) < 0 {
            return;
        }
        let vtbl = &**store.cast::<*const PropertyStoreVtbl>();
        for (pid, value) in properties {
            let value = wide_nul(value);
            let key = PropertyKey {
                fmtid: APP_USER_MODEL,
                pid,
            };
            let variant = StringVariant {
                vt: VT_LPWSTR,
                reserved: [0; 3],
                value: value.as_ptr(),
                padding: 0,
            };
            (vtbl.set_value)(store, &key, &variant);
        }
        (vtbl.commit)(store);
        (vtbl.release)(store);
    }
}

pub(crate) fn native_scale_for(window: &gpui::Window) -> f32 {
    window.scale_factor()
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

pub(crate) const GLYPH_STEPS_PER_PIXEL: f32 = 1.0;

pub(crate) const RESIZE_BY_WINDOW_MANAGER: bool = false;

pub(crate) const DRAWS_RESIZE_ZONES: bool = true;

pub(crate) fn display_scale_factor(display: gpui::DisplayId) -> f32 {
    qol_windowing::platform::windows::monitors()
        .get(u32::from(display) as usize)
        .map_or(1.0, |monitor| monitor.scale)
}

pub(crate) fn monitor_scale(monitor: &gpui::Bounds<gpui::Pixels>) -> f32 {
    let wanted = [
        monitor.origin.x.to_f64(),
        monitor.origin.y.to_f64(),
        monitor.size.width.to_f64(),
        monitor.size.height.to_f64(),
    ]
    .map(f64::round);
    qol_windowing::platform::windows::monitors()
        .into_iter()
        .find(|candidate| {
            let bounds = candidate.bounds;
            [bounds.x, bounds.y, bounds.width, bounds.height].map(f64::round) == wanted
        })
        .or_else(|| {
            let center = monitor.center();
            qol_windowing::platform::windows::monitor_at(
                center.x.to_f64().round() as i32,
                center.y.to_f64().round() as i32,
            )
        })
        .map_or(1.0, |candidate| candidate.scale)
}
