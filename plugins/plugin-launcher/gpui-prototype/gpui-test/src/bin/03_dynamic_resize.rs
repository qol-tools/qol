// Test: Dynamic window resize based on content
// Verifies: window.resize() during render, height changes with items

use gpui::*;

actions!(test, [Quit]);

struct ResizeView {
    items: Vec<String>,
    focus_handle: FocusHandle,
}

impl ResizeView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: vec![],
            focus_handle: cx.focus_handle(),
        }
    }

    fn update_height(&self, window: &mut Window) {
        let header_height = 42.0;
        let item_height = 32.0;
        let max_items = 8;
        let visible = self.items.len().min(max_items);
        let total = header_height + (visible as f32 * item_height);
        window.resize(size(px(600.), px(total)));
    }
}

impl Focusable for ResizeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ResizeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_height(window);

        div()
            .id("resize-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                match event.keystroke.key.as_str() {
                    "a" => {
                        let n = this.items.len() + 1;
                        this.items.push(format!("Item {}", n));
                        cx.notify();
                    }
                    "c" => {
                        this.items.clear();
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_4()
                    .border_b_1()
                    .border_color(rgb(0x45475a))
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_size(px(14.))
                            .child(format!("Items: {} | A=add, C=clear, Esc=quit", self.items.len())),
                    ),
            )
            .children(
                self.items.iter().enumerate().take(8).map(|(i, item)| {
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
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| ResizeView::new(cx));
            window.focus(&view.focus_handle(cx));
            view
        }).unwrap();

        cx.activate(true);
    });
}
