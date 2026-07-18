use std::path::PathBuf;

use gpui::*;

use super::persistence::save_values;
use super::rows::{merged_config, Row, RowControl};
use super::SettingsPanel;
use crate::dropdown::{Dropdown, DropdownStyle};
use crate::surface::SurfaceDismisser;
use crate::theme::{settings_panel_runtime, SettingsPanelPalette};

pub(super) struct SettingsPanelView {
    panel: SettingsPanel,
    rows: Vec<Row>,
    values: serde_json::Value,
    path: PathBuf,
    selected: usize,
    scroll_offset: usize,
    body_max: f32,
    edit: Option<String>,
    dropdown: Option<Dropdown>,
    dismisser: SurfaceDismisser,
    palette: SettingsPanelPalette,
    focus_handle: FocusHandle,
}

impl SettingsPanelView {
    pub(super) fn new(
        panel: SettingsPanel,
        rows: Vec<Row>,
        values: serde_json::Value,
        path: PathBuf,
        body_max: f32,
        dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            panel,
            rows,
            values,
            path,
            selected: 0,
            scroll_offset: 0,
            body_max,
            edit: None,
            dropdown: None,
            dismisser,
            palette: settings_panel_runtime(),
            focus_handle: cx.focus_handle(),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let key_char = event.keystroke.key_char.as_deref();
        if self.dropdown.is_some() {
            self.on_dropdown_key(key, cx);
            return;
        }
        if self.edit.is_some() && matches!(key, "up" | "down") {
            self.commit_edit();
        }
        let Some(intent) = intent(key, key_char, self.edit.is_some()) else {
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
            Intent::Activate => self.activate(),
            Intent::CommitEdit => self.commit_edit(),
            Intent::Backspace => {
                if let Some(edit) = self.edit.as_mut() {
                    edit.pop();
                }
            }
            Intent::Insert(ch) => {
                if let Some(edit) = self.edit.as_mut() {
                    edit.push_str(&ch);
                }
            }
            Intent::CancelEdit => self.edit = None,
            Intent::Close => {
                self.dismisser.dismiss(cx);
                return;
            }
        }
        cx.notify();
    }

    fn on_dropdown_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(dropdown) = self.dropdown.as_mut() else {
            return;
        };
        match key {
            "up" => dropdown.move_up(),
            "down" => dropdown.move_down(),
            "enter" | "return" | "space" => self.pick_dropdown(),
            "escape" => self.dropdown = None,
            _ => return,
        }
        cx.notify();
    }

    fn pick_dropdown(&mut self) {
        let Some(dropdown) = self.dropdown.as_ref() else {
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
                self.dropdown = None;
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
            | RowControl::Color(_) => self.dropdown = None,
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
            | RowControl::Color(_) => return,
        }
        self.persist();
    }

    fn activate(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        match &row.control {
            RowControl::Toggle(_) => self.toggle(),
            RowControl::Select { options, index, .. } => {
                self.dropdown = Some(Dropdown::open(options.len(), *index));
            }
            RowControl::MultiSelect { options, .. } => {
                self.dropdown = Some(Dropdown::open(options.len(), 0));
            }
            RowControl::Number { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_) => self.begin_edit(),
        }
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
        self.edit = Some(seed.to_string());
        true
    }

    fn begin_edit(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        self.edit = match &row.control {
            RowControl::Text(value) => Some(value.clone()),
            RowControl::TextList(values) => Some(values.join(", ")),
            RowControl::Color(value) => Some(value.clone()),
            RowControl::Number { value, .. } => Some(format_number(*value)),
            RowControl::Toggle(_) | RowControl::Select { .. } | RowControl::MultiSelect { .. } => {
                None
            }
        };
    }

    fn commit_edit(&mut self) {
        let Some(edit) = self.edit.take() else {
            return;
        };
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Text(value) => *value = edit,
            RowControl::Color(value) => {
                let trimmed = edit.trim();
                if parsed_color(trimmed).is_none() {
                    return;
                }
                *value = trimmed.to_string();
            }
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
            RowControl::Toggle(_) | RowControl::Select { .. } | RowControl::MultiSelect { .. } => {
                return
            }
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
            if let Some(edit) = &self.edit {
                return format!("{edit}_");
            }
        }
        match &self.rows[index].control {
            RowControl::Toggle(true) => "[on]".into(),
            RowControl::Toggle(false) => "[off]".into(),
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
        }
    }

    fn value_color(&self, index: usize) -> u32 {
        if index == self.selected && self.edit.is_some() {
            return self.palette.label_text;
        }
        match self.rows[index].control {
            RowControl::Toggle(true) => self.palette.state_on,
            RowControl::Toggle(false) => self.palette.state_off,
            RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Number { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_) => self.palette.label_text,
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

    fn render_value_cell(&self, index: usize) -> Div {
        let mut cell = div().flex().flex_row().items_center().gap_2();
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

    fn swatch_color(&self, index: usize) -> Option<u32> {
        let RowControl::Color(value) = &self.rows[index].control else {
            return None;
        };
        let text = if index == self.selected {
            self.edit.as_deref().unwrap_or(value)
        } else {
            value
        };
        parsed_color(text)
    }

    fn render_row(&self, index: usize) -> Div {
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
        let mut line = div()
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
                    .child(row.label.clone()),
            )
            .child(self.render_value_cell(index));
        if index == self.selected {
            line = line
                .bg(rgb(self.palette.row_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.row_border_selected));
            if let Some(dropdown) = &self.dropdown {
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
                    | RowControl::Color(_) => {}
                }
            }
        }
        container.child(line)
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
                .map(|index| self.render_row(index).into_any_element())
                .collect();
        div()
            .track_focus(&self.focus_handle)
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, _window, cx| this.on_key(event, cx)),
            )
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

fn row_height(row: &Row) -> f32 {
    let header = if row.section_label.is_some() {
        super::PANEL_SECTION_HEADER_HEIGHT
    } else {
        0.0
    };
    super::PANEL_ROW_HEIGHT + header
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
        intent, is_number_seed, parsed_color, parsed_number, scroll_offset_for, visible_row_range,
        Intent, Row, RowControl,
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
