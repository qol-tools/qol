use gpui::*;

actions!(test, [Quit]);

struct IconListView {
    items: Vec<ListItemData>,
    selected: usize,
    focus_handle: FocusHandle,
}

struct ListItemData {
    label: String,
    icon_color: Rgba,
}

impl IconListView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: vec![
                ListItemData { label: "Firefox".into(), icon_color: Rgba { r: 1.0, g: 0.3, b: 0.0, a: 1.0 } },
                ListItemData { label: "Chrome".into(), icon_color: Rgba { r: 0.2, g: 0.6, b: 0.9, a: 1.0 } },
                ListItemData { label: "VS Code".into(), icon_color: Rgba { r: 0.0, g: 0.5, b: 0.8, a: 1.0 } },
                ListItemData { label: "Slack".into(), icon_color: Rgba { r: 0.4, g: 0.1, b: 0.5, a: 1.0 } },
                ListItemData { label: "Discord".into(), icon_color: Rgba { r: 0.4, g: 0.4, b: 0.9, a: 1.0 } },
                ListItemData { label: "Spotify".into(), icon_color: Rgba { r: 0.1, g: 0.8, b: 0.3, a: 1.0 } },
                ListItemData { label: "Terminal".into(), icon_color: Rgba { r: 0.2, g: 0.2, b: 0.2, a: 1.0 } },
                ListItemData { label: "Files".into(), icon_color: Rgba { r: 0.9, g: 0.7, b: 0.2, a: 1.0 } },
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

impl Focusable for IconListView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for IconListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("icon-list")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                match event.keystroke.key.as_str() {
                    "up" => { this.move_up(); cx.notify(); }
                    "down" => { this.move_down(); cx.notify(); }
                    "enter" => {
                        println!("Selected: {}", this.items[this.selected].label);
                    }
                    _ => {}
                }
            }))
            .children(self.items.iter().enumerate().map(|(i, item)| {
                let is_selected = i == self.selected;
                div()
                    .h(px(40.))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_3()
                    .gap_3()
                    .bg(if is_selected { rgb(0x45475a) } else { rgb(0x1e1e2e) })
                    .child(
                        div()
                            .w(px(24.))
                            .h(px(24.))
                            .rounded_md()
                            .bg(item.icon_color)
                    )
                    .child(
                        div()
                            .text_color(if is_selected { rgb(0xcdd6f4) } else { rgb(0xa6adc8) })
                            .text_size(px(14.))
                            .child(item.label.clone())
                    )
            }))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(400.), px(320.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| IconListView::new(cx));
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        }).unwrap();

        cx.activate(true);
    });
}
