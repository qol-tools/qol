pub mod run;

use gpui::prelude::*;
use gpui::{
    div, font, px, rgb, rgba, AnyElement, App, AsyncApp, Context, ElementId, FocusHandle,
    Focusable, KeyDownEvent, ScrollWheelEvent, WeakEntity, Window,
};
use qol_gpui::kit::{Chip, NoticeTone};
use qol_gpui::text::{cased, TextStyled};
use qol_gpui::theme::TextStyle;
use qol_gpui::Key;

use crate::core::{
    self, Disposal, Guards, InstalledApp, ManagedPackage, PackageIndex, PackageStatus,
    RemovalOutcome, RemovalPlan,
};
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::surface::PanelDragArea;
use qol_gpui::text_edit::{self, CaretStyle, TextField, TextFieldElement};

pub const WINDOW_TITLE: &str = env!("QOL_PLUGIN_ID");
pub const WINDOW_WIDTH: f32 = 460.0;
pub const WINDOW_HEIGHT: f32 = 540.0;
const SEARCH_H: f32 = qol_gpui::theme::HEIGHT_HINT_BAR;
const SEARCH_PAD: f32 = qol_gpui::theme::SPACE_CELL;
const FAILBAR_H: f32 = qol_gpui::theme::HEIGHT_BAND;
const ROW_H: f32 = 40.0;
const PICKING_CHROME: f32 = 2.0 * qol_gpui::theme::LINE
    + SEARCH_PAD
    + SEARCH_H
    + SEARCH_PAD
    + FAILBAR_H
    + qol_gpui::theme::HEIGHT_HINT_BAR;
const MIN_ROWS: usize = 3;
pub const MIN_WINDOW_HEIGHT: f32 = PICKING_CHROME + MIN_ROWS as f32 * ROW_H;
const CONTENT_MAX_W: f32 = 720.0;
const REMOVE_KEY: Key = Key::BACKSPACE.platform();

fn continue_or_quit_hint(cx: &mut Context<RemoveAppView>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(qol_gpui::theme::SPACE_GUTTER))
        .text_color(rgb(qol_gpui::kit::kit().grounds.pane.faint))
        .text(TextStyle::Hint)
        .children(hint_buttons(
            &[
                (Key::ENTER, "continue", Command::Continue),
                (Key::ESC, "quit", Command::Quit),
            ],
            cx,
        ))
}

fn hint_buttons(
    hints: &[(Key, &str, Command)],
    cx: &mut Context<RemoveAppView>,
) -> Vec<gpui::Stateful<gpui::Div>> {
    let kit = qol_gpui::kit::kit();
    hints
        .iter()
        .enumerate()
        .map(|(index, (key, label, command))| {
            let command = *command;
            kit.hint_button(
                ("qol-removeapp-hint", index),
                *key,
                label.to_string(),
                cx.listener(move |this, _, _, cx| this.run(command, cx)),
            )
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Picking,
    Confirming,
    Working { label: &'static str, closable: bool },
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimaryAction {
    Blocked,
    Package,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Select,
    Back,
    QuitApp,
    TrashAnyway,
    Primary,
    ToggleDisposal,
    Continue,
    Quit,
}

pub struct RemoveAppView {
    apps: Vec<InstalledApp>,
    query: TextField,
    matches: Vec<InstalledApp>,
    list: ScrollList,
    mode: Mode,
    disposal: Disposal,
    plan: Option<RemovalPlan>,
    outcome: Option<RemovalOutcome>,
    error: Option<String>,
    guards: Option<Guards>,
    package_index: Option<PackageIndex>,
    sizes: std::collections::HashMap<std::path::PathBuf, u64>,
    quit_failed: bool,
    items_scroll: gpui::ScrollHandle,
    focus_handle: FocusHandle,
}

impl RemoveAppView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let apps = core::installed_apps().unwrap_or_default();
        let matches = core::filter(&apps, "");
        Self::spawn_package_prewarm(cx, apps.clone());
        Self::spawn_size_prewarm(cx, apps.clone());
        Self {
            apps,
            query: TextField::new(),
            matches,
            list: ScrollList::new(rows_that_fit(WINDOW_HEIGHT)),
            mode: Mode::Picking,
            disposal: Disposal::Trash,
            plan: None,
            outcome: None,
            error: None,
            guards: None,
            package_index: None,
            sizes: std::collections::HashMap::new(),
            quit_failed: false,
            items_scroll: gpui::ScrollHandle::new(),
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

    fn spawn_size_prewarm(cx: &mut Context<Self>, apps: Vec<InstalledApp>) {
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let sizes = async_cx
                    .background_spawn(async move { core::app_sizes(&apps) })
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    view.sizes = sizes;
                    let untouched = view.query.is_empty() && view.list.selected == 0;
                    if view.mode == Mode::Picking && untouched {
                        view.refilter();
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn app_size(&self, app: &InstalledApp) -> Option<u64> {
        self.sizes.get(&app.path).copied()
    }

    fn refilter(&mut self) {
        let mut matches = core::filter(&self.apps, self.query.text());
        matches.sort_by(|a, b| {
            let left = self.sizes.get(&a.path).copied().unwrap_or(0);
            let right = self.sizes.get(&b.path).copied().unwrap_or(0);
            right
                .cmp(&left)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.matches = matches;
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

    fn try_package(&mut self, cx: &mut Context<Self>) {
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
        self.finish_in_background(
            Mode::Working {
                label: "Waiting for the uninstaller",
                closable: true,
            },
            move || uninstall_then_remove(&plan, package),
            cx,
        );
    }

    fn finish_in_background(
        &mut self,
        mode: Mode,
        work: impl FnOnce() -> anyhow::Result<RemovalOutcome> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.mode = mode;
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx.background_spawn(async move { work() }).await;
                this.update(&mut async_cx, |view, cx| {
                    view.finish_result(result);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn execute(&mut self, disposal: Disposal, waive_running: bool, cx: &mut Context<Self>) {
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
        self.finish_in_background(
            Mode::Working {
                label: "Removing",
                closable: false,
            },
            move || core::remove(&plan, disposal, &package),
            cx,
        );
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
        let span = text_edit::span(&ev.keystroke.modifiers);
        let shift = ev.keystroke.modifiers.shift;
        let secondary = ev.keystroke.modifiers.secondary();
        match self.mode {
            Mode::Picking => match key {
                "escape" => self.run(Command::Quit, cx),
                "down" => {
                    self.list.move_down(self.matches.len());
                    cx.notify();
                }
                "up" => {
                    self.list.move_up();
                    cx.notify();
                }
                "enter" => self.run(Command::Select, cx),
                "backspace" if ev.keystroke.modifiers.platform => self.run(Command::Select, cx),
                "backspace" => {
                    self.query.backspace(span);
                    self.refilter();
                    cx.notify();
                }
                "delete" => {
                    if self.query.delete_forward(span) {
                        self.refilter();
                    }
                    cx.notify();
                }
                "left" => {
                    self.query.move_left(shift, span);
                    cx.notify();
                }
                "right" => {
                    self.query.move_right(shift, span);
                    cx.notify();
                }
                "home" => {
                    self.query.move_home(shift);
                    cx.notify();
                }
                "end" => {
                    self.query.move_end(shift);
                    cx.notify();
                }
                "a" if secondary => {
                    self.query.select_all();
                    cx.notify();
                }
                "space" => {
                    self.query.insert_char(' ');
                    self.refilter();
                    cx.notify();
                }
                k if is_typed_char(k) => {
                    if let Some(ch) = k.chars().next() {
                        self.query.insert_char(ch);
                        self.refilter();
                    }
                    cx.notify();
                }
                _ => {}
            },
            Mode::Confirming => {
                if let Some(command) = confirm_command(key) {
                    self.run(command, cx);
                }
            }
            Mode::Working { closable, .. } => {
                if let Some(command) = working_command(key, closable) {
                    self.run(command, cx);
                }
            }
            Mode::Done => {
                if let Some(command) = done_action(key) {
                    self.run(command, cx);
                }
            }
        }
    }

    fn run(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Select => self.enter_confirm(),
            Command::Back => {
                self.mode = Mode::Picking;
                self.plan = None;
                self.guards = None;
                self.error = None;
            }
            Command::QuitApp => self.try_quit(),
            Command::TrashAnyway => self.execute(Disposal::Trash, true, cx),
            Command::Primary => match primary_action(self.guards.as_ref()) {
                PrimaryAction::Package => self.try_package(cx),
                PrimaryAction::Remove => self.execute(self.disposal, false, cx),
                PrimaryAction::Blocked => {}
            },
            Command::ToggleDisposal => {
                if primary_action(self.guards.as_ref()) == PrimaryAction::Remove {
                    self.toggle_disposal();
                }
            }
            Command::Continue => self.continue_picking(cx),
            Command::Quit => cx.quit(),
        }
        cx.notify();
    }

    fn render_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.mode {
            Mode::Picking => self.render_picking(window, cx),
            Mode::Confirming => self.render_confirming(cx),
            Mode::Working { label, closable } => self.render_working(label, closable, cx),
            Mode::Done => self.render_done(cx),
        }
    }

    fn render_picking(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let range = self.list.visible_range(self.matches.len());
        let selected = self.list.selected;
        let rows: Vec<_> = self.matches[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, app)| {
                let index = range.start + offset;
                div()
                    .id(("qol-removeapp-row", index))
                    .cursor(gpui::CursorStyle::PointingHand)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.list.selected = index;
                        this.run(Command::Select, cx);
                    }))
                    .child(app_row(
                        app,
                        self.app_size(app),
                        index == selected,
                        core::is_protected(app),
                    ))
            })
            .collect();
        let counter = format!("{} / {}", self.matches.len(), self.apps.len());
        let empty = self.matches.is_empty().then(|| {
            let title = if self.query.is_empty() {
                "No installed apps found".to_string()
            } else {
                format!("No apps match \"{}\"", self.query.text())
            };
            kit.empty(title, None)
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(centered_column(
                div()
                    .flex()
                    .flex_col()
                    .child(self.search_box(window))
                    .child(
                        div()
                            .id("qol-removeapp-list")
                            .on_scroll_wheel(cx.listener(
                                |this, event: &ScrollWheelEvent, _, cx| {
                                    let rows = wheel_rows(&event.delta, ROW_H);
                                    this.list.wheel_by(rows, this.matches.len());
                                    cx.notify();
                                },
                            ))
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .flex()
                            .flex_col()
                            .pt(px(SEARCH_PAD))
                            .children(rows)
                            .children(empty)
                            .child(clear_of_resize_edge(kit.scroll_cue(
                                ScrollSource::Window {
                                    first: range.start,
                                    shown: range.len(),
                                    total: self.matches.len(),
                                },
                                kit.grounds.pane,
                            ))),
                    )
                    .children(self.remove_bar(cx)),
            ))
            .child(footer(
                &[
                    (Key::ENTER, "select", Command::Select),
                    (REMOVE_KEY, "remove", Command::Select),
                ],
                Some(counter),
                cx,
            ))
            .into_any_element()
    }

    fn remove_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let kit = qol_gpui::kit::kit();
        let app = self.matches.get(self.list.selected)?;
        if core::is_protected(app) {
            return None;
        }
        let subtitle: AnyElement = match self.app_size(app) {
            Some(size) => {
                format!("App \u{00B7} {}", qol_gpui::format_bytes(size)).into_any_element()
            }
            None => qol_gpui::Busy::new(
                "qol-removeapp-size",
                "Measuring size",
                rgb(kit.grounds.pane.faint),
            )
            .into_any_element(),
        };
        let bar = kit
            .notice(
                NoticeTone::Invalid,
                format!("Remove {}", app.name),
                Some(subtitle),
            )
            .h(px(FAILBAR_H))
            .w_full()
            .child(kit.chip(Chip::Key(REMOVE_KEY), kit.grounds.invalid))
            .id("qol-removeapp-remove")
            .cursor(gpui::CursorStyle::PointingHand)
            .on_click(cx.listener(|this, _, _, cx| this.run(Command::Select, cx)));
        Some(
            kit.pointable(bar, rgb(kit.grounds.invalid.lift))
                .into_any_element(),
        )
    }

    fn search_box(&self, window: &mut Window) -> impl IntoElement {
        let kit = qol_gpui::kit::kit();
        let empty = self.query.is_empty();
        let mono = font(qol_gpui::theme::font_mono());
        let advance =
            qol_gpui::text::shaped_width(window, "0", mono.clone(), qol_gpui::theme::TEXT_CAPTION);
        let keycap = qol_gpui::text::shaped_width(window, "/", mono, qol_gpui::theme::TEXT_KEYCAP)
            + 2.0 * qol_gpui::theme::SPACE_SNUG
            + 2.0;
        let available = content_width(f32::from(window.viewport_size().width))
            - 2.0 * SEARCH_PAD
            - 2.0 * qol_gpui::theme::SPACE_PAD
            - 10.0
            - keycap
            - 2.0;
        let visible = text_edit::visible_char_count(available, advance);
        div()
            .flex_none()
            .w_full()
            .pt(px(SEARCH_PAD))
            .px(px(SEARCH_PAD))
            .panel_drag_area()
            .child(
                div()
                    .h(px(SEARCH_H))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(qol_gpui::theme::SPACE_INSET))
                    .px(px(qol_gpui::theme::SPACE_PAD))
                    .rounded(px(qol_gpui::theme::RADIUS_WELL))
                    .border(px(qol_gpui::theme::LINE))
                    .border_color(rgba(kit.washes.hairline.packed()))
                    .bg(rgba(kit.washes.fill_hover.packed()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text(TextStyle::Code)
                            .text_color(rgb(if empty {
                                kit.grounds.pane.faint
                            } else {
                                kit.grounds.pane.ink
                            }))
                            .child(
                                TextFieldElement::new(&self.query, visible, advance)
                                    .selection(
                                        rgba(qol_gpui::theme::translucent(
                                            kit.palette.accent,
                                            qol_gpui::theme::Alpha::Wash,
                                        ))
                                        .into(),
                                        Some(rgb(kit.grounds.pane.ink).into()),
                                    )
                                    .caret(CaretStyle {
                                        color: rgb(kit.palette.accent).into(),
                                        width: 2.0,
                                        height: 16.0,
                                        top: 1.0,
                                        radius: 1.0,
                                    })
                                    .placeholder(
                                        "Search installed apps",
                                        rgb(kit.grounds.pane.faint).into(),
                                    )
                                    .render(),
                            ),
                    )
                    .child(kit.chip(Chip::Key(Key::symbol('/')), kit.grounds.pane)),
            )
    }

    fn render_confirming(&self, cx: &mut Context<Self>) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let Some(plan) = &self.plan else {
            return div().into_any_element();
        };
        let item_count = plan.items.len();
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
                            .text(TextStyle::Code)
                            .text_color(rgb(kit.grounds.pane.faint))
                            .child(l.path.display().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text(TextStyle::Code)
                            .text_color(rgb(kit.grounds.pane.faint))
                            .child(qol_gpui::format_bytes(l.size_bytes)),
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
            (Some(_), _) => ("Uninstall".to_string(), kit.palette.accent),
            (None, Disposal::Trash) => ("Move to Trash".to_string(), kit.palette.success),
            (None, Disposal::Delete) => ("Delete permanently".to_string(), kit.palette.danger),
        };
        let mut hints: Vec<(Key, &str, Command)> = Vec::new();
        if let Some(g) = &self.guards {
            if g.running {
                hints.push((Key::letter('q'), "quit app", Command::QuitApp));
            }
            if !g.running {
                if let PackageStatus::Managed(package) = &g.package {
                    let label = match package.manager() {
                        crate::core::PackageManager::Homebrew => "uninstall with Homebrew",
                        crate::core::PackageManager::Apt => "uninstall",
                        crate::core::PackageManager::Flatpak => "uninstall with Flatpak",
                        crate::core::PackageManager::Windows => "uninstall with Windows",
                        crate::core::PackageManager::MicrosoftStore => {
                            "uninstall with Microsoft Store"
                        }
                    };
                    hints.push((Key::ENTER, label, Command::Primary));
                }
            }
        }
        if primary_action(self.guards.as_ref()) == PrimaryAction::Remove {
            hints.push((Key::ENTER, "confirm", Command::Primary));
            hints.push((
                Key::letter('d'),
                "switch trash or delete",
                Command::ToggleDisposal,
            ));
        }
        hints.push((
            Key::letter('t'),
            "move to Trash anyway",
            Command::TrashAnyway,
        ));
        hints.push((Key::ESC, "back", Command::Back));
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(section_header(&plan.app.name))
            .children(self.guard_banner())
            .child(centered_column(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .child(
                                div()
                                    .id("qol-removeapp-items")
                                    .size_full()
                                    .track_scroll(&self.items_scroll)
                                    .overflow_y_scroll()
                                    .children(items),
                            )
                            .child(clear_of_resize_edge(kit.scroll_cue(
                                ScrollSource::Handle {
                                    handle: self.items_scroll.clone(),
                                    children: item_count,
                                },
                                kit.grounds.pane,
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(12.0))
                            .py(px(8.0))
                            .border_t_1()
                            .border_color(rgba(kit.grounds.pane.edge.packed()))
                            .child(
                                div()
                                    .text_color(rgb(disp_color))
                                    .text(TextStyle::Name)
                                    .child(disp_label),
                            )
                            .child(
                                div()
                                    .text_color(rgb(kit.grounds.pane.ink))
                                    .text(TextStyle::Detail)
                                    .child(format!(
                                        "{} \u{00b7} {}",
                                        qol_gpui::plural(plan.items.len(), "item", "items"),
                                        qol_gpui::format_bytes(plan.total_bytes)
                                    )),
                            ),
                    ),
            ))
            .child(footer(&hints, None, cx))
            .into_any_element()
    }

    fn render_working(
        &self,
        label: &'static str,
        closable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let Some(plan) = &self.plan else {
            return div().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(section_header(&plan.app.name))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .panel_drag_area()
                    .child(qol_gpui::Busy::new(
                        "qol-removeapp-working-busy",
                        label,
                        rgb(kit.grounds.pane.faint),
                    )),
            )
            .when(closable, |body| {
                body.child(footer(&[(Key::ESC, "close", Command::Quit)], None, cx))
            })
            .into_any_element()
    }

    fn guard_banner(&self) -> Option<AnyElement> {
        let kit = qol_gpui::kit::kit();
        let Some(g) = self.guards.as_ref() else {
            return Some(banner_container(vec![banner_busy(kit.grounds.pane.faint)]));
        };
        let mut lines: Vec<AnyElement> = Vec::new();
        if g.running {
            let text = if self.quit_failed {
                "The app is still running. Press Q to try quitting it again."
            } else {
                "The app is running. Press Q to quit it first."
            };
            lines.push(banner_line(kit.palette.warning, text).into_any_element());
        }
        match &g.package {
            PackageStatus::Managed(_) => lines.push(
                banner_line(
                    kit.palette.accent,
                    if g.running {
                        "Quit the app before uninstalling it."
                    } else {
                        "Press Enter to uninstall it properly."
                    },
                )
                .into_any_element(),
            ),
            PackageStatus::Unavailable(reason) => lines.push(
                banner_line(
                    kit.grounds.pane.faint,
                    &format!("Could not check how this app was installed. {reason}"),
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

    fn render_done(&self, cx: &mut Context<Self>) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let notice = if let Some(error) = &self.error {
            kit.notice(
                NoticeTone::Invalid,
                "Removal failed",
                Some(error.clone().into_any_element()),
            )
        } else {
            let Some(outcome) = &self.outcome else {
                return div().into_any_element();
            };
            let failed = outcome.failed.len();
            let tone = if failed > 0 {
                NoticeTone::Attention
            } else {
                NoticeTone::Done
            };
            let freed = format!("Freed {}", qol_gpui::format_bytes(outcome.freed_bytes));
            let detail = if failed > 0 {
                format!("{freed} \u{00B7} {failed} failed")
            } else {
                freed
            };
            kit.notice(
                tone,
                format!(
                    "Removed {}",
                    qol_gpui::plural(outcome.removed.len(), "item", "items")
                ),
                Some(detail.into_any_element()),
            )
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .justify_center()
            .gap(px(qol_gpui::theme::SPACE_CELL))
            .panel_drag_area()
            .child(notice.w_full())
            .child(
                div()
                    .flex()
                    .justify_center()
                    .child(continue_or_quit_hint(cx)),
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kit = qol_gpui::kit::kit();
        self.list.max_visible = rows_that_fit(f32::from(window.viewport_size().height));
        self.list.sync(self.matches.len());
        qol_gpui::kit::kit()
            .window()
            .id("qol-removeapp")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .text_color(rgb(kit.grounds.pane.ink))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| this.on_key(ev, cx)))
            .child(self.render_body(window, cx))
    }
}

fn uninstall_then_remove(
    plan: &RemovalPlan,
    package: ManagedPackage,
) -> anyhow::Result<RemovalOutcome> {
    core::uninstall_package(plan, &package)?;
    core::remove_after_package(
        plan,
        Disposal::Trash,
        &PackageStatus::Managed(package),
        true,
    )
}

fn rows_that_fit(window_height: f32) -> usize {
    (((window_height - PICKING_CHROME) / ROW_H).floor().max(0.0) as usize).max(1)
}

fn content_width(window_width: f32) -> f32 {
    window_width.min(CONTENT_MAX_W)
}

fn centered_column(content: gpui::Div) -> gpui::Div {
    div()
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .child(content.flex_1().min_h_0().w_full().max_w(px(CONTENT_MAX_W)))
}

fn clear_of_resize_edge(cue: impl IntoElement) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right(px(qol_gpui::resize_frame::EDGE_BAND))
        .child(cue)
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

fn confirm_command(key: &str) -> Option<Command> {
    match key {
        "escape" => Some(Command::Back),
        "q" => Some(Command::QuitApp),
        "t" => Some(Command::TrashAnyway),
        "enter" => Some(Command::Primary),
        "d" | "tab" => Some(Command::ToggleDisposal),
        _ => None,
    }
}

fn working_command(key: &str, closable: bool) -> Option<Command> {
    (closable && key == "escape").then_some(Command::Quit)
}

fn done_action(key: &str) -> Option<Command> {
    match key {
        "enter" => Some(Command::Continue),
        "escape" => Some(Command::Quit),
        _ => None,
    }
}

fn app_row(
    app: &InstalledApp,
    size: Option<u64>,
    selected: bool,
    protected: bool,
) -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    let ground = kit.highlight_ground(selected);
    let row = div()
        .flex_none()
        .h(px(ROW_H))
        .px(px(qol_gpui::theme::SPACE_PAD))
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(kit.letter_tile(&app.name))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text(TextStyle::ListName)
                .text_color(if protected {
                    rgb(kit.grounds.pane.faint)
                } else {
                    rgb(kit.grounds.pane.ink)
                })
                .child(app.name.clone()),
        )
        .when(protected, |d| {
            d.child(
                div()
                    .flex_none()
                    .text(TextStyle::Label)
                    .text_color(rgb(kit.palette.danger))
                    .child(cased(TextStyle::Label, "protected")),
            )
        })
        .child(match size {
            Some(size) => div()
                .flex_none()
                .text(TextStyle::Code)
                .text_color(rgb(ground.soft))
                .child(qol_gpui::format_bytes(size))
                .into_any_element(),
            None => qol_gpui::Busy::ring(
                ElementId::Name(format!("qol-removeapp-row-size-{}", app.path.display()).into()),
                rgb(ground.faint),
            )
            .into_any_element(),
        });
    kit.highlight(row, selected)
}

fn section_header(title: &str) -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    qol_gpui::kit::kit()
        .heading(format!("remove {}", title.to_lowercase()), None)
        .w_full()
        .bg(rgb(kit.grounds.rail.bg))
        .border_b_1()
        .border_color(rgba(kit.grounds.pane.edge.packed()))
        .panel_drag_area()
}

fn footer(
    hints: &[(Key, &str, Command)],
    counter: Option<String>,
    cx: &mut Context<RemoveAppView>,
) -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    kit.hint_bar()
        .children(hint_buttons(hints, cx))
        .child(div().flex_1())
        .when_some(counter, |bar, counter| {
            bar.child(
                div()
                    .flex_none()
                    .text(TextStyle::Code)
                    .text_color(rgb(kit.grounds.pane.faint))
                    .child(counter),
            )
        })
}

fn banner_container(lines: Vec<AnyElement>) -> AnyElement {
    let kit = qol_gpui::kit::kit();
    div()
        .flex()
        .flex_col()
        .w_full()
        .px(px(12.0))
        .py(px(6.0))
        .gap(px(qol_gpui::theme::SPACE_STACK))
        .bg(rgb(kit.grounds.attention.bg))
        .border_b_1()
        .border_color(rgba(kit.grounds.pane.edge.packed()))
        .children(lines)
        .into_any_element()
}

fn banner_frame(color: u32) -> gpui::Div {
    div().text(TextStyle::Detail).text_color(rgb(color))
}

fn banner_line(color: u32, text: &str) -> impl IntoElement {
    banner_frame(color).child(text.to_string())
}

fn banner_busy(color: u32) -> AnyElement {
    banner_frame(color)
        .child(qol_gpui::Busy::new(
            "qol-removeapp-guards-busy",
            "Checking how this app was installed",
            rgb(color),
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        confirm_command, content_width, done_action, primary_action, rows_that_fit,
        working_command, Command, PrimaryAction, CONTENT_MAX_W, FAILBAR_H, MIN_WINDOW_HEIGHT,
        ROW_H, SEARCH_H, SEARCH_PAD, WINDOW_HEIGHT, WINDOW_WIDTH,
    };
    use crate::core::{Guards, ManagedPackage, PackageManager, PackageScope, PackageStatus};

    #[test]
    fn rows_fill_every_height_and_are_maximal() {
        let chrome = 2.0 * qol_gpui::theme::LINE
            + SEARCH_PAD
            + SEARCH_H
            + SEARCH_PAD
            + FAILBAR_H
            + qol_gpui::theme::HEIGHT_HINT_BAR;
        let mut height = MIN_WINDOW_HEIGHT;
        while height <= 2160.0 {
            let rows = rows_that_fit(height);
            let fits = chrome + rows as f32 * ROW_H;
            let one_more = chrome + (rows + 1) as f32 * ROW_H;
            assert!(fits <= height, "{rows} rows fit {height}: {fits}");
            assert!(one_more > height, "{rows} is the most that fit {height}");
            height += 1.0;
        }
    }

    #[test]
    fn opening_size_keeps_nine_rows_and_minimum_keeps_three() {
        assert_eq!(rows_that_fit(WINDOW_HEIGHT), 9);
        assert_eq!(rows_that_fit(MIN_WINDOW_HEIGHT), 3);
        assert_eq!(rows_that_fit(100.0), 1);
    }

    #[test]
    fn minimum_fits_a_480_pixel_screen_with_a_panel() {
        let screen = 480.0;
        let panel = 48.0;
        assert!(
            MIN_WINDOW_HEIGHT <= screen - panel,
            "{MIN_WINDOW_HEIGHT} leaves no room on a {screen} px screen under a {panel} px panel"
        );
    }

    #[test]
    fn content_stops_widening_past_its_cap() {
        let cases = [
            (WINDOW_WIDTH, WINDOW_WIDTH),
            (CONTENT_MAX_W, CONTENT_MAX_W),
            (1920.0, CONTENT_MAX_W),
        ];
        for (window, expected) in cases {
            assert_eq!(content_width(window), expected, "window: {window}");
        }
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
    fn confirm_screen_maps_each_key_to_its_command() {
        let cases = [
            ("escape", Some(Command::Back)),
            ("q", Some(Command::QuitApp)),
            ("t", Some(Command::TrashAnyway)),
            ("enter", Some(Command::Primary)),
            ("d", Some(Command::ToggleDisposal)),
            ("tab", Some(Command::ToggleDisposal)),
            ("D", None),
            ("u", None),
            ("space", None),
            ("backspace", None),
        ];
        for (key, expected) in cases {
            assert_eq!(confirm_command(key), expected, "key: {key}");
        }
    }

    #[test]
    fn completed_screen_only_handles_enter_and_escape() {
        let cases = [
            ("enter", Some(Command::Continue)),
            ("escape", Some(Command::Quit)),
            ("u", None),
            ("space", None),
            ("q", None),
        ];

        for (key, expected) in cases {
            assert_eq!(done_action(key), expected);
        }
    }

    #[test]
    fn only_escape_closes_an_uninstall_wait_and_nothing_stops_a_removal() {
        let cases = [
            ("escape", true, Some(Command::Quit)),
            ("escape", false, None),
            ("enter", true, None),
            ("q", true, None),
            ("t", true, None),
            ("d", true, None),
            ("backspace", true, None),
        ];
        for (key, closable, expected) in cases {
            assert_eq!(
                working_command(key, closable),
                expected,
                "key: {key} closable: {closable}"
            );
        }
    }
}
