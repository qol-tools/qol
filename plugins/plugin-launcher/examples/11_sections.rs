use gpui::*;
use launcher::open_window_with_focus;

actions!(test, [Quit]);

struct SectionEntry {
    label: String,
    is_header: bool,
}

struct SectionListView {
    entries: Vec<SectionEntry>,
    selected: usize,
    focus_handle: FocusHandle,
}

impl SectionListView {
    fn new(cx: &mut Context<Self>) -> Self {
        let entries = vec![
            SectionEntry { label: "Apps".into(), is_header: true },
            SectionEntry { label: "Firefox".into(), is_header: false },
            SectionEntry { label: "Chrome".into(), is_header: false },
            SectionEntry { label: "Terminal".into(), is_header: false },
            SectionEntry { label: "Files".into(), is_header: true },
            SectionEntry { label: "Home".into(), is_header: false },
            SectionEntry { label: "Downloads".into(), is_header: false },
            SectionEntry { label: "Projects".into(), is_header: false },
            SectionEntry { label: "Web".into(), is_header: true },
            SectionEntry { label: "qol.tools".into(), is_header: false },
            SectionEntry { label: "Docs".into(), is_header: false },
        ];

        let selected = entries.iter().position(|e| !e.is_header).unwrap_or(0);

        Self {
            entries,
            selected,
            focus_handle: cx.focus_handle(),
        }
    }

    fn move_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let mut index = self.selected;
        while index > 0 {
            index -= 1;
            if !self.entries[index].is_header {
                self.selected = index;
                return;
            }
        }
    }

    fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let mut index = self.selected;
        while index + 1 < self.entries.len() {
            index += 1;
            if !self.entries[index].is_header {
                self.selected = index;
                return;
            }
        }
    }
}

impl Focusable for SectionListView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SectionListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header_height = 26.0;
        let item_height = 36.0;
        div()
            .id("section-list")
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
                        if let Some(entry) = this.entries.get(this.selected) {
                            if !entry.is_header {
                                println!("Selected: {}", entry.label);
                            }
                        }
                    }
                    _ => {}
                }
            }))
            .children(self.entries.iter().enumerate().map(|(i, entry)| {
                if entry.is_header {
                    div()
                        .h(px(header_height))
                        .w_full()
                        .flex()
                        .items_center()
                        .px_4()
                        .border_b_1()
                        .border_color(rgb(0x313244))
                        .child(
                            div()
                                .text_color(rgb(0x6c7086))
                                .text_size(px(12.))
                                .child(entry.label.clone())
                        )
                } else {
                    let is_selected = i == self.selected;
                    div()
                        .h(px(item_height))
                        .w_full()
                        .flex()
                        .items_center()
                        .px_4()
                        .bg(if is_selected { rgb(0x45475a) } else { rgb(0x1e1e2e) })
                        .child(
                            div()
                                .text_color(if is_selected { rgb(0xcdd6f4) } else { rgb(0xa6adc8) })
                                .text_size(px(14.))
                                .child(entry.label.clone())
                        )
                }
            }))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(520.), px(360.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| SectionListView::new(cx)).unwrap();

        cx.activate(true);
    });
}
