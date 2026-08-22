#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(super) trait PopupPresentation {
    fn present_topmost(title: &str);
    fn restore_composite(title: &str);
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use fallback::Platform as ActivePlatform;
#[cfg(target_os = "linux")]
use linux::Platform as ActivePlatform;
#[cfg(target_os = "macos")]
use macos::Platform as ActivePlatform;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use fallback::{
    capture_focus_return, configure_keepalive_window, configure_overlay_window,
    configure_pinned_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    focus_window_by_title, hide_for_capture, hide_invisible, hide_window_by_title,
    hide_windows_by_title_prefix, make_override_redirect, park_window_by_title, pinned_window_kind,
    prepare_window_reveal_by_title, release_focus_by_title, reposition_window_by_title,
    set_ghost_debug, set_unmap_hide, set_window_fixed_size_by_title, set_window_type_dock_by_title,
    show_normal_window_by_title, show_window_by_title, show_window_passive_by_title,
    sync_window_layout, sync_window_layout_by_title, visible_windows_by_title_prefix,
    window_backing_scale, window_bounds_primary_anchored, window_geometry_session,
    window_holds_input_focus, window_position_by_title, WindowGeometrySession,
};
#[cfg(target_os = "linux")]
pub use linux::{
    capture_focus_return, configure_keepalive_window, configure_overlay_window,
    configure_pinned_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    focus_window_by_title, hide_for_capture, hide_invisible, hide_window_by_title,
    hide_windows_by_title_prefix, make_override_redirect, park_window_by_title, pinned_window_kind,
    prepare_window_reveal_by_title, release_focus_by_title, reposition_window_by_title,
    set_ghost_debug, set_unmap_hide, set_window_fixed_size_by_title, set_window_type_dock_by_title,
    show_normal_window_by_title, show_window_by_title, show_window_passive_by_title,
    sync_window_layout, sync_window_layout_by_title, visible_windows_by_title_prefix,
    window_backing_scale, window_bounds_primary_anchored, window_geometry_session,
    window_holds_input_focus, window_position_by_title, WindowGeometrySession,
};
#[cfg(target_os = "macos")]
pub use macos::{
    capture_focus_return, configure_keepalive_window, configure_overlay_window,
    configure_pinned_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    focus_window_by_title, hide_for_capture, hide_invisible, hide_window_by_title,
    hide_windows_by_title_prefix, make_override_redirect, park_window_by_title, pinned_window_kind,
    prepare_window_reveal_by_title, reassert_focus_on_main, release_focus_by_title,
    reposition_window_by_title, set_ghost_debug, set_unmap_hide, set_window_fixed_size_by_title,
    set_window_type_dock_by_title, show_normal_window_by_title, show_window_by_title,
    show_window_passive_by_title, sync_window_layout, sync_window_layout_by_title,
    visible_windows_by_title_prefix, window_backing_scale, window_bounds_primary_anchored,
    window_geometry_session, window_holds_input_focus, window_position_by_title,
    WindowGeometrySession,
};

pub fn present_topmost(title: &str) {
    <ActivePlatform as PopupPresentation>::present_topmost(title);
}

pub fn restore_composite(title: &str) {
    <ActivePlatform as PopupPresentation>::restore_composite(title);
}
