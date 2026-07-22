#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use fallback::{
    capture_focus_return, configure_keepalive_window, configure_overlay_window,
    configure_pinned_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    hide_for_capture, hide_invisible, hide_window_by_title, hide_windows_by_title_prefix,
    park_window_by_title, pinned_window_kind, prepare_window_reveal_by_title,
    reposition_window_by_title, set_ghost_debug, set_window_fixed_size_by_title,
    set_window_type_dock_by_title, show_normal_window_by_title, show_window_by_title,
    show_window_passive_by_title, sync_window_layout, visible_windows_by_title_prefix,
    window_backing_scale, window_holds_input_focus,
};
#[cfg(target_os = "linux")]
pub use linux::{
    capture_focus_return, configure_keepalive_window, configure_overlay_window,
    configure_pinned_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    focus_window_by_title, force_composite_below, hide_for_capture, hide_invisible,
    hide_window_by_title, hide_windows_by_title_prefix, make_override_redirect,
    park_window_by_title, pinned_window_kind, prepare_window_reveal_by_title,
    release_focus_by_title, reposition_window_by_title, restore_composite, set_ghost_debug,
    set_window_fixed_size_by_title, set_window_type_dock_by_title, show_normal_window_by_title,
    show_window_by_title, show_window_passive_by_title, sync_window_layout,
    visible_windows_by_title_prefix, window_backing_scale, window_geometry_session,
    window_holds_input_focus, window_position_by_title, WindowGeometrySession,
};
#[cfg(target_os = "macos")]
pub use macos::{
    capture_focus_return, configure_keepalive_window, configure_overlay_window,
    configure_pinned_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    hide_for_capture, hide_invisible, hide_window_by_title, hide_windows_by_title_prefix,
    park_window_by_title, pinned_window_kind, prepare_window_reveal_by_title,
    reposition_window_by_title, set_ghost_debug, set_window_fixed_size_by_title,
    set_window_type_dock_by_title, show_normal_window_by_title, show_window_by_title,
    show_window_passive_by_title, sync_window_layout, visible_windows_by_title_prefix,
    window_backing_scale, window_holds_input_focus,
};
