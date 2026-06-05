//! Reposition popup windows by title.
//!
//! ## macOS y-axis convention
//!
//! GPUI's coordinate origin is the top-left of the primary screen (the one
//! with the menu bar, `NSScreen::screens(mtm)[0]`). Cocoa's global display
//! coordinate space is anchored to the bottom-left of that SAME screen.
//! Conversion: `ns_y = primary_screen_height - gpui_y`.
//!
//! Do NOT substitute `NSScreen::mainScreen()` here. `mainScreen` is "the
//! screen containing the window with keyboard focus" and moves with focus,
//! producing an N-pixel drift between show and ghost on multi-monitor setups.

mod platform;

pub use platform::{
    configure_popup_window, disable_window_shadow, hide_window_by_title,
    reposition_window_by_title, set_ghost_debug, show_window_by_title, window_backing_scale,
};

#[cfg(target_os = "linux")]
pub use platform::set_window_bounds_by_title;
