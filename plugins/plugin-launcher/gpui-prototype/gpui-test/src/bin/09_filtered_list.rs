// Test: Text input with filtered list
// Verifies: Combined input + dynamic list filtering (core launcher pattern)

use gpui::*;

actions!(test, [Quit]);

struct FilterView {
    query: String,
    items: Vec<String>,
    selected: usize,
    focus_handle: FocusHandle,
}

impl FilterView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            query: String::new(),
            items: vec![
                "Firefox".into(),
                "Chrome".into(),
                "Visual Studio Code".into(),
                "Slack".into(),
                "Discord".into(),
                "Spotify".into(),
                "Terminal".into(),
                "Files".into(),
                "Settings".into(),
                "Calculator".into(),
            ],
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn filtered_items(&self) -> Vec<&String> {
        if self.query.is_empty() {
            self.items.iter().collect()
        } else {
            let q = self.query.to_lowercase();
            self.items.iter().filter(|item| item.to_lowercase().contains(&q)).collect()
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        let max = self.filtered_items().len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }
}

impl Focusable for FilterView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FilterView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered = self.filtered_items();
        let max_visible = 8;

        div()
            .id("filter-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let key = &event.keystroke.key;

                match key.as_str() {
                    "up" => { this.move_up(); cx.notify(); }
                    "down" => { this.move_down(); cx.notify(); }
                    "enter" => {
                        let filtered = this.filtered_items();
                        if let Some(item) = filtered.get(this.selected) {
                            println!("Selected: {}", item);
                        }
                    }
                    "backspace" => {
                        this.query.pop();
                        this.selected = 0;
                        cx.notify();
                    }
                    "space" => {
                        this.query.push(' ');
                        this.selected = 0;
                        cx.notify();
                    }
                    _ if key.len() == 1 => {
                        let ch = key.chars().next().unwrap();
                        if ch.is_alphanumeric() {
                            if event.keystroke.modifiers.shift {
                                this.query.push(ch.to_ascii_uppercase());
                            } else {
                                this.query.push(ch);
                            }
                            this.selected = 0;
                            cx.notify();
                        }
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
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(0x45475a))
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .text_size(px(16.))
                            .child(">"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(0xcdd6f4))
                            .text_size(px(16.))
                            .child(if self.query.is_empty() {
                                "Type to search...".to_string()
                            } else {
                                self.query.clone()
                            }),
                    ),
            )
            .children(
                filtered.iter().enumerate().take(max_visible).map(|(i, item)| {
                    let is_selected = i == self.selected;
                    div()
                        .h(px(32.))
                        .w_full()
                        .flex()
                        .items_center()
                        .px_4()
                        .bg(if is_selected { rgb(0x45475a) } else { rgb(0x1e1e2e) })
                        .child(
                            div()
                                .text_color(if is_selected { rgb(0xcdd6f4) } else { rgb(0xa6adc8) })
                                .text_size(px(14.))
                                .child((*item).clone()),
                        )
                })
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let height = 42.0 + (8.0 * 32.0);
        let bounds = Bounds::centered(None, size(px(500.), px(height)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| FilterView::new(cx));
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        }).unwrap();

        cx.activate(true);
    });
}
