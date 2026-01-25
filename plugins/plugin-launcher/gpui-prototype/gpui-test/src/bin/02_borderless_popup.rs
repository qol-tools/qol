// Test: Borderless popup window (no titlebar, no decorations)
// Verifies: WindowDecorations::Client, WindowKind::PopUp

use gpui::*;

actions!(test, [Quit]);

struct PopupView;

impl Render for PopupView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .border_1()
            .border_color(rgb(0x45475a))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_color(rgb(0xcdd6f4))
                    .text_size(px(16.))
                    .child("Borderless popup - Press Escape to quit"),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(600.), px(42.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(200.), px(20.))),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        cx.open_window(options, |_, cx| cx.new(|_| PopupView)).unwrap();
        cx.activate(true);
    });
}
