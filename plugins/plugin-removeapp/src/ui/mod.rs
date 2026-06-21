pub mod run;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, App, AsyncApp, Context, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, SharedString, WeakEntity, Window,
};

use crate::core::{
    self, CaskIndex, CaskStatus, Disposal, Guards, InstalledApp, RemovalOutcome, RemovalPlan,
};
use qol_gpui::scroll_list::ScrollList;

pub const WINDOW_TITLE: &str = "removeapp";
const MAX_VISIBLE: usize = 12;

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
    list: ScrollList,
    mode: Mode,
    disposal: Disposal,
    plan: Option<RemovalPlan>,
    outcome: Option<RemovalOutcome>,
    guards: Option<Guards>,
    cask_index: Option<CaskIndex>,
    quit_failed: bool,
    focus_handle: FocusHandle,
}

impl RemoveAppView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let apps = core::installed_apps().unwrap_or_default();
        let matches = core::filter(&apps, "");
        Self::spawn_cask_prewarm(cx);
        Self {
            apps,
            query: String::new(),
            matches,
            list: ScrollList::new(MAX_VISIBLE),
            mode: Mode::Picking,
            disposal: Disposal::Trash,
            plan: None,
            outcome: None,
            guards: None,
            cask_index: None,
            quit_failed: false,
            focus_handle: cx.focus_handle(),
        }
    }

    fn spawn_cask_prewarm(cx: &mut Context<Self>) {
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let index = async_cx
                    .background_spawn(async { core::cask_index() })
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    view.cask_index = Some(index);
                    if view.mode == Mode::Confirming && view.guards.is_none() {
                        view.refresh_guards();
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn refilter(&mut self) {
        self.matches = core::filter(&self.apps, &self.query);
        self.list.reset();
    }

    fn enter_confirm(&mut self) {
        let Some(app) = self.matches.get(self.list.selected).cloned() else {
            return;
        };
        if core::is_protected(&app) {
            return;
        }
        let Ok(plan) = core::plan(&app, &self.apps) else {
            return;
        };
        self.plan = Some(plan);
        self.quit_failed = false;
        self.mode = Mode::Confirming;
        self.refresh_guards();
    }

    fn refresh_guards(&mut self) {
        let Some(plan) = &self.plan else {
            return;
        };
        self.guards = self
            .cask_index
            .as_ref()
            .map(|index| core::guards_with(&plan.app, &self.apps, index));
    }

    fn toggle_disposal(&mut self) {
        self.disposal = match self.disposal {
            Disposal::Trash => Disposal::Delete,
            Disposal::Delete => Disposal::Trash,
        };
    }

    fn unresolved(&self) -> bool {
        match &self.guards {
            None => true,
            Some(g) => g.running || matches!(g.cask, CaskStatus::Managed(_)),
        }
    }

    fn try_quit(&mut self) {
        let Some(plan) = &self.plan else {
            return;
        };
        match core::quit_app(&plan.app) {
            Ok(()) => {
                if let Some(g) = self.guards.as_mut() {
                    g.running = false;
                }
                self.quit_failed = false;
            }
            Err(_) => self.quit_failed = true,
        }
    }

    fn try_brew(&mut self) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        let token = match &self.guards {
            Some(g) if !g.running => match &g.cask {
                CaskStatus::Managed(t) => t.clone(),
                _ => return,
            },
            _ => return,
        };
        if core::brew_uninstall(&token).is_ok() {
            let cask = CaskStatus::Managed(token);
            self.outcome = Some(
                core::remove_after_brew(&plan, self.disposal, &cask, true).unwrap_or_default(),
            );
            self.mode = Mode::Done;
        }
    }

    fn execute(&mut self, disposal: Disposal) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        let cask = self
            .guards
            .as_ref()
            .map(|g| g.cask.clone())
            .unwrap_or(CaskStatus::NotManaged);
        self.outcome = Some(core::remove(&plan, disposal, &cask).unwrap_or_default());
        self.mode = Mode::Done;
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        match self.mode {
            Mode::Picking => match key {
                "escape" => cx.quit(),
                "down" => {
                    self.list.move_down(self.matches.len());
                    cx.notify();
                }
                "up" => {
                    self.list.move_up();
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
                    self.guards = None;
                    cx.notify();
                }
                "q" => {
                    self.try_quit();
                    cx.notify();
                }
                "b" => {
                    self.try_brew();
                    cx.notify();
                }
                "t" => {
                    self.execute(Disposal::Trash);
                    cx.notify();
                }
                "enter" => {
                    if !self.unresolved() {
                        self.execute(self.disposal);
                    }
                    cx.notify();
                }
                "d" | "tab" => {
                    if !self.unresolved() {
                        self.toggle_disposal();
                    }
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
        let range = self.list.visible_range(self.matches.len());
        let selected = self.list.selected;
        let rows: Vec<_> = self.matches[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, app)| {
                let index = range.start + offset;
                app_row(app, index == selected, core::is_protected(app))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.search_box())
            .child(div().flex_1().min_h_0().w_full().children(rows))
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
        let mut hints: Vec<(&str, &str)> = Vec::new();
        if let Some(g) = &self.guards {
            if g.running {
                hints.push(("Q", "quit & continue"));
            }
            if matches!(g.cask, CaskStatus::Managed(_)) {
                hints.push(("B", "brew uninstall"));
            }
        }
        if !self.unresolved() {
            hints.push(("\u{23CE}", "confirm"));
            hints.push(("d", "trash/delete"));
        }
        hints.push(("T", "trash anyway"));
        hints.push(("esc", "back"));
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(section_header(&plan.app.name))
            .children(self.guard_banner())
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
            .child(footer(&hints))
            .into_any_element()
    }

    fn guard_banner(&self) -> Option<AnyElement> {
        let Some(g) = self.guards.as_ref() else {
            return Some(banner_container(vec![banner_line(
                0x6e7681u32,
                "Checking Homebrew\u{2026}",
            )
            .into_any_element()]));
        };
        let mut lines: Vec<AnyElement> = Vec::new();
        if g.running {
            let text = if self.quit_failed {
                "still running - couldn't quit; press Q to retry"
            } else {
                "is running - press Q to quit & continue"
            };
            lines.push(banner_line(0xf0883eu32, text).into_any_element());
        }
        match &g.cask {
            CaskStatus::Managed(_) => lines.push(
                banner_line(0x58a6ffu32, "Homebrew-managed - press B to brew uninstall")
                    .into_any_element(),
            ),
            CaskStatus::Unavailable(_) => lines.push(
                banner_line(0x6e7681u32, "couldn't confirm Homebrew - check manually")
                    .into_any_element(),
            ),
            CaskStatus::NotManaged => {}
        }
        if lines.is_empty() {
            return None;
        }
        Some(banner_container(lines))
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
            .child(
                div()
                    .text_color(rgb(0x8b949eu32))
                    .text_size(px(12.0))
                    .child(format!("Freed {}", format_size(outcome.freed_bytes))),
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
        self.list.sync(self.matches.len());
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

fn banner_container(lines: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .w_full()
        .px(px(12.0))
        .py(px(6.0))
        .gap(px(3.0))
        .bg(rgba(0xf0883e1au32))
        .border_b_1()
        .border_color(rgb(0x21262du32))
        .children(lines)
        .into_any_element()
}

fn banner_line(color: u32, text: &str) -> impl IntoElement {
    div()
        .text_size(px(11.0))
        .text_color(rgb(color))
        .child(text.to_string())
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
