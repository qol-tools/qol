use gpui::{Context, Window};

pub fn hide_in_context<T>(window: &mut Window, _cx: &mut Context<T>) {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        _cx.hide();
    }
    #[cfg(not(target_os = "macos"))]
    {
        window.remove_window();
    }
}
