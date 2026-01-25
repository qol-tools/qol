// Test: Borderless popup window (no titlebar, no decorations)
// Verifies: WindowDecorations::Client, WindowKind::PopUp

use gpui::*;
use gpui_test::open_window_with_focus;

actions!(test, [Quit]);

struct PopupView {
    focus_handle: FocusHandle,
}

impl PopupView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for PopupView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PopupView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
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

        open_window_with_focus(cx, options, |_window, cx| PopupView::new(cx)).unwrap();
        cx.activate(true);
    });
}
