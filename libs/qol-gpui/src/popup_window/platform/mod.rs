#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use fallback::{
    configure_overlay_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    hide_for_capture, hide_invisible, hide_window_by_title, hide_windows_by_title_prefix,
    reposition_window_by_title, set_ghost_debug, show_window_by_title, sync_window_layout,
    visible_windows_by_title_prefix, window_backing_scale,
};
#[cfg(target_os = "linux")]
pub use linux::{
    configure_overlay_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    force_composite_below, hide_for_capture, hide_invisible, hide_window_by_title,
    hide_windows_by_title_prefix, make_override_redirect, reposition_window_by_title,
    restore_composite, set_ghost_debug, show_window_by_title, sync_window_layout,
    visible_windows_by_title_prefix, window_backing_scale,
};
#[cfg(target_os = "macos")]
pub use macos::{
    configure_overlay_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    hide_for_capture, hide_invisible, hide_window_by_title, hide_windows_by_title_prefix,
    reposition_window_by_title, set_ghost_debug, show_window_by_title, sync_window_layout,
    visible_windows_by_title_prefix, window_backing_scale,
};
