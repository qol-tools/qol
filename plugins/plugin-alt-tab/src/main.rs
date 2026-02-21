use gpui::*;

struct AltTabWindow {
    // Window state
}

impl Render for AltTabWindow {
    fn render(&mut self, _cx: &mut ViewContext<Self>) -> impl IntoElement {
        div()
            .flex()
            .bg(rgb(0x1e1e1e))
            .w_full()
            .h_full()
            .items_center()
            .justify_center()
            .child(div().text_color(rgb(0xffffff)).child("Alt Tab Window"))
    }
}

fn main() {
    let app = App::new();

    app.run(move |cx: &mut AppContext| {
        cx.open_window(
            WindowOptions {
                bounds: Some(Bounds::centered(None, size(px(800.0), px(600.0)), cx)),
                titlebar: None,
                window_bounds_edges_hittestable: false,
                focus: true,
                ..Default::default()
            },
            |cx| cx.new_view(|_cx| AltTabWindow {}),
        );
    });
}
