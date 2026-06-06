mod platform;

use std::cell::Cell;

pub use platform::{
    configure_popup_window, disable_window_shadow, dump_ghost_windows, hide_window_by_title,
    reposition_window_by_title, set_ghost_debug, show_window_by_title, window_backing_scale,
};

#[cfg(target_os = "linux")]
pub use platform::{hide_window_invisible, set_window_bounds_by_title};

thread_local! {
    static CHANGE_REASON: Cell<&'static str> = const { Cell::new("?") };
}

pub struct ReasonScope(&'static str);

pub fn reason_scope(reason: &'static str) -> ReasonScope {
    ReasonScope(CHANGE_REASON.with(|cell| cell.replace(reason)))
}

impl Drop for ReasonScope {
    fn drop(&mut self) {
        CHANGE_REASON.with(|cell| cell.set(self.0));
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn change_reason() -> &'static str {
    CHANGE_REASON.with(|cell| cell.get())
}
