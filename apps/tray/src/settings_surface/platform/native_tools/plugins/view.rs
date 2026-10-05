use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    canvas, div, AnyElement, App, AsyncApp, Bounds, Context, Div, FocusHandle, Focusable,
    KeyDownEvent, Pixels, Render, ScrollWheelEvent, WeakEntity, Window,
};
use qol_gpui::deck;
use qol_gpui::key::Key;
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::settings_panel::components::{
    settings_label, settings_label_group, settings_page, settings_value_group, RowGround,
    SettingsHint, SettingsTextField,
};
use qol_gpui::settings_panel::{
    adjacent_visible_row, escape_step, intent, settings_action_affordance, settings_action_spinner,
    settings_busy_message, settings_list, settings_value_text, CustomHints, CustomPanelCallback,
    CustomPanelNoticeTone, CustomPanelNotifier, CustomSettingsBreadcrumbs, EscapeStep, Intent,
    SettingsDestination, SettingsGroupHeader, SettingsRow,
};
use qol_gpui::text_edit::{self, TextField};

use super::data;
use super::model::{self, Enter, Level, Row, RowAction, RowKind, Snapshot, Visit};

const MAX_VISIBLE: usize = 10;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_STALE_AFTER: Duration = Duration::from_secs(5);
const SOURCES_DEPTH: usize = 1;
const SOURCES_ANIMATION: &str = "plugins-sources-slide";

pub(super) struct PluginsView {
    focus: FocusHandle,
    on_back: CustomPanelCallback,
    notify: CustomPanelNotifier,
    snapshot: Option<Snapshot>,
    level: Level,
    return_to: usize,
    requested: Vec<String>,
    seen: BTreeSet<String>,
    naming: bool,
    adding: bool,
    field: TextField,
    selected: usize,
    list: ScrollList,
    last_render: Option<Instant>,
    poll_skipped: bool,
    body_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    row_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    step: usize,
    motion: Option<deck::Motion>,
    mark: Option<f32>,
    closing: bool,
}

impl PluginsView {
    pub(super) fn new(
        on_back: CustomPanelCallback,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = Self {
            focus: cx.focus_handle(),
            on_back,
            notify,
            snapshot: None,
            level: Level::Plugins,
            return_to: 1,
            requested: Vec::new(),
            seen: BTreeSet::new(),
            naming: false,
            adding: false,
            field: TextField::new(),
            selected: 1,
            list: ScrollList::new(MAX_VISIBLE),
            last_render: None,
            poll_skipped: false,
            body_bounds: Rc::new(Cell::new(None)),
            row_bounds: Rc::new(RefCell::new(HashMap::new())),
            step: 0,
            motion: None,
            mark: None,
            closing: false,
        };
        Self::spawn_poll(cx);
        view
    }

    fn spawn_poll(cx: &mut Context<Self>) {
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                loop {
                    let Ok(due) = this.update(&mut async_cx, |view, _| view.take_poll_tick())
                    else {
                        break;
                    };
                    if due {
                        let result = async_cx.background_spawn(async { data::load() }).await;
                        let alive = this
                            .update(&mut async_cx, |view, cx| {
                                if let Ok(snapshot) = result {
                                    view.apply_snapshot(snapshot);
                                }
                                cx.notify();
                            })
                            .is_ok();
                        if !alive {
                            break;
                        }
                    }
                    async_cx.background_executor().timer(POLL_INTERVAL).await;
                }
            }
        })
        .detach();
    }

    fn take_poll_tick(&mut self) -> bool {
        let due = self
            .last_render
            .is_none_or(|last| last.elapsed() < POLL_STALE_AFTER);
        if !due {
            self.poll_skipped = true;
        }
        due
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        if self.poll_skipped {
            self.poll_skipped = false;
            self.seen.retain(|id| {
                snapshot
                    .plugins
                    .iter()
                    .any(|plugin| &plugin.id == id && model::in_queue(plugin.state))
            });
        }
        self.seen.extend(
            snapshot
                .plugins
                .iter()
                .filter(|plugin| model::in_queue(plugin.state))
                .map(|plugin| plugin.id.clone()),
        );
        self.snapshot = Some(snapshot);
    }

    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        match self.level {
            Level::Plugins => model::plugin_rows(
                snapshot,
                &Visit {
                    requested: &self.requested,
                    seen: &self.seen,
                },
            ),
            Level::Sources => model::source_rows(snapshot, self.naming),
        }
    }

    fn leaving_rows(&self) -> Vec<Row> {
        self.snapshot
            .as_ref()
            .map(|snapshot| model::source_rows(snapshot, false))
            .unwrap_or_default()
    }

    fn run(&mut self, action: RowAction, cx: &mut Context<Self>) {
        if let RowAction::Install(id) = &action {
            self.requested.push(id.clone());
            self.seen.insert(id.clone());
        }
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let sent = action.clone();
                let result = async_cx
                    .background_spawn(async move { data::run(&sent) })
                    .await;
                let snapshot = async_cx.background_spawn(async { data::load() }).await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    if let Ok(snapshot) = snapshot {
                        view.apply_snapshot(snapshot);
                    }
                    view.settle(&action, result.is_ok());
                    if let Err(error) = result {
                        view.fail(format!("{error:#}"), cx);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn settle(&mut self, action: &RowAction, succeeded: bool) {
        match action {
            RowAction::Install(id) => self.requested.retain(|sent| sent != id),
            RowAction::AddSource(_) => {
                self.adding = false;
                if succeeded {
                    self.stop_naming();
                }
            }
            _ => {}
        }
    }

    fn fail(&self, message: String, cx: &mut Context<Self>) {
        (self.notify)(CustomPanelNoticeTone::Failure, message, cx);
    }

    fn activate(&mut self, cx: &mut Context<Self>) {
        let Some((enter, _)) = self
            .rows()
            .get(self.selected)
            .and_then(|row| row.enter.clone())
        else {
            return;
        };
        match enter {
            Enter::OpenSources => self.open_sources(),
            Enter::StartAdd => {
                self.naming = true;
                self.field.clear();
            }
            Enter::CommitAdd => self.commit_source(cx),
            Enter::Run(action) => self.run(action, cx),
        }
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if let Some((action, _)) = self
            .rows()
            .get(self.selected)
            .and_then(|row| row.delete.clone())
        {
            self.run(action, cx);
        }
    }

    fn commit_source(&mut self, cx: &mut Context<Self>) {
        if self.adding {
            return;
        }
        self.adding = true;
        let repo = self.field.text().trim().to_owned();
        self.run(RowAction::AddSource(repo), cx);
    }

    fn stop_naming(&mut self) {
        self.naming = false;
        self.field.clear();
    }

    fn open_sources(&mut self) {
        self.mark = self.row_mark(self.selected);
        self.return_to = self.selected;
        self.level = Level::Sources;
        self.selected = 0;
        self.closing = false;
        self.step = self.step.wrapping_add(1);
        self.motion = Some(deck::Motion::Push);
        self.list.reset();
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        self.level = Level::Plugins;
        self.stop_naming();
        self.selected = self.return_to;
        self.step = self.step.wrapping_add(1);
        self.closing = true;
        self.list.reset();
        deck::after_transition(cx, |view, cx| {
            view.closing = false;
            cx.notify();
        });
    }

    fn row_mark(&self, index: usize) -> Option<f32> {
        let row = self.row_bounds.borrow().get(&index).copied()?;
        let body = self.body_bounds.get()?;
        Some((row.origin.y + row.size.height / 2.0 - body.origin.y).to_f64() as f32)
    }

    fn body_width(&self) -> f32 {
        self.body_bounds
            .get()
            .map(|bounds| bounds.size.width.to_f64() as f32)
            .unwrap_or(0.0)
    }

    fn sliver_click(cx: &mut Context<Self>) -> deck::SliverClick {
        let view = cx.weak_entity();
        Rc::new(move |_, _, cx| {
            let _ = view.update(cx, |view, cx| {
                if view.level == Level::Sources {
                    view.leave(cx);
                }
                cx.notify();
            });
        })
    }

    fn navigable(rows: &[Row]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| row.kind != RowKind::Header)
            .map(|(index, _)| index)
            .collect()
    }

    fn move_selection(&mut self, direction: isize) {
        let rows = self.rows();
        let navigable = Self::navigable(&rows);
        if navigable.is_empty() {
            return;
        }
        self.selected = adjacent_visible_row(&navigable, self.selected, direction);
        self.list.selected = self.selected;
        self.list.sync(rows.len());
    }

    fn sync_selection(&mut self, rows: &[Row]) {
        let navigable = Self::navigable(rows);
        self.selected = navigable
            .iter()
            .copied()
            .find(|index| *index >= self.selected)
            .or_else(|| navigable.last().copied())
            .unwrap_or(0);
        self.list.selected = self.selected;
        self.list.sync(rows.len());
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match escape_step(usize::from(self.level == Level::Sources), false, true) {
            EscapeStep::CloseFilter => {}
            EscapeStep::PopCard => self.leave(cx),
            EscapeStep::AscendRail | EscapeStep::Dismiss => (self.on_back)(window, cx),
        }
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        match intent(key, event.keystroke.key_char.as_deref(), self.naming) {
            Some(Intent::CancelEdit) => self.stop_naming(),
            Some(Intent::CommitEdit) => self.commit_source(cx),
            _ if self.naming => {
                text_edit::apply_edit_key(&mut self.field, &event.keystroke, || {
                    cx.read_from_clipboard().and_then(|item| item.text())
                });
            }
            Some(Intent::Close) => {
                self.escape(window, cx);
                return;
            }
            Some(Intent::Up) => self.move_selection(-1),
            Some(Intent::Down) => self.move_selection(1),
            Some(Intent::Activate) => self.activate(cx),
            _ if matches!(key, "backspace" | "delete") => self.delete(cx),
            _ => return,
        }
        cx.notify();
    }

    fn render_row(
        &self,
        index: usize,
        row: &Row,
        current_group: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = kit();
        if row.kind == RowKind::Header {
            return SettingsGroupHeader::new(
                row.label.clone(),
                row.description.clone().map(Into::into),
                kit,
            )
            .current(current_group && focused)
            .into_any_element();
        }
        let selected = self.selected == index;
        let ground = RowGround::of(selected, focused);
        let on_click = cx.listener(move |view: &mut Self, _, _, cx| {
            if view.naming {
                return;
            }
            view.selected = index;
            view.activate(cx);
            cx.notify();
        });
        if row.kind == RowKind::Add {
            return SettingsRow::add(("plugins-add", index), kit)
                .selected(selected, focused)
                .on_click(on_click)
                .child(settings_label(row.label.clone(), kit))
                .into_any_element();
        }
        let mut values = settings_value_group();
        if row.kind == RowKind::Field {
            values = values
                .children(
                    self.adding
                        .then(|| settings_action_spinner(("plugins-adding", index), kit)),
                )
                .child(SettingsTextField::live(self.field.clone(), ground, kit));
        } else {
            values = values
                .children(
                    row.spinner
                        .then(|| settings_action_spinner(("plugins-spinner", index), kit)),
                )
                .children(
                    row.value
                        .clone()
                        .map(|value| settings_value_text(value, row.tone, ground, kit)),
                )
                .children(row.affordance.map(|label| {
                    settings_action_affordance(
                        ("plugins-action", index),
                        label,
                        None,
                        false,
                        ground,
                        kit,
                    )
                }));
        }
        SettingsRow::setting(("plugins-row", index), kit)
            .selected(selected, focused)
            .on_click(on_click)
            .child(settings_label_group(
                row.label.clone(),
                row.description.clone().map(Into::into),
                ground,
                kit,
            ))
            .child(values)
            .child(bounds_recorder(Rc::clone(&self.row_bounds), index))
            .into_any_element()
    }

    fn render_list(&self, rows: &[Row], focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let total = rows.len();
        let cursor_group = rows
            .get(..=self.selected.min(total.saturating_sub(1)))
            .and_then(|seen| seen.iter().rposition(|row| row.kind == RowKind::Header));
        let range = self.list.visible_range(total);
        let mut list = settings_list()
            .id("plugins-list")
            .on_scroll_wheel(
                cx.listener(|view: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    let steps = wheel_rows(&event.delta, qol_theme::HEIGHT_SETTING_ROW);
                    for _ in 0..steps.unsigned_abs() {
                        view.move_selection(steps.signum());
                    }
                    cx.notify();
                }),
            );
        for index in range.clone() {
            list = list.child(self.render_row(
                index,
                &rows[index],
                cursor_group == Some(index),
                focused,
                cx,
            ));
        }
        list.child(kit().scroll_cue(
            ScrollSource::Window {
                first: range.start,
                shown: range.len(),
                total,
            },
            kit().grounds.pane,
        ))
        .into_any_element()
    }
}

impl CustomSettingsBreadcrumbs for PluginsView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        match self.level {
            Level::Plugins => Vec::new(),
            Level::Sources => SettingsDestination::new("sources")
                .ok()
                .into_iter()
                .collect(),
        }
    }

    fn settings_hints(&self) -> Option<CustomHints> {
        if self.naming {
            return Some(CustomHints {
                question: None,
                left: vec![
                    SettingsHint::new(Key::ENTER, "add"),
                    SettingsHint::new(Key::TYPE, "owner/repo"),
                ],
                right: vec![SettingsHint::new(Key::ESC, "cancel")],
            });
        }
        let rows = self.rows();
        let row = rows.get(self.selected);
        let mut left = Vec::new();
        if let Some((_, verb)) = row.and_then(|row| row.enter.as_ref()) {
            left.push(SettingsHint::new(Key::ENTER, *verb));
        }
        if let Some((_, verb)) = row.and_then(|row| row.delete.as_ref()) {
            left.push(SettingsHint::new(Key::DELETE, *verb));
        }
        left.push(SettingsHint::new(Key::UP_DOWN, "move"));
        Some(CustomHints {
            question: None,
            left,
            right: Vec::new(),
        })
    }
}

impl Focusable for PluginsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PluginsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.last_render = Some(Instant::now());
        let focused = self.focus.is_focused(window);
        let rows = self.rows();
        self.sync_selection(&rows);
        let body = if self.snapshot.is_none() {
            settings_page()
                .child(settings_busy_message(
                    "plugins-loading",
                    "Reading your plugins",
                    kit(),
                ))
                .into_any_element()
        } else {
            self.render_body(&rows, focused, cx)
        };
        let bounds = Rc::clone(&self.body_bounds);
        div()
            .id("plugins-body")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(
                canvas(
                    move |measured, _, _| bounds.set(Some(measured)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(body)
    }
}

impl PluginsView {
    fn render_body(&self, rows: &[Row], focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let kit = kit();
        let width = self.body_width();
        let list = settings_page().child(self.render_list(rows, focused, cx));
        if self.level == Level::Sources {
            return deck_shell(deck::render(
                kit,
                list,
                deck::DeckFrame {
                    depth: SOURCES_DEPTH,
                    slide: deck::slide(self.step, self.motion, SOURCES_DEPTH, width),
                    closing: None,
                    animation_id: SOURCES_ANIMATION,
                    marks: vec![self.mark],
                    on_sliver: Some(Self::sliver_click(cx)),
                },
            ));
        }
        if self.closing {
            let leaving = settings_page().child(self.render_list(&self.leaving_rows(), false, cx));
            return deck_shell(deck::reveal(
                kit,
                list,
                leaving,
                deck::exit(self.step, SOURCES_DEPTH, width),
            ));
        }
        list.into_any_element()
    }
}

fn deck_shell(deck: Div) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_row()
        .items_start()
        .child(deck)
        .into_any_element()
}

fn bounds_recorder(store: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>, index: usize) -> AnyElement {
    canvas(
        move |bounds, _, _| {
            store.borrow_mut().insert(index, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
    .into_any_element()
}
