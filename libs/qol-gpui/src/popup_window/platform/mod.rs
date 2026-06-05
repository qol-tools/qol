#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use fallback::{
    configure_popup_window, disable_window_shadow, dump_ghost_windows, hide_window_by_title,
    reposition_window_by_title, set_ghost_debug, show_window_by_title, window_backing_scale,
};
#[cfg(target_os = "linux")]
pub use linux::{
    configure_popup_window, disable_window_shadow, dump_ghost_windows, hide_window_by_title,
    hide_window_invisible, reposition_window_by_title, set_ghost_debug, set_window_bounds_by_title,
    show_window_by_title, window_backing_scale,
};
#[cfg(target_os = "macos")]
pub use macos::{
    configure_popup_window, disable_window_shadow, dump_ghost_windows, hide_window_by_title,
    reposition_window_by_title, set_ghost_debug, show_window_by_title, window_backing_scale,
};
