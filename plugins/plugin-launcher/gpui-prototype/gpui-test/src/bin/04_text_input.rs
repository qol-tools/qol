// Test: Manual text input via keystroke capture
// Verifies: on_key_down, KeyDownEvent, keystroke handling

use gpui::*;

actions!(test, [Quit]);

struct InputView {
    query: String,
    focus_handle: FocusHandle,
}

impl InputView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            query: String::new(),
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for InputView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for InputView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("input-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let key = &event.keystroke.key;

                if key == "backspace" {
                    this.query.pop();
                    cx.notify();
                } else if key == "space" {
                    this.query.push(' ');
                    cx.notify();
                } else if key.len() == 1 {
                    let ch = key.chars().next().unwrap();
                    if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                        if event.keystroke.modifiers.shift {
                            this.query.push(ch.to_ascii_uppercase());
                        } else {
                            this.query.push(ch);
                        }
                        cx.notify();
                    }
                }
            }))
            .child(
                div()
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_4()
                    .gap_2()
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .text_size(px(16.))
                            .child("›"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(0xcdd6f4))
                            .text_size(px(16.))
                            .child(if self.query.is_empty() {
                                "Type to search... (Esc to quit)".to_string()
                            } else {
                                self.query.clone()
                            }),
                    ),
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
            let view = cx.new(|cx| InputView::new(cx));
            window.focus(&view.focus_handle(cx));
            view
        }).unwrap();

        cx.activate(true);
    });
}
