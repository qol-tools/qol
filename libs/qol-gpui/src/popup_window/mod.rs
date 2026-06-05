mod platform;

pub use platform::{
    configure_popup_window, disable_window_shadow, dump_ghost_windows, hide_window_by_title,
    reposition_window_by_title, set_ghost_debug, show_window_by_title, window_backing_scale,
};

#[cfg(target_os = "linux")]
pub use platform::{hide_window_invisible, set_window_bounds_by_title};
