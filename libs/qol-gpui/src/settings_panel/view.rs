use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_config::contract::ResolvedRowAction;

use super::color_wheel::{ColorWheel, ColorWheelPopup, WheelCallbacks, WheelStyle};
use super::persistence::save_values;
use super::rows::{
    apply_runtime_query, begin_list_item_action, list_item_actions, merged_config,
    primary_list_item_action, query_flag_value, runtime_query_names, section_label_for, Row,
    RowControl, RowSection, SelectOption,
};
use super::{SettingsPanel, SettingsRuntime};
use crate::dropdown::{Dropdown, DropdownEvent, DropdownItem, DropdownStyle};
use crate::spinner::Spinner;
use crate::status_indicator::{StatusIndicator, StatusTone};
use crate::surface::{PanelDragArea, SurfaceDismisser};
use crate::theme::{settings_panel_runtime, SettingsPanelPalette};

pub(super) struct SettingsPanelView {
    panel: SettingsPanel,
    runtime: SettingsRuntime,
    runtime_queries: Vec<String>,
    rows: Vec<Row>,
    sections: Vec<RowSection>,
    active_section: Option<usize>,
    selected_section: usize,
    values: serde_json::Value,
    path: PathBuf,
    selected: usize,
    scroll_offset: usize,
    body_max: f32,
    active_control: Option<ActiveControl>,
    row_bounds: Vec<Rc<Cell<Option<Bounds<Pixels>>>>>,
    wheel_generation: u64,
    runtime_poll_generation: u64,
    dismisser: SurfaceDismisser,
    palette: SettingsPanelPalette,
    focus_handle: FocusHandle,
}

pub(super) struct SettingsPanelState {
    pub(super) rows: Vec<Row>,
    pub(super) sections: Vec<RowSection>,
    pub(super) values: serde_json::Value,
    pub(super) path: PathBuf,
    pub(super) body_max: f32,
    pub(super) runtime: SettingsRuntime,
}

enum ActiveControl {
    Edit(String),
    Dropdown(Dropdown),
    List,
    ListActions(ListActionMenu),
    Wheel(WheelControl),
}

struct ListActionMenu {
    row: usize,
    item_id: String,
    dropdown: Dropdown,
    actions: Vec<ResolvedRowAction>,
}

struct WheelControl {
    generation: u64,
    row: usize,
    value: String,
    popup: WindowHandle<ColorWheelPopup>,
}

impl SettingsPanelView {
    pub(super) fn new(
        panel: SettingsPanel,
        state: SettingsPanelState,
        dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        let row_bounds = (0..state.rows.len())
            .map(|_| Rc::new(Cell::new(None)))
            .collect();
        let runtime_queries = runtime_query_names(&state.rows);
        cx.on_release(|view, cx| view.close_wheel_popup(cx))
            .detach();
        let mut view = Self {
            panel,
            runtime: state.runtime,
            runtime_queries,
            rows: state.rows,
            active_section: initial_active_section(state.sections.len()),
            sections: state.sections,
            selected_section: 0,
            values: state.values,
            path: state.path,
            selected: 0,
            scroll_offset: 0,
            body_max: state.body_max,
            active_control: None,
            row_bounds,
            wheel_generation: 0,
            runtime_poll_generation: 0,
            dismisser,
            palette: settings_panel_runtime(),
            focus_handle: cx.focus_handle(),
        };
        view.selected = view.current_visible_rows().into_iter().next().unwrap_or(0);
        view.resume_runtime_poll(cx);
        view
    }

    fn current_visible_rows(&self) -> Vec<usize> {
        let Some(section) = self
            .active_section
            .and_then(|index| self.sections.get(index))
        else {
            return Vec::new();
        };
        section
            .rows
            .iter()
            .copied()
            .filter(|index| super::rows::row_is_visible(&self.rows, *index))
            .collect()
    }

    fn section_menu_is_open(&self) -> bool {
        self.sections.len() > 1 && self.active_section.is_none()
    }

    fn open_selected_section(&mut self) {
        if self.sections.is_empty() {
            return;
        }
        self.active_section = Some(self.selected_section.min(self.sections.len() - 1));
        self.selected = self.current_visible_rows().into_iter().next().unwrap_or(0);
        self.scroll_offset = self.selected;
        self.active_control = None;
    }

    fn open_section_menu(&mut self) {
        let Some(active) = self.active_section else {
            return;
        };
        self.selected_section = active;
        self.active_section = None;
        self.active_control = None;
        self.scroll_offset = 0;
    }

    fn on_section_menu_key(&mut self, key: &str, cx: &mut Context<Self>) {
        match key {
            "up" => self.selected_section = self.selected_section.saturating_sub(1),
            "down" => {
                self.selected_section =
                    (self.selected_section + 1).min(self.sections.len().saturating_sub(1));
            }
            "enter" | "return" | "space" | "right" => self.open_selected_section(),
            "escape" | "left" => {
                self.pause_runtime_poll();
                self.dismisser.dismiss(cx);
                return;
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn resume_runtime_poll(&mut self, cx: &mut Context<Self>) {
        let initial_delay = self
            .rows
            .iter()
            .any(|row| {
                matches!(&row.control, RowControl::Action { pending: true, .. })
                    || matches!(
                        &row.control,
                        RowControl::List { items, .. }
                            if items.iter().any(|item| item.pending)
                    )
            })
            .then_some(self.runtime.poll_interval);
        self.start_runtime_poll(initial_delay, cx);
    }

    fn start_runtime_poll(
        &mut self,
        initial_delay: Option<std::time::Duration>,
        cx: &mut Context<Self>,
    ) {
        if self.runtime_queries.is_empty() {
            return;
        }
        self.runtime_poll_generation = self.runtime_poll_generation.wrapping_add(1);
        let generation = self.runtime_poll_generation;
        let runtime = self.runtime.clone();
        let queries = self.runtime_queries.clone();
        let interval = runtime.poll_interval;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                if let Some(initial_delay) = initial_delay {
                    async_cx.background_executor().timer(initial_delay).await;
                }
                loop {
                    let active = this
                        .update(&mut async_cx, |this, _| {
                            this.runtime_poll_generation == generation
                        })
                        .unwrap_or(false);
                    if !active {
                        break;
                    }
                    let query_runtime = runtime.clone();
                    let query_names = queries.clone();
                    let results = async_cx
                        .background_spawn(async move {
                            query_names
                                .into_iter()
                                .map(|query| {
                                    let result = query_runtime.query(&query);
                                    (query, result)
                                })
                                .collect::<Vec<_>>()
                        })
                        .await;
                    let applied = this
                        .update(&mut async_cx, |this, cx| {
                            if this.runtime_poll_generation != generation {
                                return false;
                            }
                            for (query, result) in results {
                                apply_runtime_query(&mut this.rows, &query, result);
                            }
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !applied {
                        break;
                    }
                    async_cx.background_executor().timer(interval).await;
                }
            }
        })
        .detach();
    }

    fn pause_runtime_poll(&mut self) {
        self.runtime_poll_generation = self.runtime_poll_generation.wrapping_add(1);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let key_char = event.keystroke.key_char.as_deref();
        if self.section_menu_is_open() {
            self.on_section_menu_key(key, cx);
            return;
        }
        if let Some(ActiveControl::Wheel(wheel)) = &self.active_control {
            let popup = wheel.popup;
            let _ = popup.update(cx, |popup, popup_window, popup_cx| {
                popup.handle_key(key, event.keystroke.modifiers.shift, popup_window, popup_cx);
            });
            return;
        }
        if matches!(self.active_control, Some(ActiveControl::ListActions(_))) {
            self.on_list_actions_key(key, cx);
            return;
        }
        if matches!(self.active_control, Some(ActiveControl::Dropdown(_))) {
            self.on_dropdown_key(key, cx);
            return;
        }
        if matches!(self.active_control, Some(ActiveControl::List)) {
            self.on_list_key(key, cx);
            return;
        }
        let editing = matches!(self.active_control, Some(ActiveControl::Edit(_)));
        if editing && matches!(key, "up" | "down") {
            self.commit_edit();
        }
        let Some(intent) = intent(key, key_char, editing) else {
            if self.begin_number_entry(key_char) {
                cx.notify();
            }
            return;
        };
        match intent {
            Intent::Up => {
                self.selected =
                    adjacent_visible_row(&self.current_visible_rows(), self.selected, -1);
                self.sync_scroll();
            }
            Intent::Down => {
                self.selected =
                    adjacent_visible_row(&self.current_visible_rows(), self.selected, 1);
                self.sync_scroll();
            }
            Intent::Toggle => self.toggle(),
            Intent::Left => {
                let navigates_back = self
                    .rows
                    .get(self.selected)
                    .is_some_and(|row| left_navigates_back(&row.control, self.sections.len() > 1));
                if navigates_back {
                    self.open_section_menu();
                    cx.notify();
                    return;
                }
                self.adjust(-1.0);
            }
            Intent::Right => self.adjust(1.0),
            Intent::Activate => self.activate(window, cx),
            Intent::CommitEdit => self.commit_edit(),
            Intent::Backspace => {
                if let Some(ActiveControl::Edit(edit)) = self.active_control.as_mut() {
                    edit.pop();
                }
            }
            Intent::Insert(ch) => {
                if let Some(ActiveControl::Edit(edit)) = self.active_control.as_mut() {
                    edit.push_str(&ch);
                }
            }
            Intent::CancelEdit => self.active_control = None,
            Intent::Close => {
                if self.sections.len() > 1 {
                    self.open_section_menu();
                    cx.notify();
                    return;
                }
                self.pause_runtime_poll();
                self.dismisser.dismiss(cx);
                return;
            }
        }
        cx.notify();
    }

    fn open_color_wheel(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let RowControl::Color(value) = &row.control else {
            return;
        };
        let Some(anchor) = self.row_bounds.get(index).and_then(|bounds| bounds.get()) else {
            return;
        };
        let value = value.clone();
        self.close_wheel_popup(cx);
        self.selected = index;
        self.sync_scroll();
        self.wheel_generation = self.wheel_generation.wrapping_add(1);
        let generation = self.wheel_generation;
        let wheel = ColorWheel::open(&value);
        let preview = wheel.hex();
        let preview_parent = cx.weak_entity();
        let commit_parent = preview_parent.clone();
        let Some(popup) = ColorWheelPopup::open(
            wheel,
            self.wheel_style(),
            anchor,
            window,
            self.focus_handle.clone(),
            WheelCallbacks::new(
                move |value, cx| {
                    let _ = preview_parent.update(cx, |parent, cx| {
                        parent.preview_wheel(generation, value, cx);
                    });
                },
                move |value, cx| {
                    let _ = commit_parent.update(cx, |parent, cx| {
                        parent.commit_wheel(generation, value, cx);
                    });
                },
            ),
            cx,
        ) else {
            return;
        };
        self.active_control = Some(ActiveControl::Wheel(WheelControl {
            generation,
            row: index,
            value: preview,
            popup,
        }));
        cx.notify();
    }

    fn preview_wheel(&mut self, generation: u64, value: String, cx: &mut Context<Self>) {
        let Some(ActiveControl::Wheel(wheel)) = self.active_control.as_mut() else {
            return;
        };
        if wheel.generation != generation {
            return;
        }
        wheel.value = value;
        cx.notify();
    }

    fn commit_wheel(&mut self, generation: u64, value: String, cx: &mut Context<Self>) {
        let Some(ActiveControl::Wheel(wheel)) = self.active_control.as_ref() else {
            return;
        };
        if wheel.generation != generation {
            return;
        }
        let row_index = wheel.row;
        self.active_control = None;
        let Some(row) = self.rows.get_mut(row_index) else {
            return;
        };
        let RowControl::Color(row_value) = &mut row.control else {
            return;
        };
        *row_value = value;
        self.persist();
        cx.notify();
    }

    fn close_wheel_popup(&mut self, cx: &mut App) {
        if !matches!(self.active_control, Some(ActiveControl::Wheel(_))) {
            return;
        }
        let Some(ActiveControl::Wheel(wheel)) = self.active_control.take() else {
            return;
        };
        let _ = wheel
            .popup
            .update(cx, |_, window, _| window.remove_window());
    }

    fn on_dropdown_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(ActiveControl::Dropdown(dropdown)) = self.active_control.as_mut() else {
            return;
        };
        let Some(event) = dropdown.handle_key(key) else {
            return;
        };
        match event {
            DropdownEvent::Moved => {}
            DropdownEvent::Pick(_) => self.pick_dropdown(),
            DropdownEvent::Close => self.active_control = None,
        }
        cx.notify();
    }

    fn on_list_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(intent) = list_intent(key) else {
            return;
        };
        if intent == ListIntent::Activate {
            self.dispatch_list_action(cx);
            return;
        }
        if intent == ListIntent::Actions {
            self.open_list_actions();
            cx.notify();
            return;
        }
        let Some(row) = self.rows.get_mut(self.selected) else {
            self.active_control = None;
            return;
        };
        let RowControl::List { items, list, .. } = &mut row.control else {
            self.active_control = None;
            return;
        };
        match intent {
            ListIntent::Up => list.move_up(),
            ListIntent::Down => list.move_down(items.len()),
            ListIntent::Close => self.active_control = None,
            ListIntent::Activate | ListIntent::Actions => return,
        }
        list.sync(items.len());
        cx.notify();
    }

    fn on_list_actions_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(ActiveControl::ListActions(menu)) = self.active_control.as_mut() else {
            return;
        };
        let Some(event) = menu.dropdown.handle_key(key) else {
            return;
        };
        match event {
            DropdownEvent::Moved => {}
            DropdownEvent::Pick(selected) => self.dispatch_list_menu_action(selected, cx),
            DropdownEvent::Close => self.active_control = Some(ActiveControl::List),
        }
        cx.notify();
    }

    fn pick_dropdown(&mut self) {
        let Some(ActiveControl::Dropdown(dropdown)) = self.active_control.as_ref() else {
            return;
        };
        let pick = dropdown.selected();
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Select { options, index, .. } => {
                if pick < options.len() {
                    *index = pick;
                    self.persist();
                }
                self.active_control = None;
            }
            RowControl::MultiSelect { selected, .. } => {
                if let Some(flag) = selected.get_mut(pick) {
                    *flag = !*flag;
                    self.persist();
                }
            }
            RowControl::Toggle(_)
            | RowControl::Number { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_)
            | RowControl::Action { .. }
            | RowControl::Status { .. }
            | RowControl::List { .. } => self.active_control = None,
        }
    }

    fn toggle(&mut self) {
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        if let RowControl::Toggle(value) = &mut row.control {
            *value = !*value;
            self.persist();
        }
    }

    fn adjust(&mut self, direction: f64) {
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Select { options, index, .. } => {
                let len = options.len();
                if len == 0 {
                    return;
                }
                *index = (*index + if direction > 0.0 { 1 } else { len - 1 }) % len;
            }
            RowControl::Number {
                value,
                min,
                max,
                step,
            } => {
                let mut next = *value + direction * *step;
                if let Some(min) = min {
                    next = next.max(*min);
                }
                if let Some(max) = max {
                    next = next.min(*max);
                }
                *value = next;
            }
            RowControl::Toggle(_)
            | RowControl::MultiSelect { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_)
            | RowControl::Action { .. }
            | RowControl::Status { .. }
            | RowControl::List { .. } => return,
        }
        self.persist();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        match &row.control {
            RowControl::Toggle(_) => self.toggle(),
            RowControl::Select { options, index, .. } => {
                self.active_control = Some(ActiveControl::Dropdown(Dropdown::open(
                    options.len(),
                    *index,
                )));
            }
            RowControl::MultiSelect { options, .. } => {
                self.active_control =
                    Some(ActiveControl::Dropdown(Dropdown::open(options.len(), 0)));
            }
            RowControl::Color(_) => self.open_color_wheel(self.selected, window, cx),
            RowControl::Action { .. } => self.dispatch_action(cx),
            RowControl::Status { .. } => {}
            RowControl::List { items, .. } if !items.is_empty() => {
                self.active_control = Some(ActiveControl::List)
            }
            RowControl::List { .. } => {}
            RowControl::Number { .. } | RowControl::Text(_) | RowControl::TextList(_) => {
                self.begin_edit()
            }
        }
    }

    fn dispatch_action(&mut self, cx: &mut Context<Self>) {
        let Some(RowControl::Action {
            action,
            active_action,
            active_query,
            active_value_from,
            active,
            pending,
            error,
            ..
        }) = self.rows.get_mut(self.selected).map(|row| &mut row.control)
        else {
            return;
        };
        if *pending {
            return;
        }
        let action = if *active {
            active_action.as_ref().unwrap_or(action)
        } else {
            action
        }
        .clone();
        *pending = true;
        *error = None;
        let row_index = self.selected;
        let refresh_query = active_query.clone();
        let refresh_value_from = active_value_from.clone();
        let plugin_id = self.panel.plugin_id.clone();
        let dispatched_action = action.clone();
        let rearm_poll_generation = refresh_query.is_some().then(|| {
            self.pause_runtime_poll();
            self.runtime_poll_generation
        });
        let runtime = self.runtime.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move {
                        let result = runtime.run_action(&action, serde_json::Value::Null);
                        let refreshed = if result.is_ok() {
                            refresh_query.map(|query| {
                                let payload =
                                    action_refresh_payload(&result, refresh_value_from.as_deref());
                                let result =
                                    payload.map(Ok).unwrap_or_else(|| runtime.query(&query));
                                (query, result)
                            })
                        } else {
                            None
                        };
                        (result, refreshed)
                    })
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    let (result, refreshed) = result;
                    if let Some(RowControl::Action { pending, error, .. }) =
                        this.rows.get_mut(row_index).map(|row| &mut row.control)
                    {
                        *pending = false;
                        *error = result.err();
                    }
                    if let Some((query, result)) = refreshed {
                        apply_runtime_query(&mut this.rows, &query, result);
                    }
                    if let Some(RowControl::Action { active, error, .. }) =
                        this.rows.get(row_index).map(|row| &row.control)
                    {
                        qol_runtime::probe!(
                            "SETTINGS_ACTION_STATE",
                            "plugin={} action={} active={} outcome={}",
                            plugin_id,
                            dispatched_action,
                            active,
                            if error.is_some() { "error" } else { "applied" }
                        );
                    }
                    if rearm_poll_generation == Some(this.runtime_poll_generation) {
                        this.start_runtime_poll(Some(this.runtime.poll_interval), cx);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn dispatch_list_action(&mut self, cx: &mut Context<Self>) {
        let row_index = self.selected;
        let Some(RowControl::List {
            actions,
            items,
            list,
            ..
        }) = self.rows.get(row_index).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) = items.get(list.selected) else {
            return;
        };
        let Some(action) = primary_list_item_action(actions, item) else {
            return;
        };
        let item_id = item.id.clone();
        self.dispatch_resolved_list_action(row_index, &item_id, action, cx);
    }

    fn select_list_item(&mut self, row_index: usize, item_index: usize) {
        let Some(RowControl::List { items, list, .. }) =
            self.rows.get_mut(row_index).map(|row| &mut row.control)
        else {
            return;
        };
        if item_index >= items.len() {
            return;
        }
        self.selected = row_index;
        list.selected = item_index;
        list.sync(items.len());
        self.active_control = Some(ActiveControl::List);
        self.sync_scroll();
    }

    fn open_list_actions(&mut self) {
        let row_index = self.selected;
        let Some(RowControl::List {
            actions,
            items,
            list,
            ..
        }) = self.rows.get(row_index).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) = items.get(list.selected).filter(|item| !item.pending) else {
            return;
        };
        let resolved = list_item_actions(actions, item);
        if resolved.len() < 2 {
            return;
        }
        self.active_control = Some(ActiveControl::ListActions(ListActionMenu {
            row: row_index,
            item_id: item.id.clone(),
            dropdown: Dropdown::open(resolved.len(), 0),
            actions: resolved,
        }));
    }

    fn dispatch_list_menu_action(&mut self, selected: usize, cx: &mut Context<Self>) {
        let Some(ActiveControl::ListActions(menu)) = self.active_control.take() else {
            return;
        };
        let row_index = menu.row;
        let item_id = menu.item_id;
        let Some(action) = menu.actions.into_iter().nth(selected) else {
            self.active_control = Some(ActiveControl::List);
            return;
        };
        self.active_control = Some(ActiveControl::List);
        self.dispatch_resolved_list_action(row_index, &item_id, action, cx);
    }

    fn dispatch_resolved_list_action(
        &mut self,
        row_index: usize,
        item_id: &str,
        action: ResolvedRowAction,
        cx: &mut Context<Self>,
    ) {
        let Some(RowControl::List { actions, items, .. }) =
            self.rows.get_mut(row_index).map(|row| &mut row.control)
        else {
            return;
        };
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        if !list_item_actions(actions, item).contains(&action) {
            return;
        }
        let Some(action) = begin_list_item_action(item, action) else {
            return;
        };
        let item_id = item.id.clone();
        let runtime = self.runtime.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(
                        async move { runtime.run_action(&action.action, action.input) },
                    )
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if let Err(error) = result {
                        this.set_list_action_error(row_index, &item_id, error);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_list_action_error(&mut self, row_index: usize, item_id: &str, error: String) {
        let Some(RowControl::List { items, .. }) =
            self.rows.get_mut(row_index).map(|row| &mut row.control)
        else {
            return;
        };
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        item.pending = false;
        item.error = Some(error);
    }

    fn begin_number_entry(&mut self, key_char: Option<&str>) -> bool {
        let Some(seed) = key_char.filter(|ch| is_number_seed(ch)) else {
            return false;
        };
        let Some(row) = self.rows.get(self.selected) else {
            return false;
        };
        if !matches!(row.control, RowControl::Number { .. }) {
            return false;
        }
        self.active_control = Some(ActiveControl::Edit(seed.to_string()));
        true
    }

    fn begin_edit(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let edit = match &row.control {
            RowControl::Text(value) => value.clone(),
            RowControl::TextList(values) => values.join(", "),
            RowControl::Number { value, .. } => format_number(*value),
            RowControl::Toggle(_)
            | RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Color(_)
            | RowControl::Action { .. }
            | RowControl::Status { .. }
            | RowControl::List { .. } => return,
        };
        self.active_control = Some(ActiveControl::Edit(edit));
    }

    fn commit_edit(&mut self) {
        let Some(ActiveControl::Edit(edit)) = self.active_control.take() else {
            return;
        };
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Text(value) => *value = edit,
            RowControl::TextList(values) => {
                *values = edit
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .filter(|part| !part.is_empty())
                    .collect();
            }
            RowControl::Number {
                value, min, max, ..
            } => {
                let Some(parsed) = parsed_number(&edit, *min, *max) else {
                    return;
                };
                *value = parsed;
            }
            RowControl::Toggle(_)
            | RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Color(_)
            | RowControl::Action { .. }
            | RowControl::Status { .. }
            | RowControl::List { .. } => return,
        }
        self.persist();
    }

    fn persist(&mut self) {
        self.values = merged_config(&self.values, &self.rows);
        save_values(&self.panel.plugin_id, &self.path, &self.values);
    }

    fn sync_scroll(&mut self) {
        self.scroll_offset = scroll_offset_for(
            &self.rows,
            &self.current_visible_rows(),
            self.selected,
            self.scroll_offset,
            self.body_max,
            self.sections.len() <= 1,
        );
    }

    fn display_value(&self, index: usize) -> String {
        if index == self.selected {
            match &self.active_control {
                Some(ActiveControl::Edit(edit)) => return format!("{edit}_"),
                Some(ActiveControl::Wheel(wheel)) => return wheel.value.clone(),
                Some(ActiveControl::Dropdown(_))
                | Some(ActiveControl::List)
                | Some(ActiveControl::ListActions(_))
                | None => {}
            }
        }
        match &self.rows[index].control {
            RowControl::Toggle(value) => binary_state_label(*value).into(),
            RowControl::Select { options, index, .. } => options
                .get(*index)
                .map(|option| option.label.clone())
                .unwrap_or_default(),
            RowControl::MultiSelect {
                options, selected, ..
            } => {
                let chosen: Vec<&str> = options
                    .iter()
                    .zip(selected)
                    .filter(|(_, on)| **on)
                    .map(|(option, _)| option.label.as_str())
                    .collect();
                if chosen.is_empty() {
                    "none".into()
                } else {
                    chosen.join(", ")
                }
            }
            RowControl::Number { value, .. } => format_number(*value),
            RowControl::Text(value) => value.clone(),
            RowControl::TextList(values) => values.join(", "),
            RowControl::Color(value) => value.clone(),
            RowControl::Action {
                active_query,
                state_labels,
                active,
                pending,
                error,
                ..
            } => action_value_label(
                *active,
                *pending,
                error.is_some(),
                active_query.is_some(),
                state_labels,
            ),
            RowControl::Status { label, error, .. } => {
                if error.is_some() {
                    "unavailable".into()
                } else {
                    label.clone().unwrap_or_else(|| "loading...".into())
                }
            }
            RowControl::List { items, error, .. } => {
                if error.is_some() {
                    "unavailable".into()
                } else {
                    format!("{} found", items.len())
                }
            }
        }
    }

    fn value_color(&self, index: usize) -> u32 {
        if index == self.selected && matches!(self.active_control, Some(ActiveControl::Edit(_))) {
            return self.palette.label_text;
        }
        match &self.rows[index].control {
            RowControl::Toggle(value) => binary_state_color(self.palette, *value),
            RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Number { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_) => self.palette.label_text,
            RowControl::Action { error: Some(_), .. }
            | RowControl::Status { error: Some(_), .. }
            | RowControl::List { error: Some(_), .. } => self.palette.state_off,
            RowControl::Action { state_labels, .. } if !state_labels.is_empty() => {
                self.palette.label_text
            }
            RowControl::Action {
                active_query: None, ..
            } => self.palette.state_on,
            RowControl::Action { active, .. } => binary_state_color(self.palette, *active),
            RowControl::Status { tone, .. } => status_tone_color(self.palette, *tone),
            RowControl::List { .. } => self.palette.label_text,
        }
    }

    fn dropdown_style(&self) -> DropdownStyle {
        DropdownStyle {
            bg: self.palette.dropdown_bg,
            bg_selected: self.palette.row_bg_selected,
            border: self.palette.row_border_selected,
            text: self.palette.label_text,
            text_selected: self.palette.section_text,
        }
    }

    fn wheel_style(&self) -> WheelStyle {
        WheelStyle {
            bg: self.palette.dropdown_bg,
            border: self.palette.row_border_selected,
            thumb_border: self.palette.section_text,
        }
    }

    fn render_value_cell(&self, index: usize) -> Div {
        let mut cell = div().flex().flex_row().items_center().gap_2();
        if let RowControl::Status { tone, .. } = self.rows[index].control {
            return cell.child(StatusIndicator::new(
                ("settings-status", index),
                self.display_value(index),
                rgb(status_tone_color(self.palette, tone)),
            ));
        }
        if self.action_is_busy(index) {
            cell = cell.child(Spinner::new(
                ("settings-action-spinner", index),
                rgb(self.palette.state_on),
            ));
        }
        if let Some(color) = self.swatch_color(index) {
            cell = cell.child(
                div()
                    .w_3()
                    .h_3()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(self.palette.panel_border))
                    .bg(rgb(color)),
            );
        }
        if let Some(accent) = self.option_accent(index) {
            cell = cell.child(div().w_2().h_2().rounded_full().bg(rgb(accent)));
        }
        cell.child(
            div()
                .text_sm()
                .text_color(rgb(self.value_color(index)))
                .child(self.display_value(index)),
        )
    }

    fn action_is_busy(&self, index: usize) -> bool {
        action_shows_spinner(&self.rows[index].control)
    }

    fn swatch_color(&self, index: usize) -> Option<u32> {
        let RowControl::Color(value) = &self.rows[index].control else {
            return None;
        };
        let text = if index == self.selected {
            match &self.active_control {
                Some(ActiveControl::Edit(edit)) => edit,
                Some(ActiveControl::Wheel(wheel)) => return parsed_color(&wheel.value),
                Some(ActiveControl::Dropdown(_))
                | Some(ActiveControl::List)
                | Some(ActiveControl::ListActions(_))
                | None => value,
            }
        } else {
            value
        };
        parsed_color(text)
    }

    fn option_accent(&self, index: usize) -> Option<u32> {
        let RowControl::Select { options, index, .. } = &self.rows[index].control else {
            return None;
        };
        options.get(*index)?.accent
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let row = &self.rows[index];
        let mut container = div().flex().flex_col().gap_1();
        if self.sections.len() <= 1 {
            if let Some(section) = section_label_for(&self.rows, index) {
                container = container.child(
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.section_text))
                        .child(section.to_string()),
                );
            }
        }
        if matches!(row.control, RowControl::List { .. }) {
            return container.child(self.render_list(index, cx));
        }
        let label = match &row.control {
            RowControl::Action {
                active: true,
                active_label: Some(label),
                ..
            } => label.clone(),
            _ => row.label.clone(),
        };
        let mut line = div()
            .id(("settings-row", index))
            .flex()
            .flex_row()
            .justify_between()
            .px_2()
            .py_1()
            .rounded_md()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child(label),
            )
            .child(self.render_value_cell(index));
        let row_bounds = Rc::clone(&self.row_bounds[index]);
        line = line.relative().child(
            canvas(
                move |bounds, _, _| row_bounds.set(Some(bounds)),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        if matches!(
            row.control,
            RowControl::Color(_) | RowControl::Action { .. }
        ) {
            line = line.cursor(CursorStyle::PointingHand).on_click(cx.listener(
                move |this, event: &ClickEvent, window, cx| {
                    if event.standard_click() {
                        this.selected = index;
                        this.activate(window, cx);
                    }
                },
            ));
        }
        if index == self.selected {
            line = line
                .bg(rgb(self.palette.row_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.row_border_selected));
            if let Some(ActiveControl::Dropdown(dropdown)) = &self.active_control {
                match &row.control {
                    RowControl::Select { options, .. } => {
                        let items = dropdown_items(options);
                        line = line.child(dropdown.render_items(&items, self.dropdown_style()));
                    }
                    RowControl::MultiSelect {
                        options, selected, ..
                    } => {
                        let items = options
                            .iter()
                            .zip(selected)
                            .map(|(option, on)| DropdownItem {
                                label: format!(
                                    "{} {}",
                                    if *on { "[x]" } else { "[ ]" },
                                    option.label
                                ),
                                accent: option.accent,
                            })
                            .collect::<Vec<_>>();
                        line = line.child(dropdown.render_items(&items, self.dropdown_style()));
                    }
                    RowControl::Toggle(_)
                    | RowControl::Number { .. }
                    | RowControl::Text(_)
                    | RowControl::TextList(_)
                    | RowControl::Color(_)
                    | RowControl::Action { .. }
                    | RowControl::Status { .. }
                    | RowControl::List { .. } => {}
                }
            }
        }
        container = container.child(line);
        let error = match &row.control {
            RowControl::Action {
                error: Some(error), ..
            }
            | RowControl::Status {
                error: Some(error), ..
            } => Some(error),
            _ => None,
        };
        if let Some(error) = error {
            container = container.child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(rgb(self.palette.state_off))
                    .child(error.clone()),
            );
        }
        container
    }

    fn render_section_menu_item(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let section = &self.sections[index];
        let selected = index == self.selected_section;
        let mut item = div()
            .id(("settings-section", index))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if selected {
                self.palette.row_border_selected
            } else {
                self.palette.panel_border
            }))
            .bg(rgb(if selected {
                self.palette.row_bg_selected
            } else {
                self.palette.window_bg
            }))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(self.palette.section_text))
                            .child(section.label.clone()),
                    )
                    .when_some(section.description.clone(), |content, description| {
                        content.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(self.palette.label_text))
                                .child(description),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child("›"),
            )
            .cursor(CursorStyle::PointingHand);
        item = item.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            this.selected_section = index;
            this.open_selected_section();
            cx.notify();
        }));
        item
    }

    fn detail_heading(&self) -> Option<String> {
        if self.sections.len() <= 1 {
            return None;
        }
        self.active_section
            .and_then(|index| self.sections.get(index))
            .map(|section| format!("‹ {}", section.label))
    }

    fn render_list(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let row = &self.rows[index];
        let RowControl::List {
            active_label,
            active: runtime_active,
            empty_message,
            items,
            list,
            error,
            ..
        } = &row.control
        else {
            return div();
        };
        let list_active = index == self.selected
            && matches!(
                self.active_control,
                Some(ActiveControl::List) | Some(ActiveControl::ListActions(_))
            );
        let mut header_status = div().flex().items_center().gap_2();
        if *runtime_active {
            header_status = header_status.child(
                StatusIndicator::new(
                    ("settings-list-activity", index),
                    active_label.as_deref().unwrap_or("Live").to_string(),
                    rgb(self.palette.state_on),
                )
                .pulse(),
            );
        }
        header_status = header_status.child(
            div()
                .text_color(rgb(self.value_color(index)))
                .child(self.display_value(index)),
        );
        let mut container = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .h(px(super::PANEL_LIST_HEIGHT))
            .overflow_hidden()
            .px_2()
            .py_1()
            .rounded_md();
        if index == self.selected {
            container = container
                .bg(rgb(self.palette.row_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.row_border_selected));
        }
        container = container.child(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .h(px(super::PANEL_LIST_HEADER_HEIGHT))
                .justify_between()
                .text_sm()
                .child(
                    div()
                        .text_color(rgb(self.palette.label_text))
                        .child(row.label.clone()),
                )
                .child(header_status),
        );
        if let Some(error) = error {
            return container.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.state_off))
                    .child(error.clone()),
            );
        }
        if items.is_empty() {
            return container.child(
                div()
                    .py_2()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child(empty_message.clone()),
            );
        }
        for item_index in list.visible_range(items.len()) {
            container = container.child(self.render_list_item(index, item_index, list_active, cx));
        }
        container
    }

    fn render_list_item(
        &self,
        index: usize,
        item_index: usize,
        list_active: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let RowControl::List {
            actions,
            items,
            list,
            ..
        } = &self.rows[index].control
        else {
            return div().id((
                "settings-list-item-empty",
                ((index as u64) << 32) | item_index as u64,
            ));
        };
        let item = &items[item_index];
        let selected = list_active && item_index == list.selected;
        let action = primary_list_item_action(actions, item);
        let title = self.render_list_item_title(index, item_index, selected, item, action, cx);
        let accent = item
            .accent
            .map(|tone| rgb(status_tone_color(self.palette, tone)))
            .unwrap_or_else(|| rgba(self.palette.transparent_rgba));
        let mut item_row = div()
            .id((
                "settings-list-item",
                ((index as u64) << 32) | item_index as u64,
            ))
            .flex()
            .flex_col()
            .flex_none()
            .h(px(super::PANEL_LIST_ITEM_HEIGHT))
            .overflow_hidden()
            .border_l_2()
            .border_color(accent)
            .px_2()
            .py_1()
            .rounded_sm()
            .cursor(CursorStyle::PointingHand)
            .child(title);
        item_row = item_row.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            this.select_list_item(index, item_index);
            cx.notify();
        }));
        if selected {
            item_row = item_row.bg(rgb(self.palette.dropdown_bg));
        }
        let Some(detail) = item.error.as_ref().or(item.subtitle.as_ref()) else {
            return item_row;
        };
        item_row.child(
            div()
                .text_xs()
                .text_color(rgb(if item.error.is_some() {
                    self.palette.state_off
                } else {
                    self.palette.label_text
                }))
                .child(detail.clone()),
        )
    }

    fn render_list_item_title(
        &self,
        index: usize,
        item_index: usize,
        selected: bool,
        item: &super::rows::ListItem,
        action: Option<qol_config::contract::ResolvedRowAction>,
        cx: &mut Context<Self>,
    ) -> Div {
        let badge = item.badge.as_ref().map(|label| {
            StatusIndicator::new(
                (
                    "settings-list-item-status",
                    ((index as u64) << 32) | item_index as u64,
                ),
                label.clone(),
                rgb(status_tone_color(self.palette, item.effective_badge_tone())),
            )
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_sm()
                    .text_color(rgb(if selected {
                        self.palette.section_text
                    } else {
                        self.palette.label_text
                    }))
                    .child(item.label.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .children(badge)
                    .child(self.render_list_action(index, item_index, selected, item, action, cx)),
            )
    }

    fn render_list_action(
        &self,
        index: usize,
        item_index: usize,
        selected: bool,
        item: &super::rows::ListItem,
        action: Option<qol_config::contract::ResolvedRowAction>,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut cell = div().flex().flex_none().flex_row().items_center().gap_2();
        if item.pending {
            cell = cell.child(Spinner::new(
                (
                    "settings-list-action-spinner",
                    ((index as u64) << 32) | item_index as u64,
                ),
                rgb(self.palette.state_on),
            ));
        }
        let Some(action) = action else {
            return cell;
        };
        let action_count = match &self.rows[index].control {
            RowControl::List { actions, .. } => list_item_actions(actions, item).len(),
            _ => 0,
        };
        let label = list_action_affordance(&action.label, action_count);
        let mut affordance = div()
            .id((
                "settings-list-action",
                ((index as u64) << 32) | item_index as u64,
            ))
            .text_xs()
            .text_color(rgb(if selected {
                self.palette.state_on
            } else {
                self.palette.label_text
            }))
            .child(label);
        if !item.pending {
            affordance = affordance
                .cursor(CursorStyle::PointingHand)
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if !event.standard_click() {
                        return;
                    }
                    cx.stop_propagation();
                    this.select_list_item(index, item_index);
                    if action_count > 1 {
                        this.open_list_actions();
                        cx.notify();
                        return;
                    }
                    this.dispatch_list_action(cx);
                    cx.notify();
                }));
        }
        if let Some(ActiveControl::ListActions(menu)) = &self.active_control {
            if menu.row == index && menu.item_id == item.id {
                let labels = menu
                    .actions
                    .iter()
                    .map(|action| action.label.clone())
                    .collect::<Vec<_>>();
                let view = cx.weak_entity();
                affordance = affordance.child(menu.dropdown.render_clickable(
                    format!("settings-list-actions-{index}-{item_index}"),
                    &labels,
                    self.dropdown_style(),
                    move |selected, event, _, cx| {
                        if !event.standard_click() {
                            return;
                        }
                        cx.stop_propagation();
                        let _ = view.update(cx, |this, cx| {
                            this.dispatch_list_menu_action(selected, cx);
                            cx.notify();
                        });
                    },
                ));
            }
        }
        cell.child(affordance)
    }
}

impl Focusable for SettingsPanelView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<AnyElement> = if self.section_menu_is_open() {
            (0..self.sections.len())
                .map(|index| self.render_section_menu_item(index, cx).into_any_element())
                .collect()
        } else {
            visible_row_window(
                &self.rows,
                &self.current_visible_rows(),
                self.scroll_offset,
                self.body_max,
                self.sections.len() <= 1,
            )
            .into_iter()
            .map(|index| self.render_row(index, cx).into_any_element())
            .collect()
        };
        let detail_heading = self.detail_heading();
        div()
            .id("settings-panel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(rgb(self.palette.panel_border))
            .bg(rgb(self.palette.window_bg))
            .child(
                div()
                    .w_full()
                    .pt_4()
                    .px_4()
                    .pb_1()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .panel_drag_area()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(self.panel.heading.clone())
                            .when_some(detail_heading, |header, detail| {
                                header.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(self.palette.section_text))
                                        .child(detail),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .pb_4()
                    .children(items),
            )
    }
}

fn is_number_seed(ch: &str) -> bool {
    !ch.is_empty()
        && ch
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
}

fn dropdown_items(options: &[SelectOption]) -> Vec<DropdownItem> {
    options
        .iter()
        .map(|option| DropdownItem {
            label: option.label.clone(),
            accent: option.accent,
        })
        .collect()
}

fn parsed_number(edit: &str, min: Option<f64>, max: Option<f64>) -> Option<f64> {
    let mut value = edit.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
    if let Some(min) = min {
        value = value.max(min);
    }
    if let Some(max) = max {
        value = value.min(max);
    }
    Some(value)
}

fn parsed_color(text: &str) -> Option<u32> {
    let hex = text.trim();
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

fn binary_state_label(active: bool) -> &'static str {
    if active {
        "[on]"
    } else {
        "[off]"
    }
}

fn action_value_label(
    active: bool,
    pending: bool,
    failed: bool,
    has_runtime_state: bool,
    state_labels: &std::collections::BTreeMap<String, String>,
) -> String {
    if pending {
        return "working...".into();
    }
    if failed {
        return "failed".into();
    }
    if !has_runtime_state {
        return "[run]".into();
    }
    state_labels
        .get(if active { "true" } else { "false" })
        .cloned()
        .unwrap_or_else(|| binary_state_label(active).into())
}

fn binary_state_color(palette: SettingsPanelPalette, active: bool) -> u32 {
    if active {
        palette.state_on
    } else {
        palette.state_off
    }
}

fn status_tone_color(palette: SettingsPanelPalette, tone: StatusTone) -> u32 {
    match tone {
        StatusTone::Accent => palette.status_accent,
        StatusTone::Success => palette.status_success,
        StatusTone::Danger => palette.status_danger,
        StatusTone::Warning => palette.status_warning,
        StatusTone::Muted => palette.status_muted,
    }
}

fn list_action_affordance(primary: &str, action_count: usize) -> String {
    if action_count > 1 {
        return format!("[{primary} +{}]", action_count - 1);
    }
    format!("[{primary}]")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Up,
    Down,
    Toggle,
    Left,
    Right,
    Activate,
    CommitEdit,
    Backspace,
    Insert(String),
    Close,
    CancelEdit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListIntent {
    Up,
    Down,
    Activate,
    Actions,
    Close,
}

fn list_intent(key: &str) -> Option<ListIntent> {
    match key {
        "up" => Some(ListIntent::Up),
        "down" => Some(ListIntent::Down),
        "enter" | "return" => Some(ListIntent::Activate),
        "space" | "right" => Some(ListIntent::Actions),
        "escape" | "left" => Some(ListIntent::Close),
        _ => None,
    }
}

fn action_refresh_payload(
    result: &Result<Option<serde_json::Value>, String>,
    active_value_from: Option<&str>,
) -> Option<serde_json::Value> {
    result
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .filter(|value| query_flag_value(value, active_value_from).is_some())
        .cloned()
}

fn action_shows_spinner(control: &RowControl) -> bool {
    match control {
        RowControl::Action {
            pending: true,
            error: None,
            ..
        } => true,
        RowControl::Action {
            active: true,
            error: None,
            variant,
            ..
        } => variant.as_deref() != Some("toggle"),
        _ => false,
    }
}

fn left_navigates_back(control: &RowControl, has_section_menu: bool) -> bool {
    has_section_menu
        && !matches!(
            control,
            RowControl::Number { .. } | RowControl::Select { .. }
        )
}

fn intent(key: &str, key_char: Option<&str>, editing: bool) -> Option<Intent> {
    if editing {
        return match key {
            "enter" | "return" => Some(Intent::CommitEdit),
            "escape" => Some(Intent::CancelEdit),
            "backspace" => Some(Intent::Backspace),
            _ => key_char.map(|ch| Intent::Insert(ch.to_string())),
        };
    }
    match key {
        "up" => Some(Intent::Up),
        "down" => Some(Intent::Down),
        "space" => Some(Intent::Toggle),
        "left" => Some(Intent::Left),
        "right" => Some(Intent::Right),
        "enter" | "return" => Some(Intent::Activate),
        "escape" => Some(Intent::Close),
        _ => None,
    }
}

pub(super) fn row_height(row: &Row) -> f32 {
    let header = if row.section_label.is_some() {
        super::PANEL_SECTION_HEADER_HEIGHT
    } else {
        0.0
    };
    let body = if matches!(row.control, RowControl::List { .. }) {
        super::PANEL_LIST_HEIGHT
    } else {
        super::PANEL_ROW_HEIGHT
    };
    body + header
}

fn visible_row_height(rows: &[Row], index: usize, show_section_headers: bool) -> f32 {
    let header = if show_section_headers && section_label_for(rows, index).is_some() {
        super::PANEL_SECTION_HEADER_HEIGHT
    } else {
        0.0
    };
    let body = if matches!(rows[index].control, RowControl::List { .. }) {
        super::PANEL_LIST_HEIGHT
    } else {
        super::PANEL_ROW_HEIGHT
    };
    body + header
}

fn visible_row_window(
    rows: &[Row],
    visible: &[usize],
    offset: usize,
    body_max: f32,
    show_section_headers: bool,
) -> Vec<usize> {
    let start = visible
        .iter()
        .position(|index| *index >= offset)
        .unwrap_or(visible.len());
    let mut used = 0.0;
    let mut window = Vec::new();
    for index in visible.iter().copied().skip(start) {
        used += visible_row_height(rows, index, show_section_headers);
        if used > body_max && !window.is_empty() {
            break;
        }
        window.push(index);
    }
    window
}

fn scroll_offset_for(
    rows: &[Row],
    visible: &[usize],
    selected: usize,
    offset: usize,
    body_max: f32,
    show_section_headers: bool,
) -> usize {
    let Some(selected_position) = visible.iter().position(|index| *index == selected) else {
        return visible.first().copied().unwrap_or(0);
    };
    let mut offset_position = visible
        .iter()
        .position(|index| *index >= offset)
        .unwrap_or(selected_position);
    if selected_position < offset_position {
        return selected;
    }
    while !visible_row_window(
        rows,
        visible,
        visible[offset_position],
        body_max,
        show_section_headers,
    )
    .contains(&selected)
    {
        offset_position += 1;
    }
    visible[offset_position]
}

fn adjacent_visible_row(visible: &[usize], selected: usize, direction: isize) -> usize {
    let Some(position) = visible.iter().position(|index| *index == selected) else {
        return visible.first().copied().unwrap_or(0);
    };
    let next = if direction < 0 {
        position.saturating_sub(1)
    } else {
        (position + 1).min(visible.len() - 1)
    };
    visible[next]
}

fn initial_active_section(section_count: usize) -> Option<usize> {
    if section_count == 1 {
        return Some(0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        action_refresh_payload, action_shows_spinner, action_value_label, adjacent_visible_row,
        binary_state_label, initial_active_section, intent, is_number_seed, left_navigates_back,
        list_action_affordance, list_intent, parsed_color, parsed_number, scroll_offset_for,
        visible_row_window, Intent, ListIntent, Row, RowControl,
    };
    use crate::scroll_list::ScrollList;
    use crate::settings_panel::rows::{rows_from_resolved, visible_row_indices, SelectOption};

    fn rows(headers: &[bool]) -> Vec<Row> {
        headers
            .iter()
            .map(|header| Row {
                id: "field".into(),
                section_id: header.then(|| "section".to_string()),
                section_label: header.then(|| "Section".to_string()),
                label: "Label".into(),
                config_key: "key".into(),
                visibility: None,
                control: RowControl::Toggle(false),
            })
            .collect()
    }

    fn list_row() -> Row {
        Row {
            id: "items".into(),
            section_id: None,
            section_label: None,
            label: "Items".into(),
            config_key: "items".into(),
            visibility: None,
            control: RowControl::List {
                query: "items".into(),
                active_query: None,
                active_value_from: None,
                active_label: None,
                active: false,
                row_label: "{name}".into(),
                row_subtitle: Some("{detail}".into()),
                actions: Box::new(super::super::rows::ListActions {
                    primary: None,
                    additional: Vec::new(),
                }),
                empty_message: "No items".into(),
                items: Vec::new(),
                list: ScrollList::new(super::super::rows::LIST_MAX_VISIBLE),
                error: None,
            },
        }
    }

    #[test]
    fn visible_range_windows_rows_by_height_budget() {
        let rows = rows(&[true, false, false, false]);
        let two_plain_rows = 2.0 * super::super::PANEL_ROW_HEIGHT;
        let cases = [
            (0, two_plain_rows, 0..1),
            (1, two_plain_rows, 1..3),
            (3, two_plain_rows, 3..4),
            (0, 1000.0, 0..4),
            (0, 1.0, 0..1),
        ];
        let visible = (0..rows.len()).collect::<Vec<_>>();
        for (offset, budget, expected) in cases {
            assert_eq!(
                visible_row_window(&rows, &visible, offset, budget, true),
                expected.collect::<Vec<_>>(),
                "offset {offset} budget {budget}"
            );
        }
    }

    #[test]
    fn visible_range_reserves_the_full_runtime_list_height() {
        let mut panel_rows = rows(&[true]);
        panel_rows.push(list_row());
        panel_rows.extend(rows(&[true, false, false, false]));
        let visible = (0..panel_rows.len()).collect::<Vec<_>>();

        assert_eq!(
            visible_row_window(&panel_rows, &visible, 0, 480.0, true),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn scroll_offset_keeps_selection_visible_in_both_directions() {
        let rows = rows(&[false, false, false, false]);
        let two_rows = 2.0 * super::super::PANEL_ROW_HEIGHT;
        let cases = [(0, 0, 0), (1, 0, 0), (2, 0, 1), (3, 1, 2), (0, 2, 0)];
        let visible = (0..rows.len()).collect::<Vec<_>>();
        for (selected, offset, expected) in cases {
            assert_eq!(
                scroll_offset_for(&rows, &visible, selected, offset, two_rows, true),
                expected,
                "selected {selected} offset {offset}"
            );
        }
    }

    #[test]
    fn keyboard_navigation_skips_conditional_rows() {
        const SPEC: &str = r#"
schema_version = 1

[field.enabled]
type = "boolean"
default = false

[field.detail]
type = "number"
default = 4

[field.detail.show_when]
field = "enabled"
equals = true

[field.always]
type = "string"
default = "visible"
"#;
        let spec = qol_config::contract::parse_spec_str(SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved);
        let visible = visible_row_indices(&rows);

        assert_eq!(adjacent_visible_row(&visible, 0, 1), 2);
        assert_eq!(adjacent_visible_row(&visible, 2, -1), 0);
    }

    #[test]
    fn is_number_seed_accepts_numeric_starters_only() {
        let cases = [
            ("5", true),
            ("-", true),
            (".", true),
            ("a", false),
            (" ", false),
            ("", false),
        ];
        for (ch, expected) in cases {
            assert_eq!(is_number_seed(ch), expected, "char: {ch:?}");
        }
    }

    #[test]
    fn list_intent_activates_enter_instead_of_closing_navigation() {
        let cases = [
            ("up", Some(ListIntent::Up)),
            ("down", Some(ListIntent::Down)),
            ("enter", Some(ListIntent::Activate)),
            ("return", Some(ListIntent::Activate)),
            ("space", Some(ListIntent::Actions)),
            ("right", Some(ListIntent::Actions)),
            ("escape", Some(ListIntent::Close)),
            ("left", Some(ListIntent::Close)),
            ("tab", None),
        ];
        for (key, expected) in cases {
            assert_eq!(list_intent(key), expected, "key: {key}");
        }
    }

    #[test]
    fn binary_runtime_and_config_states_share_on_off_labels() {
        assert_eq!(binary_state_label(true), "[on]");
        assert_eq!(binary_state_label(false), "[off]");
    }

    #[test]
    fn action_values_distinguish_commands_from_semantic_runtime_state() {
        let labels = std::collections::BTreeMap::from([
            ("false".into(), "Light".into()),
            ("true".into(), "Dark".into()),
        ]);
        let cases = [
            (false, false, false, false, "[run]"),
            (false, true, false, false, "working..."),
            (false, false, true, false, "failed"),
            (false, false, false, true, "Light"),
            (true, false, false, true, "Dark"),
        ];
        for (active, pending, failed, runtime, expected) in cases {
            assert_eq!(
                action_value_label(active, pending, failed, runtime, &labels),
                expected
            );
        }
    }

    #[test]
    fn action_refresh_uses_only_payloads_that_answer_the_active_query() {
        let cases = [
            (
                Ok(Some(serde_json::json!({"dark": true}))),
                Some("dark"),
                Some(serde_json::json!({"dark": true})),
            ),
            (
                Ok(Some(serde_json::json!({"dark": false}))),
                Some("dark"),
                Some(serde_json::json!({"dark": false})),
            ),
            (
                Ok(Some(serde_json::json!({"scheme": "dark"}))),
                Some("dark"),
                None,
            ),
            (
                Ok(Some(serde_json::json!({"dark": "yes"}))),
                Some("dark"),
                None,
            ),
            (
                Ok(Some(serde_json::json!(true))),
                None,
                Some(serde_json::json!(true)),
            ),
            (Ok(None), Some("dark"), None),
            (Err("failed".into()), Some("dark"), None),
        ];
        for (result, path, expected) in cases {
            assert_eq!(action_refresh_payload(&result, path), expected);
        }
    }

    #[test]
    fn multiple_contract_sections_open_in_the_shared_section_menu() {
        let cases = [(0, None), (1, Some(0)), (2, None), (8, None)];
        for (count, expected) in cases {
            assert_eq!(initial_active_section(count), expected, "count: {count}");
        }
    }

    #[test]
    fn left_returns_to_sections_unless_the_row_adjusts_horizontally() {
        let toggle = RowControl::Toggle(false);
        let number = RowControl::Number {
            value: 4.0,
            min: Some(1.0),
            max: Some(8.0),
            step: 1.0,
        };
        let select = RowControl::Select {
            options: vec![
                SelectOption::plain("one", "One"),
                SelectOption::plain("two", "Two"),
            ],
            index: 0,
            dynamic: None,
        };
        let cases = [
            (&toggle, false, false),
            (&toggle, true, true),
            (&number, true, false),
            (&select, true, false),
        ];
        for (control, has_sections, expected) in cases {
            assert_eq!(
                left_navigates_back(control, has_sections),
                expected,
                "control: {control:?}, sections: {has_sections}"
            );
        }
    }

    #[test]
    fn toggle_actions_show_state_without_a_permanent_spinner() {
        let mut control = RowControl::Action {
            action: "enable_adapter".into(),
            active_action: Some("disable_adapter".into()),
            active_label: Some("Bluetooth".into()),
            active_query: Some("adapter_status".into()),
            active_value_from: Some("powered".into()),
            state_labels: std::collections::BTreeMap::new(),
            variant: Some("toggle".into()),
            active: true,
            pending: false,
            error: None,
        };
        assert!(!action_shows_spinner(&control));

        let RowControl::Action { pending, .. } = &mut control else {
            unreachable!();
        };
        *pending = true;
        assert!(action_shows_spinner(&control));
    }

    #[test]
    fn list_action_affordance_exposes_additional_action_count() {
        let cases = [
            ("Connect", 1, "[Connect]"),
            ("Disconnect", 2, "[Disconnect +1]"),
            ("Pair", 6, "[Pair +5]"),
        ];
        for (primary, count, expected) in cases {
            assert_eq!(list_action_affordance(primary, count), expected);
        }
    }

    #[test]
    fn parsed_number_parses_clamps_and_rejects() {
        let cases = [
            ("18", None, None, Some(18.0)),
            (" 23.5 ", None, None, Some(23.5)),
            ("-4", Some(0.0), Some(51.0), Some(0.0)),
            ("99", Some(0.0), Some(51.0), Some(51.0)),
            ("abc", None, None, None),
            ("", None, None, None),
            ("inf", None, None, None),
        ];
        for (edit, min, max, expected) in cases {
            assert_eq!(parsed_number(edit, min, max), expected, "edit: {edit:?}");
        }
    }

    #[test]
    fn parsed_color_accepts_six_digit_hex_with_optional_hash() {
        let cases = [
            ("#202322", Some(0x202322)),
            ("202322", Some(0x202322)),
            (" #ffffff ", Some(0xffffff)),
            ("AABBCC", Some(0xaabbcc)),
            ("#fff", None),
            ("#2023221", None),
            ("20232g", None),
            ("", None),
        ];
        for (text, expected) in cases {
            assert_eq!(parsed_color(text), expected, "text: {text:?}");
        }
    }

    #[test]
    fn intent_maps_navigation_editing_and_close() {
        let cases = [
            ("up", None, false, Some(Intent::Up)),
            ("down", None, false, Some(Intent::Down)),
            ("space", None, false, Some(Intent::Toggle)),
            ("left", None, false, Some(Intent::Left)),
            ("right", None, false, Some(Intent::Right)),
            ("enter", None, false, Some(Intent::Activate)),
            ("return", None, false, Some(Intent::Activate)),
            ("escape", None, false, Some(Intent::Close)),
            ("enter", None, true, Some(Intent::CommitEdit)),
            ("return", None, true, Some(Intent::CommitEdit)),
            ("escape", None, true, Some(Intent::CancelEdit)),
            ("backspace", None, true, Some(Intent::Backspace)),
            ("a", Some("a"), true, Some(Intent::Insert("a".into()))),
            ("a", Some("a"), false, None),
        ];
        for (key, ch, editing, expected) in cases {
            assert_eq!(
                intent(key, ch, editing),
                expected,
                "key {key} editing {editing}"
            );
        }
    }
}
