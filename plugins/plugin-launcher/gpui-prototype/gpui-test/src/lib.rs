use gpui::*;

pub fn open_window_with_focus<T, F>(
    cx: &mut App,
    options: WindowOptions,
    build: F,
) -> Result<WindowHandle<T>>
where
    T: Render + Focusable + 'static,
    F: FnOnce(&mut Window, &mut Context<T>) -> T + 'static,
{
    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| build(window, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    })
}
