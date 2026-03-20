use gpui::*;
use launcher::open_window_with_focus;
use std::collections::HashMap;

actions!(test, [Quit]);

const SECS_PER_DAY: u64 = 86400;
const NOW: u64 = 1_000_000_000;
const HALF_LIFE_DAYS: f64 = 7.0;
const FREQUENCY_BONUS: i32 = 500;

struct FrequencyEntry {
    count: u32,
    last_accessed: u64,
}

fn effective_count(entry: &FrequencyEntry, half_life_days: f64) -> f64 {
    let days_elapsed = NOW.saturating_sub(entry.last_accessed) as f64 / SECS_PER_DAY as f64;
    let decay = (-days_elapsed * 0.693 / half_life_days).exp();
    entry.count as f64 * decay
}

struct ScoredItem {
    name: String,
    score: i32,
}

struct FrecencyView {
    query: String,
    items: Vec<String>,
    frequency: HashMap<String, FrequencyEntry>,
    selected: usize,
    focus_handle: FocusHandle,
}

impl FrecencyView {
    fn new(cx: &mut Context<Self>) -> Self {
        let items = vec![
            "Firefox",
            "Chrome",
            "VS Code",
            "Slack",
            "Discord",
            "Spotify",
            "Terminal",
            "Files",
            "Settings",
            "Calculator",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let mut frequency = HashMap::new();
        frequency.insert(
            "Terminal".into(),
            FrequencyEntry {
                count: 20,
                last_accessed: NOW,
            },
        );
        frequency.insert(
            "Firefox".into(),
            FrequencyEntry {
                count: 15,
                last_accessed: NOW,
            },
        );
        frequency.insert(
            "VS Code".into(),
            FrequencyEntry {
                count: 10,
                last_accessed: NOW - 3 * SECS_PER_DAY,
            },
        );
        frequency.insert(
            "Slack".into(),
            FrequencyEntry {
                count: 5,
                last_accessed: NOW - 14 * SECS_PER_DAY,
            },
        );

        Self {
            query: String::new(),
            items,
            frequency,
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn score_item(&self, name: &str) -> i32 {
        let n = name.to_lowercase();
        let q = self.query.to_lowercase();

        let match_penalty = if q.is_empty() || n == q {
            0
        } else if n.starts_with(&q) {
            100
        } else if n.contains(&q) {
            200
        } else {
            10000
        };

        let length_penalty = name.len() as i32;

        let frequency_bonus = self
            .frequency
            .get(name)
            .map(|e| (effective_count(e, HALF_LIFE_DAYS) * FREQUENCY_BONUS as f64) as i32)
            .unwrap_or(0);

        match_penalty + length_penalty - frequency_bonus
    }

    fn ranked_items(&self) -> Vec<ScoredItem> {
        let mut scored: Vec<_> = self
            .items
            .iter()
            .map(|name| ScoredItem {
                name: name.clone(),
                score: self.score_item(name),
            })
            .filter(|item| {
                if self.query.is_empty() {
                    return true;
                }
                let q = self.query.to_lowercase();
                item.name.to_lowercase().contains(&q)
            })
            .collect();
        scored.sort_by_key(|item| item.score);
        scored
    }

    fn record_launch(&mut self) {
        let ranked = self.ranked_items();
        if let Some(item) = ranked.get(self.selected) {
            let name = item.name.clone();
            let entry = self.frequency.entry(name).or_insert(FrequencyEntry {
                count: 0,
                last_accessed: NOW,
            });
            entry.count += 1;
            entry.last_accessed = NOW;
        }
    }
}

impl Focusable for FrecencyView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FrecencyView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ranked = self.ranked_items();
        let max_visible = 8;

        div()
            .id("frecency-view")
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
                        let max = this.ranked_items().len().saturating_sub(1);
                        if this.selected < max {
                            this.selected += 1;
                        }
                        cx.notify();
                    }
                    "enter" => {
                        let ranked = this.ranked_items();
                        if let Some(item) = ranked.get(this.selected) {
                            println!("Launched: {} (score: {})", item.name, item.score);
                        }
                        this.record_launch();
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
                                "Type to search...".to_string()
                            } else {
                                self.query.clone()
                            }),
                    ),
            )
            .children(
                ranked
                    .iter()
                    .enumerate()
                    .take(max_visible)
                    .map(|(i, item)| {
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
                            .child(
                                div()
                                    .text_color(if is_selected {
                                        rgb(0xcdd6f4)
                                    } else {
                                        rgb(0xa6adc8)
                                    })
                                    .text_size(px(14.))
                                    .child(item.name.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x585b70))
                                    .text_size(px(11.))
                                    .child(format!("{}", item.score)),
                            )
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

        open_window_with_focus(cx, options, |_window, cx| FrecencyView::new(cx)).unwrap();
        cx.activate(true);
    });
}
