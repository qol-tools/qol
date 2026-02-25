use gpui::*;

pub(crate) fn open_keepalive(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1.0), px(1.0)), cx);
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: false,
            show: false,
            ..Default::default()
        },
        |_window, cx| cx.new(|_cx| KeepAlive),
    );
}

struct KeepAlive;

impl Render for KeepAlive {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
