pub mod run;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, App, AsyncApp, Context, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, SharedString, WeakEntity, Window,
};

use crate::core::{
    self, Disposal, Guards, InstalledApp, PackageIndex, PackageStatus, RemovalOutcome, RemovalPlan,
};
use qol_gpui::scroll_list::ScrollList;
use qol_gpui::surface::PanelDragArea;
use qol_gpui::theme::{remove_app_runtime, RemoveAppPalette};

pub const WINDOW_TITLE: &str = "removeapp";
pub const WINDOW_WIDTH: f32 = 460.0;
pub const WINDOW_HEIGHT: f32 = 540.0;
const SEARCH_H: f32 = 40.0;
const FOOTER_H: f32 = 34.0;
const ROW_H: f32 = 38.0;
const MAX_VISIBLE: usize = ((WINDOW_HEIGHT - SEARCH_H - FOOTER_H) / ROW_H) as usize;
const CONTINUE_OR_QUIT_HINT: &str = "Enter to continue \u{00b7} Esc to quit";

fn current_palette() -> RemoveAppPalette {
    remove_app_runtime()
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Picking,
    Confirming,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimaryAction {
    Blocked,
    Package,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoneAction {
    Continue,
    Quit,
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
    error: Option<String>,
    guards: Option<Guards>,
    package_index: Option<PackageIndex>,
    quit_failed: bool,
    focus_handle: FocusHandle,
}

impl RemoveAppView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let apps = core::installed_apps().unwrap_or_default();
        let matches = core::filter(&apps, "");
        Self::spawn_package_prewarm(cx, apps.clone());
        Self {
            apps,
            query: String::new(),
            matches,
            list: ScrollList::new(MAX_VISIBLE),
            mode: Mode::Picking,
            disposal: Disposal::Trash,
            plan: None,
            outcome: None,
            error: None,
            guards: None,
            package_index: None,
            quit_failed: false,
            focus_handle: cx.focus_handle(),
        }
    }

    fn spawn_package_prewarm(cx: &mut Context<Self>, apps: Vec<InstalledApp>) {
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let index = async_cx
                    .background_spawn(async move { core::package_index(&apps) })
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    view.package_index = Some(index);
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
        self.disposal = Disposal::Trash;
        self.outcome = None;
        self.error = None;
        self.quit_failed = false;
        self.mode = Mode::Confirming;
        self.refresh_guards();
    }

    fn refresh_guards(&mut self) {
        let Some(plan) = &self.plan else {
            return;
        };
        self.guards = self
            .package_index
            .as_ref()
            .map(|index| core::guards_with(&plan.app, index));
    }

    fn toggle_disposal(&mut self) {
        self.disposal = match self.disposal {
            Disposal::Trash => Disposal::Delete,
            Disposal::Delete => Disposal::Trash,
        };
    }

    fn try_quit(&mut self) {
        let Some(plan) = &self.plan else {
            return;
        };
        match core::quit_and_wait(&plan.app) {
            Ok(()) => {
                if let Some(g) = self.guards.as_mut() {
                    g.running = core::is_running(&plan.app);
                }
                self.quit_failed = false;
            }
            Err(_) => self.quit_failed = true,
        }
    }

    fn try_package(&mut self) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        let package = match &self.guards {
            Some(g) if !g.running => match &g.package {
                PackageStatus::Managed(package) => package.clone(),
                _ => return,
            },
            _ => return,
        };
        if core::is_running(&plan.app) {
            if let Some(g) = self.guards.as_mut() {
                g.running = true;
            }
            return;
        }
        match core::uninstall_package(&plan, &package) {
            Ok(()) => {
                let status = PackageStatus::Managed(package);
                self.finish_result(core::remove_after_package(
                    &plan,
                    Disposal::Trash,
                    &status,
                    true,
                ));
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.outcome = None;
                self.mode = Mode::Done;
            }
        }
    }

    fn execute(&mut self, disposal: Disposal, waive_running: bool) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        if !waive_running && core::is_running(&plan.app) {
            if let Some(g) = self.guards.as_mut() {
                g.running = true;
            }
            return;
        }
        let package = self
            .guards
            .as_ref()
            .map(|g| g.package.clone())
            .unwrap_or(PackageStatus::NotManaged);
        self.finish_result(core::remove(&plan, disposal, &package));
    }

    fn finish_result(&mut self, result: anyhow::Result<RemovalOutcome>) {
        match result {
            Ok(outcome) => {
                self.outcome = Some(outcome);
                self.error = None;
            }
            Err(error) => {
                self.outcome = None;
                self.error = Some(error.to_string());
            }
        }
        self.mode = Mode::Done;
    }

    fn continue_picking(&mut self, cx: &mut Context<Self>) {
        if let Ok(apps) = core::installed_apps() {
            self.apps = apps;
        }
        self.query.clear();
        self.refilter();
        self.mode = Mode::Picking;
        self.disposal = Disposal::Trash;
        self.plan = None;
        self.outcome = None;
        self.error = None;
        self.guards = None;
        self.package_index = None;
        self.quit_failed = false;
        Self::spawn_package_prewarm(cx, self.apps.clone());
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
                    self.error = None;
                    cx.notify();
                }
                "q" => {
                    self.try_quit();
                    cx.notify();
                }
                "t" => {
                    self.execute(Disposal::Trash, true);
                    cx.notify();
                }
                "enter" => {
                    match primary_action(self.guards.as_ref()) {
                        PrimaryAction::Package => self.try_package(),
                        PrimaryAction::Remove => self.execute(self.disposal, false),
                        PrimaryAction::Blocked => {}
                    }
                    cx.notify();
                }
                "d" | "tab" => {
                    if primary_action(self.guards.as_ref()) == PrimaryAction::Remove {
                        self.toggle_disposal();
                    }
                    cx.notify();
                }
                _ => {}
            },
            Mode::Done => match done_action(key) {
                Some(DoneAction::Continue) => {
                    self.continue_picking(cx);
                    cx.notify();
                }
                Some(DoneAction::Quit) => cx.quit(),
                None => {}
            },
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
        let palette = current_palette();
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
            .bg(rgb(palette.chrome_bg))
            .border_b_1()
            .border_color(rgb(palette.border))
            .panel_drag_area()
            .child(div().text_color(rgb(palette.text_muted)).child(">"))
            .child(
                div()
                    .flex_1()
                    .text_color(rgb(if empty {
                        palette.text_muted
                    } else {
                        palette.text_primary
                    }))
                    .child(shown),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(palette.text_muted))
                    .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                    .child(format!("{}", self.matches.len())),
            )
    }

    fn render_confirming(&self) -> AnyElement {
        let palette = current_palette();
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
                            .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                            .text_color(rgb(palette.text_secondary))
                            .child(l.path.display().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                            .text_color(rgb(palette.text_muted))
                            .child(format_size(l.size_bytes)),
                    )
            })
            .collect();
        let managed = self
            .guards
            .as_ref()
            .and_then(|guards| match &guards.package {
                PackageStatus::Managed(package) => Some(package),
                _ => None,
            });
        let (disp_label, disp_color) = match (managed, self.disposal) {
            (Some(package), _) => (
                format!("Uninstall with {}", package.manager().label()),
                palette.accent,
            ),
            (None, Disposal::Trash) => ("Move to Trash".to_string(), palette.success),
            (None, Disposal::Delete) => ("PERMANENTLY DELETE".to_string(), palette.danger),
        };
        let mut hints: Vec<(&str, &str)> = Vec::new();
        if let Some(g) = &self.guards {
            if g.running {
                hints.push(("Q", "quit app"));
            }
            if !g.running {
                if let PackageStatus::Managed(package) = &g.package {
                    let label = match package.manager() {
                        crate::core::PackageManager::Homebrew => "uninstall with Homebrew",
                        crate::core::PackageManager::Apt => "uninstall with APT",
                        crate::core::PackageManager::Flatpak => "uninstall with Flatpak",
                    };
                    hints.push(("\u{23CE}", label));
                }
            }
        }
        if primary_action(self.guards.as_ref()) == PrimaryAction::Remove {
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
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .text_color(rgb(disp_color))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(disp_label),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.text_primary))
                            .text_size(px(qol_gpui::theme::TEXT_CAPTION))
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
        let palette = current_palette();
        let Some(g) = self.guards.as_ref() else {
            return Some(banner_container(vec![banner_line(
                palette.text_muted,
                "Checking package manager\u{2026}",
            )
            .into_any_element()]));
        };
        let mut lines: Vec<AnyElement> = Vec::new();
        if g.running {
            let text = if self.quit_failed {
                "still running - couldn't quit; press Q to retry"
            } else {
                "is running - press Q to quit first"
            };
            lines.push(banner_line(palette.warning, text).into_any_element());
        }
        match &g.package {
            PackageStatus::Managed(package) => lines.push(
                banner_line(
                    palette.accent,
                    &if g.running {
                        format!(
                            "{}-managed - quit it before uninstalling",
                            package.manager().label()
                        )
                    } else {
                        format!(
                            "{}-managed - Enter uninstalls through {}",
                            package.manager().label(),
                            package.manager().label()
                        )
                    },
                )
                .into_any_element(),
            ),
            PackageStatus::Unavailable(reason) => lines.push(
                banner_line(
                    palette.text_muted,
                    &format!("couldn't confirm package ownership - {reason}"),
                )
                .into_any_element(),
            ),
            PackageStatus::NotManaged => {}
        }
        if lines.is_empty() {
            return None;
        }
        Some(banner_container(lines))
    }

    fn render_done(&self) -> AnyElement {
        let palette = current_palette();
        if let Some(error) = &self.error {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .px(px(24.0))
                .panel_drag_area()
                .child(
                    div()
                        .text_size(px(qol_gpui::theme::TEXT_TITLE))
                        .text_color(rgb(palette.danger))
                        .child("Removal failed"),
                )
                .child(
                    div()
                        .text_color(rgb(palette.text_secondary))
                        .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                        .child(error.clone()),
                )
                .child(
                    div()
                        .text_color(rgb(palette.text_muted))
                        .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                        .child(CONTINUE_OR_QUIT_HINT),
                )
                .into_any_element();
        }
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
            .panel_drag_area()
            .child(
                div()
                    .text_size(px(qol_gpui::theme::TEXT_TITLE))
                    .text_color(rgb(palette.success))
                    .child(format!("Removed {removed} item(s)")),
            )
            .child(
                div()
                    .text_color(rgb(palette.text_secondary))
                    .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                    .child(format!("Freed {}", format_size(outcome.freed_bytes))),
            )
            .when(failed > 0, |d| {
                d.child(
                    div()
                        .text_color(rgb(palette.danger))
                        .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                        .child(format!("{failed} failed")),
                )
            })
            .child(
                div()
                    .text_color(rgb(palette.text_muted))
                    .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                    .child(CONTINUE_OR_QUIT_HINT),
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
        let palette = current_palette();
        self.list.sync(self.matches.len());
        div()
            .id("removeapp")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(palette.panel_bg))
            .text_color(rgb(palette.text_primary))
            .font_family(SharedString::from(qol_gpui::theme::font_mono()))
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

fn primary_action(guards: Option<&Guards>) -> PrimaryAction {
    let Some(guards) = guards else {
        return PrimaryAction::Blocked;
    };
    if guards.running {
        return PrimaryAction::Blocked;
    }
    if matches!(guards.package, PackageStatus::Managed(_)) {
        return PrimaryAction::Package;
    }
    PrimaryAction::Remove
}

fn done_action(key: &str) -> Option<DoneAction> {
    match key {
        "enter" => Some(DoneAction::Continue),
        "escape" => Some(DoneAction::Quit),
        _ => None,
    }
}

fn app_row(app: &InstalledApp, selected: bool, protected: bool) -> impl IntoElement {
    let palette = current_palette();
    div()
        .w_full()
        .h(px(ROW_H))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .border_l_2()
        .border_color(if selected {
            rgb(palette.accent)
        } else {
            rgba(palette.transparent_rgba)
        })
        .bg(if selected {
            rgba(palette.selection_bg_rgba)
        } else {
            rgba(palette.transparent_rgba)
        })
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_color(if protected {
                    rgb(palette.text_muted)
                } else {
                    rgb(palette.text_primary)
                })
                .child(app.name.clone()),
        )
        .when(protected, |d| {
            d.child(
                div()
                    .flex_none()
                    .text_size(px(qol_gpui::theme::TEXT_NANO))
                    .text_color(rgb(palette.danger))
                    .child("protected"),
            )
        })
}

fn section_header(title: &str) -> impl IntoElement {
    let palette = current_palette();
    div()
        .h(px(36.0))
        .w_full()
        .flex()
        .items_center()
        .px(px(12.0))
        .bg(rgb(palette.chrome_bg))
        .border_b_1()
        .border_color(rgb(palette.border))
        .panel_drag_area()
        .child(
            div()
                .text_color(rgb(palette.text_primary))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Remove {title}")),
        )
}

fn footer(hints: &[(&str, &str)]) -> impl IntoElement {
    let palette = current_palette();
    div()
        .flex()
        .flex_none()
        .h(px(FOOTER_H))
        .items_center()
        .gap(px(10.0))
        .w_full()
        .px(px(11.0))
        .bg(rgb(palette.chrome_bg))
        .border_t_1()
        .border_color(rgb(palette.border))
        .children(hints.iter().map(|(key, label)| {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_color(rgb(palette.text_muted))
                .text_size(px(qol_gpui::theme::TEXT_NANO))
                .child(
                    div()
                        .text_color(rgb(palette.text_heading))
                        .text_size(px(qol_gpui::theme::TEXT_NANO))
                        .bg(rgba(palette.keycap_bg_rgba))
                        .border_1()
                        .border_color(rgb(palette.border_strong))
                        .rounded_none()
                        .px(px(5.0))
                        .py(px(1.0))
                        .child(key.to_string()),
                )
                .child(label.to_string())
        }))
}

fn banner_container(lines: Vec<AnyElement>) -> AnyElement {
    let palette = current_palette();
    div()
        .flex()
        .flex_col()
        .w_full()
        .px(px(12.0))
        .py(px(6.0))
        .gap(px(3.0))
        .bg(rgba(palette.warning_banner_rgba))
        .border_b_1()
        .border_color(rgb(palette.border))
        .children(lines)
        .into_any_element()
}

fn banner_line(color: u32, text: &str) -> impl IntoElement {
    div()
        .text_size(px(qol_gpui::theme::TEXT_CAPTION))
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

#[cfg(test)]
mod tests {
    use super::{
        done_action, primary_action, DoneAction, PrimaryAction, FOOTER_H, MAX_VISIBLE, ROW_H,
        SEARCH_H, WINDOW_HEIGHT,
    };
    use crate::core::{Guards, ManagedPackage, PackageManager, PackageScope, PackageStatus};

    #[test]
    fn visible_rows_fit_window_and_are_maximal() {
        let chrome = SEARCH_H + FOOTER_H;
        let fits = chrome + MAX_VISIBLE as f32 * ROW_H;
        let one_more = chrome + (MAX_VISIBLE + 1) as f32 * ROW_H;
        assert!(
            fits <= WINDOW_HEIGHT,
            "all {MAX_VISIBLE} rows fit: {fits} <= {WINDOW_HEIGHT}"
        );
        assert!(
            one_more > WINDOW_HEIGHT,
            "MAX_VISIBLE is the most that fit: {one_more} > {WINDOW_HEIGHT}"
        );
    }

    #[test]
    fn enter_uses_the_contextual_primary_action() {
        let managed =
            ManagedPackage::parse(PackageManager::Apt, "antigravity", PackageScope::System)
                .expect("valid package");
        let cases = [
            (None, PrimaryAction::Blocked),
            (
                Some(Guards {
                    running: true,
                    package: PackageStatus::Managed(managed.clone()),
                }),
                PrimaryAction::Blocked,
            ),
            (
                Some(Guards {
                    running: false,
                    package: PackageStatus::Managed(managed),
                }),
                PrimaryAction::Package,
            ),
            (
                Some(Guards {
                    running: false,
                    package: PackageStatus::NotManaged,
                }),
                PrimaryAction::Remove,
            ),
            (
                Some(Guards {
                    running: false,
                    package: PackageStatus::Unavailable("offline".to_string()),
                }),
                PrimaryAction::Remove,
            ),
        ];

        for (guards, expected) in &cases {
            assert_eq!(primary_action(guards.as_ref()), *expected);
        }
    }

    #[test]
    fn completed_screen_only_handles_enter_and_escape() {
        let cases = [
            ("enter", Some(DoneAction::Continue)),
            ("escape", Some(DoneAction::Quit)),
            ("u", None),
            ("space", None),
            ("q", None),
        ];

        for (key, expected) in cases {
            assert_eq!(done_action(key), expected);
        }
    }
}
