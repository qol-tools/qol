use gpui::{Context, Window};

pub fn hide_in_context<T>(window: &mut Window, _cx: &mut Context<T>) {
    // On all platforms, hide the launcher by removing the window.
    // On macOS, cx.hide() hides the entire NSApplication which causes
    // crashes when the daemon later tries to re-activate the window.
    // minimize_window() leaves a dock icon flash. remove_window() is
    // the cleanest approach for a popup-style launcher.
    let _ = _cx;
    window.remove_window();
}
