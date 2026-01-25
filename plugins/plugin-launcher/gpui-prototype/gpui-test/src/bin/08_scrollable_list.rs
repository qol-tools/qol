// Test: Scrollable list with overflow
// Verifies: overflow_y_scroll, scroll behavior

use gpui::*;
use gpui_test::open_window_with_focus;

actions!(test, [Quit]);

struct ScrollView {
    items: Vec<String>,
    focus_handle: FocusHandle,
}

impl ScrollView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: (1..=50).map(|i| format!("Item {}", i)).collect(),
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for ScrollView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ScrollView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("scroll-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .child(
                div()
                    .h(px(36.))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_4()
                    .border_b_1()
                    .border_color(rgb(0x45475a))
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .text_size(px(12.))
                            .child("Scroll with mouse wheel, Esc to quit"),
                    ),
            )
            .child(
                div()
                    .id("scroll-container")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(
                        self.items.iter().enumerate().map(|(i, item)| {
                            div()
                                .h(px(32.))
                                .w_full()
                                .flex()
                                .items_center()
                                .px_4()
                                .bg(if i % 2 == 0 { rgb(0x1e1e2e) } else { rgb(0x252536) })
                                .child(
                                    div()
                                        .text_color(rgb(0xcdd6f4))
                                        .text_size(px(14.))
                                        .child(item.clone()),
                                )
                        })
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(400.), px(300.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| ScrollView::new(cx)).unwrap();

        cx.activate(true);
    });
}
