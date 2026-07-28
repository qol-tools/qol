// Test: List with selection highlight and keyboard navigation
// Verifies: Up/Down navigation, selection state, visual highlight

use gpui::*;
use launcher::open_window_with_focus;

actions!(test, [Quit]);

struct ListView {
    items: Vec<String>,
    selected: usize,
    focus_handle: FocusHandle,
}

impl ListView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: vec![
                "Firefox".into(),
                "Chrome".into(),
                "Visual Studio Code".into(),
                "Slack".into(),
                "Discord".into(),
                "Spotify".into(),
                "Terminal".into(),
                "Files".into(),
            ],
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected < self.items.len() - 1 {
            self.selected += 1;
        }
    }
}

impl Focusable for ListView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("list-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                match event.keystroke.key.as_str() {
                    "up" => {
                        this.move_up();
                        cx.notify();
                    }
                    "down" => {
                        this.move_down();
                        cx.notify();
                    }
                    "enter" => {
                        println!("Selected: {}", this.items[this.selected]);
                    }
                    _ => {}
                }
            }))
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
                            .child("↑/↓ navigate, Enter select, Esc quit"),
                    ),
            )
            .children(self.items.iter().enumerate().map(|(i, item)| {
                let is_selected = i == self.selected;
                div()
                    .h(px(32.))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_4()
                    .bg(if is_selected {
                        rgb(0x45475a)
                    } else {
                        rgb(0x1e1e2e)
                    })
                    .child(
                        div()
                            .text_color(if is_selected {
                                rgb(0xcdd6f4)
                            } else {
                                rgb(0xa6adc8)
                            })
                            .text_size(px(14.))
                            .child(item.clone()),
                    )
            }))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let item_count = 8;
        let height = 36.0 + (item_count as f32 * 32.0);
        let bounds = Bounds::centered(None, size(px(400.), px(height)), cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| ListView::new(cx)).unwrap();

        cx.activate(true);
    });
}
