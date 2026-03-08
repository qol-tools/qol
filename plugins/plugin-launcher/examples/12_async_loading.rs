use gpui::*;
use launcher::open_window_with_focus;
use std::time::Duration;

actions!(test, [Quit]);

struct IndexingView {
    query: String,
    items: Vec<String>,
    selected: usize,
    indexed: usize,
    total: usize,
    focus_handle: FocusHandle,
    started: bool,
}

impl IndexingView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            query: String::new(),
            items: Vec::new(),
            selected: 0,
            indexed: 0,
            total: 60,
            focus_handle: cx.focus_handle(),
            started: false,
        }
    }

    fn start_indexing(&mut self, cx: &mut Context<Self>) {
        if self.started {
            return;
        }
        self.started = true;

        let total = self.total;
        let mut all_items = Vec::with_capacity(total);
        for i in 0..total {
            all_items.push(format!("Item {}", i + 1));
        }

        let task = cx.spawn(move |this: WeakEntity<IndexingView>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                for item in all_items {
                    async_cx
                        .background_executor()
                        .timer(Duration::from_millis(40))
                        .await;
                    let next = item.clone();
                    this.update(&mut async_cx, |view, cx| {
                        view.items.push(next);
                        view.indexed = view.items.len();
                        if view.selected >= view.filtered_items().len() {
                            view.selected = view.filtered_items().len().saturating_sub(1);
                        }
                        cx.notify();
                    })
                    .ok();
                }
            }
        });
        task.detach();
    }

    fn filtered_items(&self) -> Vec<&String> {
        if self.query.is_empty() {
            self.items.iter().collect()
        } else {
            let q = self.query.to_lowercase();
            self.items
                .iter()
                .filter(|item| item.to_lowercase().contains(&q))
                .collect()
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

impl Focusable for IndexingView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for IndexingView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_indexing(cx);
        let filtered = self.filtered_items();
        let status = if self.indexed < self.total {
            format!("Indexing… {}/{}", self.indexed, self.total)
        } else {
            "Indexing complete".to_string()
        };

        div()
            .id("indexing-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let key = &event.keystroke.key;

                match key.as_str() {
                    "up" => {
                        this.move_up();
                        cx.notify();
                    }
                    "down" => {
                        this.move_down();
                        cx.notify();
                    }
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
            .child(
                div()
                    .h(px(26.))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_4()
                    .text_color(rgb(0x6c7086))
                    .text_size(px(12.))
                    .child(status),
            )
            .children(filtered.iter().enumerate().take(8).map(|(i, item)| {
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
                            .child((*item).clone()),
                    )
            }))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(500.), px(360.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| IndexingView::new(cx)).unwrap();

        cx.activate(true);
    });
}
