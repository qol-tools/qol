use gpui::*;
use launcher::{fuzzy_match, open_window_with_focus, FuzzyMatch};

actions!(test, [Quit]);

struct FuzzyView {
    query: String,
    items: Vec<String>,
    selected: usize,
    focus_handle: FocusHandle,
}

impl FuzzyView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            query: String::new(),
            items: vec![
                "Firefox",
                "Google Chrome",
                "Visual Studio Code",
                "Slack",
                "Discord",
                "Spotify",
                "Terminal Emulator",
                "Files Manager",
                "System Settings",
                "Calculator",
                "LibreOffice Writer",
                "GIMP Image Editor",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn matched_items(&self) -> Vec<(String, FuzzyMatch)> {
        if self.query.is_empty() {
            return self
                .items
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        FuzzyMatch {
                            score: 0,
                            positions: vec![],
                        },
                    )
                })
                .collect();
        }

        let mut results: Vec<_> = self
            .items
            .iter()
            .filter_map(|name| fuzzy_match(&self.query, name).map(|m| (name.clone(), m)))
            .collect();
        results.sort_by_key(|(_, m)| m.score);
        results
    }
}

impl Focusable for FuzzyView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn render_highlighted(name: &str, positions: &[usize], selected: bool) -> Div {
    let highlight = rgb(0xf9e2af);
    let normal = if selected {
        rgb(0xcdd6f4)
    } else {
        rgb(0xa6adc8)
    };
    let chars: Vec<char> = name.chars().collect();

    let mut container = div().flex().flex_row();
    let mut i = 0;

    while i < chars.len() {
        let matched = positions.contains(&i);
        let mut segment = String::new();
        while i < chars.len() && positions.contains(&i) == matched {
            segment.push(chars[i]);
            i += 1;
        }
        container = container.child(
            div()
                .text_size(px(14.))
                .text_color(if matched { highlight } else { normal })
                .child(segment),
        );
    }

    container
}

impl Render for FuzzyView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let matched = self.matched_items();
        let max_visible = 8;

        div()
            .id("fuzzy-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let key = &event.keystroke.key;
                match key.as_str() {
                    "up" => {
                        if this.selected > 0 {
                            this.selected -= 1;
                        }
                        cx.notify();
                    }
                    "down" => {
                        let max = this.matched_items().len().saturating_sub(1);
                        if this.selected < max {
                            this.selected += 1;
                        }
                        cx.notify();
                    }
                    "enter" => {
                        let matched = this.matched_items();
                        if let Some((name, m)) = matched.get(this.selected) {
                            println!("Launched: {} (score: {})", name, m.score);
                        }
                        this.selected = 0;
                        this.query.clear();
                        cx.notify();
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
                                "Type to search (try: vsc, ff, gie)...".to_string()
                            } else {
                                self.query.clone()
                            }),
                    ),
            )
            .children(
                matched
                    .iter()
                    .enumerate()
                    .take(max_visible)
                    .map(|(i, (name, m))| {
                        let is_selected = i == self.selected;
                        div()
                            .h(px(32.))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .bg(if is_selected {
                                rgb(0x45475a)
                            } else {
                                rgb(0x1e1e2e)
                            })
                            .child(render_highlighted(name, &m.positions, is_selected))
                            .child(div().text_color(rgb(0x585b70)).text_size(px(11.)).child(
                                if self.query.is_empty() {
                                    String::new()
                                } else {
                                    format!("{}", m.score)
                                },
                            ))
                    }),
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

        open_window_with_focus(cx, options, |_window, cx| FuzzyView::new(cx)).unwrap();
        cx.activate(true);
    });
}
