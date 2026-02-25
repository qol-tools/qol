use gpui::*;

use super::LAUNCHER_APP_ID;

pub(crate) fn open_keepalive_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1.), px(1.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: false,
        show: false,
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    };

    let _ = cx.open_window(options, |_window, cx| {
        cx.new(|_cx| KeepAliveView)
    });
}

struct KeepAliveView;

impl Render for KeepAliveView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
