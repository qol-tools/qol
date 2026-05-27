use gpui::*;

pub fn open_keepalive(cx: &mut App, app_id: Option<&str>) -> Option<AnyWindowHandle> {
    let bounds = Bounds::centered(None, size(px(1.0), px(1.0)), cx);
    let mut options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: false,
        show: false,
        ..Default::default()
    };
    if let Some(id) = app_id {
        options.app_id = Some(id.to_string());
    }
    cx.open_window(options, |_window, cx| cx.new(|_cx| KeepAlive))
        .ok()
        .map(|h| h.into())
}

struct KeepAlive;

impl Render for KeepAlive {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
