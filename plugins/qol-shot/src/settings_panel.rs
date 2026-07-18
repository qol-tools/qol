use std::path::{Path, PathBuf};

use gpui::*;
use qol_config::contract::{FieldDefault, FieldKind};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection};
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{Anchor, Surface, SurfaceDismisser, SurfaceKind};
use qol_gpui::theme::{shot_preview_runtime, ShotPreviewPalette};

const PANEL_WIDTH: f32 = 520.0;
const PANEL_ROW_HEIGHT: f32 = 36.0;
const PANEL_SECTION_HEADER_HEIGHT: f32 = 26.0;
const PANEL_CHROME_HEIGHT: f32 = 72.0;

fn panel_height(rows: &[Row]) -> f32 {
    let headers = rows
        .iter()
        .filter(|row| row.section_label.is_some())
        .count() as f32;
    PANEL_CHROME_HEIGHT
        + rows.len() as f32 * PANEL_ROW_HEIGHT
        + headers * PANEL_SECTION_HEADER_HEIGHT
}

pub(crate) fn open(tracker: &MonitorTracker, cx: &mut App) -> anyhow::Result<()> {
    let spec = qol_config::contract::parse_spec_str(crate::config::contract())
        .map_err(|error| anyhow::anyhow!("contract parse failed: {error:?}"))?;
    let path = config_path()?;
    let values = load_values(&path);
    let resolved = qol_config::normalized::resolve_config(&spec, &values)
        .map_err(|errors| anyhow::anyhow!("contract resolve failed: {errors:?}"))?;
    let rows = rows_from_resolved(&resolved);
    let title = format!("qol-shot-settings-{}", std::process::id());
    Surface::new(SurfaceKind::Panel)
        .title(title)
        .anchor(Anchor::MonitorCenter)
        .size(size(px(PANEL_WIDTH), px(panel_height(&rows))))
        .show_focused(tracker, cx, move |dismisser, _window, cx| {
            SettingsPanelView::new(rows, values, path, dismisser, cx)
        })
        .map(|_| ())
}

fn config_path() -> anyhow::Result<PathBuf> {
    qol_config::plugin_config_paths_from_env(crate::PLUGIN_ID)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no plugin config path available"))
}

fn load_values(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn save_values(path: &Path, values: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(values) {
        Ok(raw) => {
            if let Err(error) = std::fs::write(path, raw) {
                eprintln!("[qol-shot] settings save failed: {error:#}");
            }
        }
        Err(error) => eprintln!("[qol-shot] settings serialize failed: {error:#}"),
    }
}

struct SettingsPanelView {
    rows: Vec<Row>,
    values: serde_json::Value,
    path: PathBuf,
    selected: usize,
    edit: Option<String>,
    dismisser: SurfaceDismisser,
    palette: ShotPreviewPalette,
    focus_handle: FocusHandle,
}

impl SettingsPanelView {
    fn new(
        rows: Vec<Row>,
        values: serde_json::Value,
        path: PathBuf,
        dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            rows,
            values,
            path,
            selected: 0,
            edit: None,
            dismisser,
            palette: shot_preview_runtime(),
            focus_handle: cx.focus_handle(),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let key_char = event.keystroke.key_char.as_deref();
        let Some(intent) = intent(key, key_char, self.edit.is_some()) else {
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
            RowControl::Toggle(_) | RowControl::Text(_) | RowControl::TextList(_) => return,
        }
        self.persist();
    }

    fn activate(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        match &row.control {
            RowControl::Toggle(_) => self.toggle(),
            RowControl::Select { .. } => self.adjust(1.0),
            RowControl::Number { .. } | RowControl::Text(_) | RowControl::TextList(_) => {
                self.begin_edit()
            }
        }
    }

    fn begin_edit(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        self.edit = match &row.control {
            RowControl::Text(value) => Some(value.clone()),
            RowControl::TextList(values) => Some(values.join(", ")),
            RowControl::Number { value, .. } => Some(format_number(*value)),
            RowControl::Toggle(_) | RowControl::Select { .. } => None,
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
            RowControl::Toggle(_) | RowControl::Select { .. } => return,
        }
        self.persist();
    }

    fn persist(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        set_config_value(
            &mut self.values,
            &row.config_key,
            row_value_json(&row.control),
        );
        save_values(&self.path, &self.values);
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
            | RowControl::Number { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_) => self.palette.label_text,
        }
    }

    fn render_row(&self, index: usize) -> Div {
        let row = &self.rows[index];
        let mut container = div().flex().flex_col().gap_1();
        if let Some(section) = &row.section_label {
            container = container.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.action_glyph))
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
                .bg(rgb(self.palette.action_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.action_border_selected));
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
            .border_color(rgb(self.palette.thumb_border))
            .bg(rgb(self.palette.window_bg))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child("QoL Shot Settings"),
            )
            .children(items)
    }
}

pub(crate) fn parsed_number(edit: &str, min: Option<f64>, max: Option<f64>) -> Option<f64> {
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
pub(crate) enum RowControl {
    Toggle(bool),
    Select {
        options: Vec<String>,
        labels: Vec<String>,
        index: usize,
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

pub(crate) struct Row {
    pub(crate) section_label: Option<String>,
    pub(crate) label: String,
    pub(crate) config_key: String,
    pub(crate) control: RowControl,
}

pub(crate) fn rows_from_resolved(config: &ResolvedConfig) -> Vec<Row> {
    let mut rows = Vec::new();
    for field in &config.fields {
        push_row(&mut rows, None, field);
    }
    for section in &config.sections {
        push_section_rows(&mut rows, section);
    }
    rows
}

fn push_section_rows(rows: &mut Vec<Row>, section: &ResolvedSection) {
    let mut label = Some(section.label.clone());
    for field in &section.fields {
        let before = rows.len();
        push_row(rows, label.clone(), field);
        if rows.len() > before {
            label = None;
        }
    }
}

fn push_row(rows: &mut Vec<Row>, section_label: Option<String>, field: &ResolvedField) {
    let Some(control) = control_for(field) else {
        return;
    };
    rows.push(Row {
        section_label,
        label: field.label.clone(),
        config_key: field.config_key.clone(),
        control,
    });
}

fn control_for(field: &ResolvedField) -> Option<RowControl> {
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
            let index = field.options.iter().position(|o| *o == current)?;
            let labels = field
                .options
                .iter()
                .map(|o| {
                    field
                        .option_labels
                        .get(o)
                        .cloned()
                        .unwrap_or_else(|| o.clone())
                })
                .collect();
            Some(RowControl::Select {
                options: field.options.clone(),
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
            FieldDefault::StringArray(values) => Some(RowControl::TextList(values.clone())),
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

pub(crate) fn row_value_json(control: &RowControl) -> serde_json::Value {
    match control {
        RowControl::Toggle(value) => serde_json::json!(value),
        RowControl::Select { options, index, .. } => serde_json::json!(options[*index]),
        RowControl::Number { value, .. } => serde_json::json!(value),
        RowControl::Text(value) => serde_json::json!(value),
        RowControl::TextList(values) => serde_json::json!(values),
    }
}

pub(crate) fn set_config_value(
    root: &mut serde_json::Value,
    dotted_key: &str,
    value: serde_json::Value,
) {
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
pub(crate) enum Intent {
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

pub(crate) fn intent(key: &str, key_char: Option<&str>, editing: bool) -> Option<Intent> {
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
        intent, parsed_number, rows_from_resolved, set_config_value, Intent, ResolvedConfig,
        RowControl,
    };

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
"#;

    fn resolved(overrides: serde_json::Value) -> ResolvedConfig {
        let spec = qol_config::contract::parse_spec_str(SPEC).unwrap();
        qol_config::normalized::resolve_config(&spec, &overrides).unwrap()
    }

    #[test]
    fn rows_map_every_supported_kind_with_override_values() {
        let rows = rows_from_resolved(&resolved(serde_json::json!({
            "capture": { "pin_border": false, "saved_feedback": "toast" }
        })));
        assert_eq!(rows.len(), 5);
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
        assert!(
            matches!(&rows[4].control, RowControl::TextList(v) if v == &vec!["mic".to_string()])
        );
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
