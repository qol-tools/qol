pub mod run;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, App, Context, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    SharedString, Window,
};

use crate::core::{self, Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

pub const WINDOW_TITLE: &str = "removeapp";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Picking,
    Confirming,
    Done,
}

pub struct RemoveAppView {
    apps: Vec<InstalledApp>,
    query: String,
    matches: Vec<InstalledApp>,
    selected: usize,
    mode: Mode,
    disposal: Disposal,
    plan: Option<RemovalPlan>,
    outcome: Option<RemovalOutcome>,
    focus_handle: FocusHandle,
}

impl RemoveAppView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let apps = core::installed_apps().unwrap_or_default();
        let matches = core::filter(&apps, "");
        Self {
            apps,
            query: String::new(),
            matches,
            selected: 0,
            mode: Mode::Picking,
            disposal: Disposal::Trash,
            plan: None,
            outcome: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn refilter(&mut self) {
        self.matches = core::filter(&self.apps, &self.query);
        self.selected = 0;
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, len as isize - 1);
        self.selected = next as usize;
    }

    fn enter_confirm(&mut self) {
        let Some(app) = self.matches.get(self.selected) else {
            return;
        };
        if core::is_protected(app) {
            return;
        }
        if let Ok(plan) = core::plan(app) {
            self.plan = Some(plan);
            self.mode = Mode::Confirming;
        }
    }

    fn toggle_disposal(&mut self) {
        self.disposal = match self.disposal {
            Disposal::Trash => Disposal::Delete,
            Disposal::Delete => Disposal::Trash,
        };
    }

    fn execute(&mut self) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        self.outcome = Some(core::remove(&plan, self.disposal).unwrap_or_default());
        self.mode = Mode::Done;
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        match self.mode {
            Mode::Picking => match key {
                "escape" => cx.quit(),
                "down" => {
                    self.move_selection(1);
                    cx.notify();
                }
                "up" => {
                    self.move_selection(-1);
                    cx.notify();
                }
                "enter" => {
                    self.enter_confirm();
                    cx.notify();
                }
                "backspace" => {
                    self.query.pop();
                    self.refilter();
                    cx.notify();
                }
                "space" => {
                    self.query.push(' ');
                    self.refilter();
                    cx.notify();
                }
                k if is_typed_char(k) => {
                    self.query.push_str(k);
                    self.refilter();
                    cx.notify();
                }
                _ => {}
            },
            Mode::Confirming => match key {
                "escape" => {
                    self.mode = Mode::Picking;
                    self.plan = None;
                    cx.notify();
                }
                "enter" => {
                    self.execute();
                    cx.notify();
                }
                "d" | "tab" => {
                    self.toggle_disposal();
                    cx.notify();
                }
                _ => {}
            },
            Mode::Done => cx.quit(),
        }
    }

    fn render_body(&self) -> AnyElement {
        match self.mode {
            Mode::Picking => self.render_picking(),
            Mode::Confirming => self.render_confirming(),
            Mode::Done => self.render_done(),
        }
    }

    fn render_picking(&self) -> AnyElement {
        let rows: Vec<_> = self
            .matches
            .iter()
            .enumerate()
            .map(|(i, app)| app_row(app, i == self.selected, core::is_protected(app)))
            .collect();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.search_box())
            .child(
                div()
                    .id("removeapp-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(footer(&[
                ("\u{2191}\u{2193}", "move"),
                ("\u{23CE}", "select"),
                ("esc", "quit"),
            ]))
            .into_any_element()
    }

    fn search_box(&self) -> impl IntoElement {
        let empty = self.query.is_empty();
        let shown = if empty {
            "Type to search apps".to_string()
        } else {
            self.query.clone()
        };
        div()
            .h(px(40.0))
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .bg(rgb(0x0d1117u32))
            .border_b_1()
            .border_color(rgb(0x21262du32))
            .child(div().text_color(rgb(0x6e7681u32)).child(">"))
            .child(
                div()
                    .flex_1()
                    .text_color(rgb(if empty { 0x6e7681u32 } else { 0xe6edf3u32 }))
                    .child(shown),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(0x6e7681u32))
                    .text_size(px(11.0))
                    .child(format!("{}", self.matches.len())),
            )
    }

    fn render_confirming(&self) -> AnyElement {
        let Some(plan) = &self.plan else {
            return div().into_any_element();
        };
        let items: Vec<_> = plan
            .items
            .iter()
            .map(|l| {
                div()
                    .flex()
                    .justify_between()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(4.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(rgb(0x8b949eu32))
                            .child(l.path.display().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.0))
                            .text_color(rgb(0x6e7681u32))
                            .child(format_size(l.size_bytes)),
                    )
            })
            .collect();
        let (disp_label, disp_color) = match self.disposal {
            Disposal::Trash => ("Move to Trash", 0x3fb950u32),
            Disposal::Delete => ("PERMANENTLY DELETE", 0xf85149u32),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(section_header(&plan.app.name))
            .child(
                div()
                    .id("removeapp-items")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .children(items),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(rgb(0x21262du32))
                    .child(
                        div()
                            .text_color(rgb(disp_color))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(disp_label),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xe6edf3u32))
                            .text_size(px(11.0))
                            .child(format!(
                                "{} items \u{00b7} {}",
                                plan.items.len(),
                                format_size(plan.total_bytes)
                            )),
                    ),
            )
            .child(footer(&[
                ("\u{23CE}", "confirm"),
                ("d", "trash/delete"),
                ("esc", "back"),
            ]))
            .into_any_element()
    }

    fn render_done(&self) -> AnyElement {
        let Some(outcome) = &self.outcome else {
            return div().into_any_element();
        };
        let removed = outcome.removed.len();
        let failed = outcome.failed.len();
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .px(px(24.0))
            .child(
                div()
                    .text_size(px(15.0))
                    .text_color(rgb(0x3fb950u32))
                    .child(format!("Removed {removed} item(s)")),
            )
            .when(failed > 0, |d| {
                d.child(
                    div()
                        .text_color(rgb(0xf85149u32))
                        .text_size(px(11.0))
                        .child(format!("{failed} failed")),
                )
            })
            .child(
                div()
                    .text_color(rgb(0x6e7681u32))
                    .text_size(px(11.0))
                    .child("Press any key to close"),
            )
            .into_any_element()
    }
}

impl Focusable for RemoveAppView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RemoveAppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.matches.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.matches.len() - 1);
        }
        div()
            .id("removeapp")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(0x161b22u32))
            .text_color(rgb(0xe6edf3u32))
            .font_family(SharedString::from("Menlo"))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| this.on_key(ev, cx)))
            .child(self.render_body())
    }
}

fn is_typed_char(key: &str) -> bool {
    key.chars().count() == 1
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || "-_.".contains(c))
}

fn app_row(app: &InstalledApp, selected: bool, protected: bool) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(8.0))
        .border_l_2()
        .border_color(if selected {
            rgb(0x58a6ffu32)
        } else {
            rgba(0x00000000u32)
        })
        .bg(if selected {
            rgba(0x58a6ff14u32)
        } else {
            rgba(0x00000000u32)
        })
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_color(if protected {
                    rgb(0x6e7681u32)
                } else {
                    rgb(0xe6edf3u32)
                })
                .child(app.name.clone()),
        )
        .when(protected, |d| {
            d.child(
                div()
                    .flex_none()
                    .text_size(px(10.0))
                    .text_color(rgb(0xf85149u32))
                    .child("protected"),
            )
        })
}

fn section_header(title: &str) -> impl IntoElement {
    div()
        .h(px(36.0))
        .w_full()
        .flex()
        .items_center()
        .px(px(12.0))
        .bg(rgb(0x0d1117u32))
        .border_b_1()
        .border_color(rgb(0x21262du32))
        .child(
            div()
                .text_color(rgb(0xe6edf3u32))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Remove {title}")),
        )
}

fn footer(hints: &[(&str, &str)]) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .w_full()
        .px(px(11.0))
        .py(px(7.0))
        .bg(rgb(0x0d1117u32))
        .border_t_1()
        .border_color(rgb(0x21262du32))
        .children(hints.iter().map(|(key, label)| {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_color(rgb(0x6e7681u32))
                .text_size(px(10.0))
                .child(
                    div()
                        .text_color(rgb(0xc9d1d9u32))
                        .text_size(px(9.0))
                        .bg(rgba(0xffffff0fu32))
                        .border_1()
                        .border_color(rgb(0x30363du32))
                        .rounded(px(4.0))
                        .px(px(5.0))
                        .py(px(1.0))
                        .child(key.to_string()),
                )
                .child(label.to_string())
        }))
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
