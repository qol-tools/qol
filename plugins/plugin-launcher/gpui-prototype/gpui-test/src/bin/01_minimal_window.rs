// Test: Minimal 42px window with Escape to quit
// Verifies: Basic gpui setup, window creation, action handling

use gpui::*;
use gpui_test::open_window_with_focus;

actions!(test, [Quit]);

struct MinimalView {
    focus_handle: FocusHandle,
}

impl MinimalView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for MinimalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MinimalView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_color(rgb(0xcdd6f4))
                    .text_size(px(16.))
                    .child("Minimal 42px window - Press Escape to quit"),
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
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| MinimalView::new(cx)).unwrap();
        cx.activate(true);
    });
}
