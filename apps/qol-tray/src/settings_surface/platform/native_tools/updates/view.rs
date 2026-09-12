use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::settings_panel::components::{
    settings_label, settings_label_group, settings_page, settings_value_group,
};
use qol_gpui::settings_panel::{
    adjacent_visible_row, escape_step, intent, settings_action_affordance, settings_action_spinner,
    settings_busy_message, settings_description, settings_value_text, CustomPanelCallback,
    CustomPanelNoticeTone, CustomPanelNotifier, CustomSettingsBreadcrumbs, EscapeStep, Intent,
    SettingsDestination, SettingsGroupHeader, SettingsRow, SettingsValueTone,
};
use qol_gpui::surface::SurfaceDismisser;
use qol_gpui::theme::{settings_panel_runtime, SettingsPanelPalette};

use super::data;
use super::model::{
    page, CheckRow, RowAction, Summary, SummaryDot, TargetRow, TargetState, UpdatesSnapshot,
    ValueTone,
};

const MAX_VISIBLE: usize = 10;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_STALE_AFTER: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    None,
    Summary,
    Check,
    Host,
    Plugin(String),
}

enum PageRow {
    Header { title: String, detail: &'static str },
    Summary(Summary),
    Check(CheckRow),
    Host(TargetRow),
    Plugin(TargetRow),
}

impl PageRow {
    fn action(&self) -> Option<&RowAction> {
        match self {
            PageRow::Header { .. } => None,
            PageRow::Summary(summary) => summary.action.as_ref(),
            PageRow::Check(check) => check.action.as_ref(),
            PageRow::Host(target) | PageRow::Plugin(target) => target.action.as_ref(),
        }
    }

    fn selection(&self) -> Option<Selection> {
        Some(match self {
            PageRow::Header { .. } => return None,
            PageRow::Summary(_) => Selection::Summary,
            PageRow::Check(_) => Selection::Check,
            PageRow::Host(_) => Selection::Host,
            PageRow::Plugin(target) => Selection::Plugin(target.id.clone()),
        })
    }

    fn action_label(&self) -> Option<&'static str> {
        match self {
            PageRow::Header { .. } => None,
            PageRow::Summary(summary) => summary.action_label,
            PageRow::Check(check) => check.action_label,
            PageRow::Host(target) | PageRow::Plugin(target) => target.action_label,
        }
    }
}

pub(super) struct UpdatesView {
    focus_handle: FocusHandle,
    body_focused: bool,
    dismisser: SurfaceDismisser,
    on_back: Option<CustomPanelCallback>,
    notify: CustomPanelNotifier,
    list: ScrollList,
    selection: Selection,
    selected: usize,
    snapshot: Option<UpdatesSnapshot>,
    visited: BTreeSet<String>,
    pending: bool,
    last_render: Option<Instant>,
    poll_skipped: bool,
}

impl UpdatesView {
    pub(super) fn new(
        dismisser: SurfaceDismisser,
        on_back: Option<CustomPanelCallback>,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = Self {
            focus_handle: cx.focus_handle(),
            body_focused: false,
            dismisser,
            on_back,
            notify,
            list: ScrollList::new(MAX_VISIBLE),
            selection: Selection::None,
            selected: 0,
            snapshot: None,
            visited: BTreeSet::new(),
            pending: false,
            last_render: None,
            poll_skipped: false,
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

    fn apply_snapshot(&mut self, snapshot: UpdatesSnapshot) {
        if self.poll_skipped {
            self.poll_skipped = false;
            self.visited.retain(|id| {
                snapshot.plugins.iter().any(|plugin| {
                    &plugin.id == id
                        && !matches!(plugin.state, TargetState::UpToDate | TargetState::DevLinked)
                })
            });
        }
        self.note_visited(&snapshot);
        self.snapshot = Some(snapshot);
    }

    fn note_visited(&mut self, snapshot: &UpdatesSnapshot) {
        for plugin in &snapshot.plugins {
            if !matches!(plugin.state, TargetState::UpToDate | TargetState::DevLinked) {
                self.visited.insert(plugin.id.clone());
            }
        }
    }

    fn page_rows(&self) -> (Vec<PageRow>, Option<String>) {
        let Some(snapshot) = &self.snapshot else {
            return (Vec::new(), None);
        };
        let model = page(snapshot, &self.visited);
        let mut rows = vec![
            PageRow::Header {
                title: "Status".to_string(),
                detail: "What the last check found.",
            },
            PageRow::Summary(model.summary),
            PageRow::Check(model.check),
            PageRow::Header {
                title: "qol-tray".to_string(),
                detail: "The tray itself.",
            },
            PageRow::Host(model.host),
            PageRow::Header {
                title: "Plugins".to_string(),
                detail: "Everything you installed.",
            },
        ];
        rows.extend(model.plugins.into_iter().map(PageRow::Plugin));
        (rows, model.fold)
    }

    fn sync_selection(&mut self, rows: &[PageRow]) {
        let navigable = navigable(rows);
        if navigable.is_empty() {
            self.selection = Selection::None;
            self.selected = 0;
            self.list.selected = 0;
            self.list.sync(rows.len());
            return;
        }
        let position = navigable
            .iter()
            .position(|(_, selection)| *selection == self.selection)
            .unwrap_or_else(|| nearest_navigable(&navigable, self.selected));
        self.select_position(&navigable, position, rows.len());
    }

    fn move_selection(&mut self, direction: isize) {
        let (rows, _) = self.page_rows();
        let navigable = navigable(&rows);
        if navigable.is_empty() {
            return;
        }
        let indices = navigable
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        let next = adjacent_visible_row(&indices, self.selected, direction);
        let position = indices.iter().position(|index| *index == next).unwrap_or(0);
        self.select_position(&navigable, position, rows.len());
    }

    fn select_position(&mut self, navigable: &[(usize, Selection)], position: usize, rows: usize) {
        self.selected = navigable[position].0;
        self.selection = navigable[position].1.clone();
        self.list.selected = self.selected;
        self.list.sync(rows);
    }

    fn select_row(&mut self, index: usize, selection: Selection, cx: &mut Context<Self>) {
        self.selected = index;
        self.selection = selection;
        self.activate(cx);
        cx.notify();
    }

    fn activate(&mut self, cx: &mut Context<Self>) {
        let (rows, _) = self.page_rows();
        let Some(action) = rows.get(self.selected).and_then(PageRow::action).cloned() else {
            return;
        };
        self.run_action(action, cx);
    }

    fn run_action(&mut self, action: RowAction, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let (name, body) = match action {
            RowAction::Check => ("check_updates", None),
            RowAction::UpdateAll => ("update_all", None),
            RowAction::Update(id) => ("update", Some(serde_json::json!({ "id": id }).to_string())),
        };
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { data::action(name, body.as_deref()) })
                    .await;
                match result {
                    Ok(()) => {
                        let snapshot = async_cx.background_spawn(async { data::load() }).await;
                        let _ = this.update(&mut async_cx, |view, cx| {
                            view.pending = false;
                            if let Ok(snapshot) = snapshot {
                                view.apply_snapshot(snapshot);
                            }
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        let _ = this.update(&mut async_cx, |view, cx| {
                            view.pending = false;
                            view.fail(&message, cx);
                            cx.notify();
                        });
                    }
                }
            }
        })
        .detach();
    }

    fn fail(&self, message: &str, cx: &mut Context<Self>) {
        (self.notify)(CustomPanelNoticeTone::Failure, message.to_string(), cx);
    }

    fn go_back(&self, window: &mut Window, cx: &mut App) {
        if let Some(on_back) = &self.on_back {
            on_back(window, cx);
        } else {
            self.dismisser.dismiss(cx);
        }
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match escape_step(0, false, self.on_back.is_some()) {
            EscapeStep::CloseFilter | EscapeStep::PopCard => {}
            EscapeStep::AscendRail | EscapeStep::Dismiss => self.go_back(window, cx),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        if matches!(key, "escape" | "esc") {
            self.escape(window, cx);
            cx.notify();
            return;
        }
        match intent(key, None, false) {
            Some(Intent::Up) => self.move_selection(-1),
            Some(Intent::Down) => self.move_selection(1),
            Some(Intent::Activate) => self.activate(cx),
            _ => return,
        }
        cx.notify();
    }

    fn render_body(
        &self,
        rows: &[PageRow],
        fold: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.snapshot.is_none() {
            return settings_busy_message(
                "updates-loading",
                "Loading updates",
                settings_panel_runtime(),
            )
            .into_any_element();
        }
        settings_page()
            .child(self.render_list(rows, fold, cx))
            .into_any_element()
    }

    fn render_list(
        &self,
        rows: &[PageRow],
        fold: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total = rows.len();
        let last = total.checked_sub(1);
        let mut list = div()
            .id("updates-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .on_scroll_wheel(
                cx.listener(|this: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    let rows = wheel_rows(&event.delta, qol_theme::HEIGHT_SETTING_ROW);
                    for _ in 0..rows.max(0) as usize {
                        this.move_selection(1);
                    }
                    for _ in 0..(-rows).max(0) as usize {
                        this.move_selection(-1);
                    }
                    cx.notify();
                }),
            );
        let cursor_group = self
            .selected
            .checked_add(1)
            .and_then(|end| rows.get(..end))
            .and_then(|seen| {
                seen.iter()
                    .rposition(|row| matches!(row, PageRow::Header { .. }))
            });
        for index in self.list.visible_range(total) {
            let current = self.body_focused && cursor_group == Some(index);
            list = list.child(self.render_row(index, &rows[index], current, cx));
            if Some(index) == last {
                list = list.children(fold.map(|text| self.render_fold(text)));
            }
        }
        list.into_any_element()
    }

    fn render_fold(&self, text: &str) -> Div {
        let palette = settings_panel_runtime();
        div()
            .flex_none()
            .h(px(qol_theme::HEIGHT_CONTROL))
            .flex()
            .items_center()
            .px(px(qol_theme::SPACE_INSET))
            .child(settings_description(text.to_string(), palette))
    }

    fn render_row(
        &self,
        index: usize,
        row: &PageRow,
        current: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings_panel_runtime();
        match row {
            PageRow::Header { title, detail } => {
                SettingsGroupHeader::new(title.clone(), Some((*detail).into()), palette)
                    .current(current)
                    .into_any_element()
            }
            PageRow::Summary(summary) => self.render_summary(index, summary, cx),
            PageRow::Check(check) => self.render_check(index, check, cx),
            PageRow::Host(target) => self.render_target(index, target, Selection::Host, cx),
            PageRow::Plugin(target) => {
                let selection = Selection::Plugin(target.id.clone());
                self.render_target(index, target, selection, cx)
            }
        }
    }

    fn render_summary(
        &self,
        index: usize,
        summary: &Summary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings_panel_runtime();
        let row = SettingsRow::setting(("updates-summary", index), palette)
            .selected(self.selected == index, self.body_focused)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.standard_click() {
                    this.select_row(index, Selection::Summary, cx);
                }
            }));
        let label = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_STACK))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(qol_theme::SPACE_INSET))
                    .children(summary.busy.then(|| {
                        settings_action_spinner(("updates-summary-spinner", index), palette)
                    }))
                    .children(summary.dot.map(|dot| summary_dot(dot, palette)))
                    .child(settings_label(summary.label.clone(), palette)),
            )
            .child(settings_description(summary.description.clone(), palette));
        row.child(label)
            .child(
                settings_value_group().children(summary.action_label.map(|label| {
                    settings_action_affordance(
                        ("updates-summary-action", index),
                        label,
                        None,
                        false,
                        palette,
                    )
                })),
            )
            .into_any_element()
    }

    fn render_check(&self, index: usize, check: &CheckRow, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let row = SettingsRow::setting("updates-checked", palette)
            .selected(self.selected == index, self.body_focused)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.standard_click() {
                    this.select_row(index, Selection::Check, cx);
                }
            }));
        row.child(settings_label_group(
            check.label,
            Some(check.description.clone().into()),
            palette,
        ))
        .child(
            settings_value_group()
                .children(
                    check.checking.then(|| {
                        settings_action_spinner(("updates-checked-spinner", index), palette)
                    }),
                )
                .child(settings_value_text(
                    check.value.clone(),
                    value_tone(check.tone),
                    palette,
                ))
                .children(check.action_label.map(|label| {
                    settings_action_affordance(
                        ("updates-checked-action", index),
                        label,
                        None,
                        false,
                        palette,
                    )
                })),
        )
        .into_any_element()
    }

    fn render_target(
        &self,
        index: usize,
        target: &TargetRow,
        selection: Selection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings_panel_runtime();
        let row = SettingsRow::setting(("updates-target", index), palette)
            .selected(self.selected == index, self.body_focused)
            .attention(target.attention)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.standard_click() {
                    this.select_row(index, selection.clone(), cx);
                }
            }));
        row.child(settings_label_group(
            target.name.clone(),
            target.description.clone().map(Into::into),
            palette,
        ))
        .child(
            settings_value_group()
                .children(
                    target.spinner.then(|| {
                        settings_action_spinner(("updates-target-spinner", index), palette)
                    }),
                )
                .child(settings_value_text(
                    target.value.clone(),
                    value_tone(target.tone),
                    palette,
                ))
                .children(target.action_label.map(|label| {
                    settings_action_affordance(
                        ("updates-target-action", index),
                        label,
                        None,
                        false,
                        palette,
                    )
                })),
        )
        .into_any_element()
    }
}

impl CustomSettingsBreadcrumbs for UpdatesView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        Vec::new()
    }

    fn settings_hints(&self) -> Option<Vec<(SharedString, SharedString)>> {
        let (rows, _) = self.page_rows();
        let mut hints = Vec::new();
        if let Some(label) = rows.get(self.selected).and_then(PageRow::action_label) {
            hints.push((
                SharedString::from("\u{21b5}"),
                SharedString::from(label.to_lowercase()),
            ));
        }
        hints.push((
            SharedString::from("\u{2191}\u{2193}"),
            SharedString::from("move"),
        ));
        Some(hints)
    }
}

impl Focusable for UpdatesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UpdatesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.body_focused = self.focus_handle.is_focused(window);
        self.last_render = Some(Instant::now());
        let (rows, fold) = self.page_rows();
        self.sync_selection(&rows);
        div()
            .id("qol-native-updates-body")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }),
            )
            .child(
                div()
                    .id("qol-native-updates-content")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_body(&rows, fold.as_deref(), cx)),
            )
    }
}

fn navigable(rows: &[PageRow]) -> Vec<(usize, Selection)> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| row.selection().map(|selection| (index, selection)))
        .collect()
}

fn nearest_navigable(navigable: &[(usize, Selection)], index: usize) -> usize {
    navigable
        .iter()
        .position(|(row, _)| *row >= index)
        .unwrap_or_else(|| navigable.len().saturating_sub(1))
}

fn summary_dot(dot: SummaryDot, palette: SettingsPanelPalette) -> Div {
    let kit = kit();
    let (tone, halo) = match dot {
        SummaryDot::Danger => (palette.status_danger, kit.washes.halo_invalid),
        SummaryDot::Warning => (palette.status_warning, kit.washes.halo_attention),
        SummaryDot::Success => (palette.status_success, kit.washes.halo_success),
    };
    kit.status_dot(tone, halo.packed())
}

fn value_tone(tone: ValueTone) -> SettingsValueTone {
    match tone {
        ValueTone::Normal => SettingsValueTone::Normal,
        ValueTone::Muted => SettingsValueTone::Muted,
        ValueTone::Attention => SettingsValueTone::Attention,
        ValueTone::Danger => SettingsValueTone::Danger,
        ValueTone::Success => SettingsValueTone::Success,
    }
}

#[cfg(test)]
mod tests {
    use super::{navigable, nearest_navigable, PageRow, Selection};
    use crate::settings_surface::platform::native_tools::updates::model::{
        CheckRow, Summary, TargetRow, ValueTone,
    };

    fn summary() -> Summary {
        Summary {
            dot: None,
            busy: false,
            label: "Everything is up to date".to_string(),
            description: "No updates are waiting".to_string(),
            action: None,
            action_label: None,
        }
    }

    fn check() -> CheckRow {
        CheckRow {
            label: "Update checks",
            value: "Off".to_string(),
            tone: ValueTone::Muted,
            description: "Development builds do not check for updates.".to_string(),
            action: None,
            action_label: None,
            checking: false,
        }
    }

    fn target() -> TargetRow {
        TargetRow {
            id: "qol-tray".to_string(),
            name: "Version".to_string(),
            value: "3.66.1".to_string(),
            tone: ValueTone::Muted,
            description: Some("Updates arrive through Recompile".to_string()),
            action: None,
            action_label: None,
            attention: false,
            spinner: false,
        }
    }

    #[test]
    fn every_row_but_a_header_navigates_even_without_an_action() {
        let rows = vec![
            PageRow::Header {
                title: "Status".to_string(),
                detail: "What the last check found.",
            },
            PageRow::Summary(summary()),
            PageRow::Check(check()),
            PageRow::Header {
                title: "qol-tray".to_string(),
                detail: "The tray itself.",
            },
            PageRow::Host(target()),
        ];
        let navigable = navigable(&rows);
        assert_eq!(
            navigable,
            vec![
                (1, Selection::Summary),
                (2, Selection::Check),
                (4, Selection::Host),
            ]
        );
    }

    #[test]
    fn nearest_navigable_keeps_the_cursor_close_when_a_row_disappears() {
        let navigable = [
            (2usize, Selection::Summary),
            (5, Selection::Check),
            (9, Selection::Host),
        ];
        let cases = [(0, 0), (2, 0), (3, 1), (5, 1), (8, 2), (40, 2)];
        for (index, expected) in cases {
            assert_eq!(
                nearest_navigable(&navigable, index),
                expected,
                "index={index}"
            );
        }
    }
}
