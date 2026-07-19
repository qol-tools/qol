use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::*;

use super::color_wheel::{ColorWheel, ColorWheelPopup, WheelCallbacks, WheelStyle};
use super::persistence::save_values;
use super::rows::{apply_runtime_query, merged_config, runtime_query_names, Row, RowControl};
use super::{SettingsPanel, SettingsRuntime};
use crate::dropdown::{Dropdown, DropdownStyle};
use crate::spinner::Spinner;
use crate::surface::SurfaceDismisser;
use crate::theme::{settings_panel_runtime, SettingsPanelPalette};

pub(super) struct SettingsPanelView {
    panel: SettingsPanel,
    runtime: SettingsRuntime,
    runtime_queries: Vec<String>,
    rows: Vec<Row>,
    values: serde_json::Value,
    path: PathBuf,
    selected: usize,
    scroll_offset: usize,
    body_max: f32,
    active_control: Option<ActiveControl>,
    row_bounds: Vec<Rc<Cell<Option<Bounds<Pixels>>>>>,
    wheel_generation: u64,
    dismisser: SurfaceDismisser,
    palette: SettingsPanelPalette,
    focus_handle: FocusHandle,
}

pub(super) struct SettingsPanelState {
    pub(super) rows: Vec<Row>,
    pub(super) values: serde_json::Value,
    pub(super) path: PathBuf,
    pub(super) body_max: f32,
    pub(super) runtime: SettingsRuntime,
}

enum ActiveControl {
    Edit(String),
    Dropdown(Dropdown),
    List,
    Wheel(WheelControl),
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
        let view = Self {
            panel,
            runtime: state.runtime,
            runtime_queries,
            rows: state.rows,
            values: state.values,
            path: state.path,
            selected: 0,
            scroll_offset: 0,
            body_max: state.body_max,
            active_control: None,
            row_bounds,
            wheel_generation: 0,
            dismisser,
            palette: settings_panel_runtime(),
            focus_handle: cx.focus_handle(),
        };
        view.spawn_runtime_poll(cx);
        view
    }

    fn spawn_runtime_poll(&self, cx: &mut Context<Self>) {
        if self.runtime_queries.is_empty() {
            return;
        }
        let runtime = self.runtime.clone();
        let queries = self.runtime_queries.clone();
        let interval = runtime.poll_interval;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                loop {
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
                    if this
                        .update(&mut async_cx, |this, cx| {
                            for (query, result) in results {
                                apply_runtime_query(&mut this.rows, &query, result);
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                    async_cx.background_executor().timer(interval).await;
                }
            }
        })
        .detach();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let key_char = event.keystroke.key_char.as_deref();
        if let Some(ActiveControl::Wheel(wheel)) = &self.active_control {
            let popup = wheel.popup;
            let _ = popup.update(cx, |popup, popup_window, popup_cx| {
                popup.handle_key(key, event.keystroke.modifiers.shift, popup_window, popup_cx);
            });
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
                self.selected = self.selected.saturating_sub(1);
                self.sync_scroll();
            }
            Intent::Down => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
                self.sync_scroll();
            }
            Intent::Toggle => self.toggle(),
            Intent::Left => self.adjust(-1.0),
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
        match key {
            "up" => dropdown.move_up(),
            "down" => dropdown.move_down(),
            "enter" | "return" | "space" => self.pick_dropdown(),
            "escape" => self.active_control = None,
            _ => return,
        }
        cx.notify();
    }

    fn on_list_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get_mut(self.selected) else {
            self.active_control = None;
            return;
        };
        let RowControl::List { items, list, .. } = &mut row.control else {
            self.active_control = None;
            return;
        };
        match key {
            "up" => list.move_up(),
            "down" => list.move_down(items.len()),
            "escape" | "left" | "enter" | "return" => self.active_control = None,
            _ => return,
        }
        list.sync(items.len());
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
        let runtime = self.runtime.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { runtime.run_action(&action) })
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if let Some(RowControl::Action { pending, error, .. }) =
                        this.rows.get_mut(row_index).map(|row| &mut row.control)
                    {
                        *pending = false;
                        *error = result.err();
                    }
                    cx.notify();
                });
            }
        })
        .detach();
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
            | RowControl::List { .. } => return,
        }
        self.persist();
    }

    fn persist(&mut self) {
        self.values = merged_config(&self.values, &self.rows);
        save_values(self.panel.plugin_id, &self.path, &self.values);
    }

    fn sync_scroll(&mut self) {
        self.scroll_offset =
            scroll_offset_for(&self.rows, self.selected, self.scroll_offset, self.body_max);
    }

    fn display_value(&self, index: usize) -> String {
        if index == self.selected {
            match &self.active_control {
                Some(ActiveControl::Edit(edit)) => return format!("{edit}_"),
                Some(ActiveControl::Wheel(wheel)) => return wheel.value.clone(),
                Some(ActiveControl::Dropdown(_)) | Some(ActiveControl::List) | None => {}
            }
        }
        match &self.rows[index].control {
            RowControl::Toggle(value) => binary_state_label(*value).into(),
            RowControl::Select { labels, index, .. } => {
                labels.get(*index).cloned().unwrap_or_default()
            }
            RowControl::MultiSelect {
                labels, selected, ..
            } => {
                let chosen: Vec<&str> = labels
                    .iter()
                    .zip(selected)
                    .filter(|(_, on)| **on)
                    .map(|(label, _)| label.as_str())
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
                active,
                pending,
                error,
                ..
            } => {
                if *pending {
                    "working...".into()
                } else if error.is_some() {
                    "failed".into()
                } else {
                    binary_state_label(*active).into()
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
        match self.rows[index].control {
            RowControl::Toggle(value) => binary_state_color(self.palette, value),
            RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Number { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_) => self.palette.label_text,
            RowControl::Action { error: Some(_), .. } | RowControl::List { error: Some(_), .. } => {
                self.palette.state_off
            }
            RowControl::Action { active, .. } => binary_state_color(self.palette, active),
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
        cell.child(
            div()
                .text_sm()
                .text_color(rgb(self.value_color(index)))
                .child(self.display_value(index)),
        )
    }

    fn action_is_busy(&self, index: usize) -> bool {
        matches!(
            self.rows[index].control,
            RowControl::Action {
                active: true,
                error: None,
                ..
            } | RowControl::Action {
                pending: true,
                error: None,
                ..
            }
        )
    }

    fn swatch_color(&self, index: usize) -> Option<u32> {
        let RowControl::Color(value) = &self.rows[index].control else {
            return None;
        };
        let text = if index == self.selected {
            match &self.active_control {
                Some(ActiveControl::Edit(edit)) => edit,
                Some(ActiveControl::Wheel(wheel)) => return parsed_color(&wheel.value),
                Some(ActiveControl::Dropdown(_)) | Some(ActiveControl::List) | None => value,
            }
        } else {
            value
        };
        parsed_color(text)
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let row = &self.rows[index];
        let mut container = div().flex().flex_col().gap_1();
        if let Some(section) = &row.section_label {
            container = container.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.section_text))
                    .child(section.clone()),
            );
        }
        if matches!(row.control, RowControl::List { .. }) {
            return container.child(self.render_list(index));
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
                    RowControl::Select { labels, .. } => {
                        line = line.child(dropdown.render(labels, self.dropdown_style()));
                    }
                    RowControl::MultiSelect {
                        labels, selected, ..
                    } => {
                        let marked: Vec<String> = labels
                            .iter()
                            .zip(selected)
                            .map(|(label, on)| {
                                format!("{} {label}", if *on { "[x]" } else { "[ ]" })
                            })
                            .collect();
                        line = line.child(dropdown.render(&marked, self.dropdown_style()));
                    }
                    RowControl::Toggle(_)
                    | RowControl::Number { .. }
                    | RowControl::Text(_)
                    | RowControl::TextList(_)
                    | RowControl::Color(_)
                    | RowControl::Action { .. }
                    | RowControl::List { .. } => {}
                }
            }
        }
        container = container.child(line);
        if let RowControl::Action {
            error: Some(error), ..
        } = &row.control
        {
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

    fn render_list(&self, index: usize) -> Div {
        let row = &self.rows[index];
        let RowControl::List {
            empty_message,
            items,
            list,
            error,
            ..
        } = &row.control
        else {
            return div();
        };
        let active =
            index == self.selected && matches!(self.active_control, Some(ActiveControl::List));
        let mut container = div().flex().flex_col().gap_1().px_2().py_1().rounded_md();
        if index == self.selected {
            container = container
                .bg(rgb(self.palette.row_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.row_border_selected));
        }
        container = container.child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_sm()
                .child(
                    div()
                        .text_color(rgb(self.palette.label_text))
                        .child(row.label.clone()),
                )
                .child(
                    div()
                        .text_color(rgb(self.value_color(index)))
                        .child(self.display_value(index)),
                ),
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
            let item = &items[item_index];
            let mut item_row = div().flex().flex_col().px_2().py_1().rounded_sm();
            if active && item_index == list.selected {
                item_row = item_row.bg(rgb(self.palette.dropdown_bg));
            }
            item_row = item_row.child(
                div()
                    .text_sm()
                    .text_color(rgb(if active && item_index == list.selected {
                        self.palette.section_text
                    } else {
                        self.palette.label_text
                    }))
                    .child(item.label.clone()),
            );
            if let Some(subtitle) = &item.subtitle {
                item_row = item_row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.label_text))
                        .child(subtitle.clone()),
                );
            }
            container = container.child(item_row);
        }
        container
    }
}

impl Focusable for SettingsPanelView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<AnyElement> =
            visible_row_range(&self.rows, self.scroll_offset, self.body_max)
                .map(|index| self.render_row(index, cx).into_any_element())
                .collect();
        div()
            .id("settings-panel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_4()
            .rounded_xl()
            .border_1()
            .border_color(rgb(self.palette.panel_border))
            .bg(rgb(self.palette.window_bg))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child(self.panel.heading),
            )
            .children(items)
    }
}

fn is_number_seed(ch: &str) -> bool {
    !ch.is_empty()
        && ch
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
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

fn binary_state_color(palette: SettingsPanelPalette, active: bool) -> u32 {
    if active {
        palette.state_on
    } else {
        palette.state_off
    }
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

fn visible_row_range(rows: &[Row], offset: usize, body_max: f32) -> std::ops::Range<usize> {
    let mut used = 0.0;
    let mut end = offset;
    for row in rows.iter().skip(offset) {
        used += row_height(row);
        if used > body_max && end > offset {
            break;
        }
        end += 1;
    }
    offset..end.min(rows.len())
}

fn scroll_offset_for(rows: &[Row], selected: usize, offset: usize, body_max: f32) -> usize {
    if selected < offset {
        return selected;
    }
    let mut offset = offset;
    while !visible_row_range(rows, offset, body_max).contains(&selected) {
        offset += 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::{
        binary_state_label, intent, is_number_seed, parsed_color, parsed_number, scroll_offset_for,
        visible_row_range, Intent, Row, RowControl,
    };

    fn rows(headers: &[bool]) -> Vec<Row> {
        headers
            .iter()
            .map(|header| Row {
                section_label: header.then(|| "Section".to_string()),
                label: "Label".into(),
                config_key: "key".into(),
                control: RowControl::Toggle(false),
            })
            .collect()
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
        for (offset, budget, expected) in cases {
            assert_eq!(
                visible_row_range(&rows, offset, budget),
                expected,
                "offset {offset} budget {budget}"
            );
        }
    }

    #[test]
    fn scroll_offset_keeps_selection_visible_in_both_directions() {
        let rows = rows(&[false, false, false, false]);
        let two_rows = 2.0 * super::super::PANEL_ROW_HEIGHT;
        let cases = [(0, 0, 0), (1, 0, 0), (2, 0, 1), (3, 1, 2), (0, 2, 0)];
        for (selected, offset, expected) in cases {
            assert_eq!(
                scroll_offset_for(&rows, selected, offset, two_rows),
                expected,
                "selected {selected} offset {offset}"
            );
        }
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
    fn binary_runtime_and_config_states_share_on_off_labels() {
        assert_eq!(binary_state_label(true), "[on]");
        assert_eq!(binary_state_label(false), "[off]");
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
