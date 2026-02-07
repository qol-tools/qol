use gpui::*;
use gpui_test::desktop_entry::{self, DesktopEntry};
use gpui_test::monitor;
use gpui_test::{fuzzy_match, open_window_with_focus, FuzzyMatch};
use std::process::Command;

actions!(launcher, [Quit]);

const MAX_VISIBLE: usize = 8;
const HEADER_HEIGHT: f32 = 42.0;
const ROW_HEIGHT: f32 = 32.0;
const WINDOW_WIDTH: f32 = 500.0;

const BG: u32 = 0x1e1e2e;
const BG_SELECTED: u32 = 0x45475a;
const TEXT: u32 = 0xcdd6f4;
const TEXT_DIM: u32 = 0xa6adc8;
const TEXT_MUTED: u32 = 0x6c7086;
const HIGHLIGHT: u32 = 0xf9e2af;
const BORDER: u32 = 0x45475a;

struct LauncherView {
    query: String,
    entries: Vec<DesktopEntry>,
    selected: usize,
    window_height: f32,
    focus_handle: FocusHandle,
}

struct Scored<'a> {
    entry: &'a DesktopEntry,
    m: FuzzyMatch,
}

impl LauncherView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            query: String::new(),
            entries: desktop_entry::scan(&desktop_entry::default_dirs()),
            selected: 0,
            window_height: HEADER_HEIGHT,
            focus_handle: cx.focus_handle(),
        }
    }

    fn filtered(&self) -> Vec<Scored<'_>> {
        if self.query.trim().is_empty() {
            return Vec::new();
        }

        let mut results: Vec<Scored<'_>> = self
            .entries
            .iter()
            .filter_map(|entry| {
                fuzzy_match(&self.query, &entry.name).map(|m| Scored { entry, m })
            })
            .collect();
        results.sort_by_key(|s| s.m.score);
        results
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        let max = self.filtered().len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }

    fn launch(&self, cx: &mut Context<Self>) {
        let exec = self
            .filtered()
            .get(self.selected)
            .map(|s| s.entry.exec.clone());

        if let Some(exec) = exec {
            spawn_detached(&exec);
        }
        cx.quit();
    }

    fn resize_for_visible_rows(&mut self, visible: usize, window: &mut Window) {
        let target_height = HEADER_HEIGHT + (visible as f32 * ROW_HEIGHT);
        if (self.window_height - target_height).abs() > f32::EPSILON {
            window.resize(size(px(WINDOW_WIDTH), px(target_height)));
            self.window_height = target_height;
        }
    }

    fn handle_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke.key;
        match key.as_str() {
            "up" => self.move_up(),
            "down" => self.move_down(),
            "enter" => {
                self.launch(cx);
                return;
            }
            "backspace" => {
                self.query.pop();
                self.selected = 0;
            }
            "space" => {
                self.query.push(' ');
                self.selected = 0;
            }
            _ => {
                let Some(ch) = typeable_char(key, event.keystroke.modifiers.shift) else {
                    return;
                };
                self.query.push(ch);
                self.selected = 0;
            }
        }
        cx.notify();
    }
}

fn typeable_char(key: &str, shift: bool) -> Option<char> {
    let ch = key.chars().next().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')?;
    Some(if shift { ch.to_ascii_uppercase() } else { ch })
}

impl Focusable for LauncherView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.filtered().len().min(MAX_VISIBLE);
        let results_height = visible as f32 * ROW_HEIGHT;
        self.resize_for_visible_rows(visible, window);
        let filtered = self.filtered();

        div()
            .id("launcher")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .on_key_down(cx.listener(Self::handle_key))
            .child(search_bar(&self.query))
            .child(
                div()
                    .h(px(results_height))
                    .w_full()
                    .flex()
                    .flex_col()
                    .bg(rgb(BG))
                    .children(
                        filtered
                            .iter()
                            .enumerate()
                            .take(MAX_VISIBLE)
                            .map(|(i, scored)| result_row(scored, i == self.selected)),
                    ),
            )
    }
}

fn search_bar(query: &str) -> Div {
    let (text, color) = if query.is_empty() {
        ("Type to search...".to_string(), rgb(TEXT_MUTED))
    } else {
        (query.to_string(), rgb(TEXT))
    };

    div()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .px_4()
        .gap_2()
        .bg(rgb(BG))
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(div().text_color(rgb(TEXT_MUTED)).text_size(px(16.)).child(">"))
        .child(div().flex_1().text_color(color).text_size(px(16.)).child(text))
}

fn result_row(scored: &Scored<'_>, selected: bool) -> Div {
    let positions = &scored.m.positions;
    let base_color = if selected { rgb(TEXT) } else { rgb(TEXT_DIM) };
    let bg = if selected { rgb(BG_SELECTED) } else { rgb(BG) };

    let spans: Vec<AnyElement> = scored
        .entry
        .name
        .char_indices()
        .map(|(i, ch)| {
            let color = if positions.contains(&i) { rgb(HIGHLIGHT) } else { base_color };
            div()
                .text_color(color)
                .text_size(px(14.))
                .child(ch.to_string())
                .into_any_element()
        })
        .collect();

    div()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .px_4()
        .bg(bg)
        .children(spans)
}

fn spawn_detached(exec: &str) {
    let mut parts = exec.split_whitespace();
    let Some(cmd) = parts.next() else { return };
    let _ = Command::new(cmd).args(parts).spawn();
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let height = HEADER_HEIGHT;
        let win_size = size(px(WINDOW_WIDTH), px(height));
        let bounds = monitor::active(cx)
            .map(|m| m.centered_bounds(win_size))
            .unwrap_or_else(|| Bounds::centered(None, win_size, cx));

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| LauncherView::new(cx)).unwrap();
        cx.activate(true);
    });
}
