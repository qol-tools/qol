use std::ffi::c_int;

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

pub fn square_window_corners(_window: &mut gpui::Window) {}

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
