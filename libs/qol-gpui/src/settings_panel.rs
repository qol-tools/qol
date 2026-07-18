use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::*;
use qol_config::contract::{FieldDefault, FieldKind};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection};
use qol_conventions::DEFAULT_PORT;

use crate::dropdown::{Dropdown, DropdownStyle};
use crate::monitor::MonitorTracker;
use crate::surface::{Anchor, Surface, SurfaceDismisser, SurfaceKind};
use crate::theme::{settings_panel_runtime, SettingsPanelPalette};

const PANEL_WIDTH: f32 = 520.0;
const PANEL_ROW_HEIGHT: f32 = 36.0;
const PANEL_SECTION_HEADER_HEIGHT: f32 = 26.0;
const PANEL_CHROME_HEIGHT: f32 = 72.0;

#[derive(Clone, Copy)]
pub struct SettingsPanel {
    pub plugin_id: &'static str,
    pub contract: &'static str,
    pub heading: &'static str,
}

pub type QueryOptions = dyn Fn(&str) -> Vec<(String, String)>;

fn panel_height(rows: &[Row]) -> f32 {
    let headers = rows
        .iter()
        .filter(|row| row.section_label.is_some())
        .count() as f32;
    PANEL_CHROME_HEIGHT
        + rows.len() as f32 * PANEL_ROW_HEIGHT
        + headers * PANEL_SECTION_HEADER_HEIGHT
}

pub fn open(
    panel: SettingsPanel,
    tracker: &MonitorTracker,
    provider: &QueryOptions,
    cx: &mut App,
) -> anyhow::Result<()> {
    let spec = qol_config::contract::parse_spec_str(panel.contract)
        .map_err(|error| anyhow::anyhow!("contract parse failed: {error:?}"))?;
    let path = config_path(panel.plugin_id)?;
    let values = load_values(panel.plugin_id, &path);
    let resolved = qol_config::normalized::resolve_config(&spec, &values)
        .map_err(|errors| anyhow::anyhow!("contract resolve failed: {errors:?}"))?;
    let rows = rows_from_resolved(&resolved, provider);
    let title = format!("{}-settings-{}", panel.plugin_id, std::process::id());
    Surface::new(SurfaceKind::Panel)
        .title(title)
        .anchor(Anchor::MonitorCenter)
        .size(size(px(PANEL_WIDTH), px(panel_height(&rows))))
        .show_focused(tracker, cx, move |dismisser, _window, cx| {
            SettingsPanelView::new(panel, rows, values, path, dismisser, cx)
        })
        .map(|_| ())
}

fn config_path(plugin_id: &str) -> anyhow::Result<PathBuf> {
    qol_config::plugin_config_paths_from_env(plugin_id)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no plugin config path available"))
}

fn tray_config_route(plugin_id: &str) -> String {
    format!("/api/plugins/{plugin_id}/config")
}

fn load_values(plugin_id: &str, path: &Path) -> serde_json::Value {
    if let Ok((200, body)) = tray_http("GET", &tray_config_route(plugin_id), None) {
        if let Ok(values) = serde_json::from_str(&body) {
            return values;
        }
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn save_values(plugin_id: &str, path: &Path, values: &serde_json::Value) {
    let body = match serde_json::to_string(values) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("[{plugin_id}] settings serialize failed: {error:#}");
            return;
        }
    };
    match tray_http("PUT", &tray_config_route(plugin_id), Some(&body)) {
        Ok((200, _)) => return,
        Ok((status, payload)) => {
            eprintln!(
                "[{plugin_id}] settings save rejected by tray ({status}): {}",
                payload.trim()
            );
            return;
        }
        Err(error) => eprintln!("[{plugin_id}] tray unreachable, saving locally: {error:#}"),
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(path, body) {
        eprintln!("[{plugin_id}] settings save failed: {error:#}");
    }
}

fn tray_http(method: &str, route: &str, body: Option<&str>) -> anyhow::Result<(u16, String)> {
    use std::io::{Read, Write};
    let stream = std::net::TcpStream::connect((qol_conventions::LOCAL_HOST, DEFAULT_PORT))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut stream = stream;
    let body = body.unwrap_or("");
    let request = format!(
        "{method} {route} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        qol_conventions::LOCAL_HOST,
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    Ok(parse_http_response(&raw))
}

fn parse_http_response(raw: &str) -> (u16, String) {
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

struct SettingsPanelView {
    panel: SettingsPanel,
    rows: Vec<Row>,
    values: serde_json::Value,
    path: PathBuf,
    selected: usize,
    edit: Option<String>,
    dropdown: Option<Dropdown>,
    dismisser: SurfaceDismisser,
    palette: SettingsPanelPalette,
    focus_handle: FocusHandle,
}

impl SettingsPanelView {
    fn new(
        panel: SettingsPanel,
        rows: Vec<Row>,
        values: serde_json::Value,
        path: PathBuf,
        dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            panel,
            rows,
            values,
            path,
            selected: 0,
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
            Intent::Up => self.selected = self.selected.saturating_sub(1),
            Intent::Down => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
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
            | RowControl::TextList(_) => self.dropdown = None,
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
            | RowControl::TextList(_) => return,
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
            RowControl::Number { .. } | RowControl::Text(_) | RowControl::TextList(_) => {
                self.begin_edit()
            }
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
        let mut config = serde_json::json!({});
        for row in &self.rows {
            set_config_value(&mut config, &row.config_key, row_value_json(&row.control));
        }
        self.values = config;
        save_values(self.panel.plugin_id, &self.path, &self.values);
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
            | RowControl::TextList(_) => self.palette.label_text,
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
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(self.value_color(index)))
                    .child(self.display_value(index)),
            );
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
                    | RowControl::TextList(_) => {}
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
        let items: Vec<AnyElement> = (0..self.rows.len())
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

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

#[derive(Debug)]
enum RowControl {
    Toggle(bool),
    Select {
        options: Vec<String>,
        labels: Vec<String>,
        index: usize,
    },
    MultiSelect {
        options: Vec<String>,
        labels: Vec<String>,
        selected: Vec<bool>,
    },
    Number {
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
    },
    Text(String),
    TextList(Vec<String>),
}

struct Row {
    section_label: Option<String>,
    label: String,
    config_key: String,
    control: RowControl,
}

fn rows_from_resolved(config: &ResolvedConfig, provider: &QueryOptions) -> Vec<Row> {
    let mut rows = Vec::new();
    for field in &config.fields {
        push_row(&mut rows, None, field, provider);
    }
    for section in &config.sections {
        push_section_rows(&mut rows, section, provider);
    }
    rows
}

fn push_section_rows(rows: &mut Vec<Row>, section: &ResolvedSection, provider: &QueryOptions) {
    let mut label = Some(section.label.clone());
    for field in &section.fields {
        let before = rows.len();
        push_row(rows, label.clone(), field, provider);
        if rows.len() > before {
            label = None;
        }
    }
}

fn push_row(
    rows: &mut Vec<Row>,
    section_label: Option<String>,
    field: &ResolvedField,
    provider: &QueryOptions,
) {
    let Some(control) = control_for(field, provider) else {
        return;
    };
    rows.push(Row {
        section_label,
        label: field.label.clone(),
        config_key: field.config_key.clone(),
        control,
    });
}

fn control_for(field: &ResolvedField, provider: &QueryOptions) -> Option<RowControl> {
    match field.kind {
        FieldKind::Boolean => match field.value {
            FieldDefault::Boolean(value) => Some(RowControl::Toggle(value)),
            _ => None,
        },
        FieldKind::Select => {
            let current = match &field.value {
                FieldDefault::String(value) => value.clone(),
                _ => return None,
            };
            let (options, labels) = select_options(field, &current, provider);
            let index = options.iter().position(|o| *o == current)?;
            Some(RowControl::Select {
                options,
                labels,
                index,
            })
        }
        FieldKind::Number => match field.value {
            FieldDefault::Number(value) => Some(RowControl::Number {
                value,
                min: field.number.min,
                max: field.number.max,
                step: field.number.step.unwrap_or(1.0),
            }),
            _ => None,
        },
        FieldKind::String => match &field.value {
            FieldDefault::String(value) => Some(RowControl::Text(value.clone())),
            _ => None,
        },
        FieldKind::StringArray => match &field.value {
            FieldDefault::StringArray(values) => {
                if field.options.is_empty() {
                    Some(RowControl::TextList(values.clone()))
                } else {
                    Some(RowControl::MultiSelect {
                        selected: field
                            .options
                            .iter()
                            .map(|option| values.contains(option))
                            .collect(),
                        options: field.options.clone(),
                        labels: option_labels_for(field),
                    })
                }
            }
            _ => None,
        },
        FieldKind::ObjectArray
        | FieldKind::ObjectMap
        | FieldKind::Color
        | FieldKind::Action
        | FieldKind::List
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad => None,
    }
}

fn option_labels_for(field: &ResolvedField) -> Vec<String> {
    field
        .options
        .iter()
        .map(|option| {
            field
                .option_labels
                .get(option)
                .cloned()
                .unwrap_or_else(|| option.clone())
        })
        .collect()
}

fn select_options(
    field: &ResolvedField,
    current: &str,
    provider: &QueryOptions,
) -> (Vec<String>, Vec<String>) {
    let dynamic = field.query.as_deref().map(provider).unwrap_or_default();
    let mut options = field.options.clone();
    if field.query.is_some() {
        for option in field.option_labels.keys() {
            if !options.iter().any(|candidate| candidate == option) {
                options.push(option.clone());
            }
        }
    }
    for (value, _) in &dynamic {
        if !options.iter().any(|option| option == value) {
            options.push(value.clone());
        }
    }
    if !options.iter().any(|option| option == current) {
        options.insert(0, current.to_string());
    }
    let labels = options
        .iter()
        .map(|option| {
            field
                .option_labels
                .get(option)
                .cloned()
                .or_else(|| {
                    dynamic
                        .iter()
                        .find(|(value, _)| value == option)
                        .map(|(_, label)| label.clone())
                })
                .unwrap_or_else(|| option.clone())
        })
        .collect();
    (options, labels)
}

fn row_value_json(control: &RowControl) -> serde_json::Value {
    match control {
        RowControl::Toggle(value) => serde_json::json!(value),
        RowControl::Select { options, index, .. } => serde_json::json!(options[*index]),
        RowControl::MultiSelect {
            options, selected, ..
        } => {
            let values: Vec<&String> = options
                .iter()
                .zip(selected)
                .filter(|(_, on)| **on)
                .map(|(option, _)| option)
                .collect();
            serde_json::json!(values)
        }
        RowControl::Number { value, .. } => serde_json::json!(value),
        RowControl::Text(value) => serde_json::json!(value),
        RowControl::TextList(values) => serde_json::json!(values),
    }
}

fn set_config_value(root: &mut serde_json::Value, dotted_key: &str, value: serde_json::Value) {
    if !root.is_object() {
        *root = serde_json::json!({});
    }
    let mut cursor = root;
    let mut parts = dotted_key.split('.').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            cursor[part] = value;
            return;
        }
        if !cursor[part].is_object() {
            cursor[part] = serde_json::json!({});
        }
        cursor = &mut cursor[part];
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

#[cfg(test)]
mod tests {
    use super::{
        intent, is_number_seed, parse_http_response, parsed_number, row_value_json,
        rows_from_resolved, set_config_value, Intent, ResolvedConfig, RowControl,
    };

    #[test]
    fn http_response_parsing_extracts_status_and_body() {
        let cases = [
            ("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}", 200, "{}"),
            (
                "HTTP/1.1 422 Unprocessable\r\n\r\nbad value",
                422,
                "bad value",
            ),
            ("garbage", 0, ""),
        ];
        for (raw, status, body) in cases {
            assert_eq!(
                parse_http_response(raw),
                (status, body.to_string()),
                "raw: {raw}"
            );
        }
    }

    const SPEC: &str = r#"
schema_version = 1

[section.capture]
label = "Capture"

[field.pin_border]
type = "boolean"
config_key = "capture.pin_border"
label = "Pinned Preview Border"
section = "capture"
default = true

[field.saved_feedback]
type = "select"
config_key = "capture.saved_feedback"
label = "Saved Feedback"
section = "capture"
default = "notification"
options = ["notification", "toast"]

[field.crf]
type = "number"
config_key = "video.crf"
label = "CRF"
section = "capture"
default = 18
min = 0
max = 51
step = 1

[field.mic]
type = "string"
config_key = "audio.mic_device"
label = "Mic Device"
section = "capture"
default = "default"

[field.inputs]
type = "string_array"
config_key = "audio.inputs"
label = "Audio Inputs"
section = "capture"
default = ["mic"]
options = ["mic", "system"]

[field.inputs.option_labels]
mic = "Microphone"
system = "System Audio"

[field.tags]
type = "string_array"
config_key = "capture.tags"
label = "Tags"
section = "capture"
default = ["foo"]
"#;

    fn resolved(overrides: serde_json::Value) -> ResolvedConfig {
        let spec = qol_config::contract::parse_spec_str(SPEC).unwrap();
        qol_config::normalized::resolve_config(&spec, &overrides).unwrap()
    }

    #[test]
    fn rows_map_every_supported_kind_with_override_values() {
        let rows = rows_from_resolved(
            &resolved(serde_json::json!({
                "capture": { "pin_border": false, "saved_feedback": "toast" }
            })),
            &|_| Vec::new(),
        );
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].section_label.as_deref(), Some("Capture"));
        assert!(rows[1..].iter().all(|r| r.section_label.is_none()));
        assert!(matches!(rows[0].control, RowControl::Toggle(false)));
        match &rows[1].control {
            RowControl::Select { index, options, .. } => {
                assert_eq!(options[*index], "toast");
            }
            other => panic!("expected select, got {other:?}"),
        }
        match &rows[2].control {
            RowControl::Number { value, step, .. } => {
                assert_eq!((*value, *step), (18.0, 1.0));
            }
            other => panic!("expected number, got {other:?}"),
        }
        assert!(matches!(&rows[3].control, RowControl::Text(v) if v == "default"));
        match &rows[4].control {
            RowControl::MultiSelect {
                options,
                labels,
                selected,
            } => {
                assert_eq!(options, &["mic", "system"]);
                assert_eq!(labels, &["Microphone", "System Audio"]);
                assert_eq!(selected, &[true, false]);
            }
            other => panic!("expected multi select, got {other:?}"),
        }
        assert!(
            matches!(&rows[5].control, RowControl::TextList(v) if v == &vec!["foo".to_string()])
        );
    }

    #[test]
    fn query_select_merges_labeled_dynamic_and_current_options() {
        const QUERY_SPEC: &str = r#"
schema_version = 1

[field.mic]
type = "select"
config_key = "audio.mic_device"
label = "Mic Device"
default = "default"
query = "audio_sources"

[field.mic.option_labels]
default = "System Default"
"#;
        let spec = qol_config::contract::parse_spec_str(QUERY_SPEC).unwrap();
        let provider = |query: &str| {
            assert_eq!(query, "audio_sources");
            vec![("alsa_input.foo".to_string(), "Built-in Mic".to_string())]
        };

        let cases = [
            (
                serde_json::json!({}),
                vec!["default", "alsa_input.foo"],
                vec!["System Default", "Built-in Mic"],
                0usize,
            ),
            (
                serde_json::json!({ "audio": { "mic_device": "gone_device" } }),
                vec!["gone_device", "default", "alsa_input.foo"],
                vec!["gone_device", "System Default", "Built-in Mic"],
                0usize,
            ),
        ];
        for (overrides, expected_options, expected_labels, expected_index) in cases {
            let resolved = qol_config::normalized::resolve_config(&spec, &overrides).unwrap();
            let rows = rows_from_resolved(&resolved, &provider);
            match &rows[0].control {
                RowControl::Select {
                    options,
                    labels,
                    index,
                } => {
                    assert_eq!(options, &expected_options, "overrides: {overrides}");
                    assert_eq!(labels, &expected_labels, "overrides: {overrides}");
                    assert_eq!(*index, expected_index, "overrides: {overrides}");
                }
                other => panic!("expected select, got {other:?}"),
            }
        }
    }

    #[test]
    fn multi_select_value_json_preserves_option_order() {
        let control = RowControl::MultiSelect {
            options: vec!["mic".into(), "system".into()],
            labels: vec!["Microphone".into(), "System Audio".into()],
            selected: vec![false, true],
        };
        assert_eq!(row_value_json(&control), serde_json::json!(["system"]));
    }

    #[test]
    fn set_config_value_creates_nested_paths_and_overwrites() {
        let mut root = serde_json::json!({ "capture": { "pin_border": true } });
        set_config_value(
            &mut root,
            "capture.saved_feedback",
            serde_json::json!("toast"),
        );
        set_config_value(
            &mut root,
            "audio.inputs",
            serde_json::json!(["mic", "system"]),
        );
        assert_eq!(
            root,
            serde_json::json!({
                "capture": { "pin_border": true, "saved_feedback": "toast" },
                "audio": { "inputs": ["mic", "system"] }
            })
        );
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
