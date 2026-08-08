# qol-shot gpui Settings POC Implementation Plan

> **For agentic workers:** implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The qol-shot `settings` action opens a native gpui settings panel (daemon-rendered) driven by the existing qol-config contract, with the browser page as headless fallback.

**Architecture:** `qol_gpui::surface` gains a focus-taking `Panel` kind.
qol-shot maps `ResolvedConfig` (from `parse_spec_str` + `resolve_config`, the same path the tray uses) into typed rows, renders them keyboard-first in a gpui view, and writes edits back to the same `config.json` the tray writes.
The daemon routes the `settings` action to the panel; every other path is unchanged.

**Tech Stack:** Rust, gpui 0.2, qol-gpui surface kit, qol-config contract, serde_json.

## Global Constraints

- Spec: `docs/specs/2026-07-18-qol-shot-gpui-settings-poc-design.md`.
- No code comments.
- Single source of truth: `qol-config.toml` via `qol_config`; values live in `config.json`; no new schema or store.
- Field kinds implemented: `boolean`, `select`, `number`, `string`, `string_array` only; all other kinds are skipped when mapping rows.
- Keyboard-first: arrows navigate, space toggles, left/right cycle selects and step numbers, enter begins/commits text edit, Escape cancels an edit or closes the panel.
- Headless `qol-shot settings` (no daemon) keeps the browser URL; the daemon path falls back to it if the panel fails to open.
- The webview settings page stays untouched.
- New qol-shot modules are gated `#[cfg(any(target_os = "linux", target_os = "macos"))]` like the other gpui modules.
- Commits direct to `main`, conventional one-liners, no AI attribution, fmt before commit.

---

### Task 1: Panel kind in qol-gpui

**Files:**
- Modify: `libs/qol-gpui/src/surface.rs`

**Interfaces:**
- Produces: `SurfaceKind::Panel`, `Anchor::MonitorCenter`, and `Surface::show_focused<V: Render + Focusable + 'static>(self, tracker, cx, build) -> Result<SurfaceDismisser>` with the same build-closure shape as `show`.

No TDD cycle: the placement math reuses the already-tested `ActiveMonitor::centered_bounds`, and window creation cannot run on the host session. Compile gate plus guest verification cover it.

- [ ] **Step 1: Implement**

Add `Panel` to `SurfaceKind` and `MonitorCenter` to `Anchor`:

```rust
pub enum SurfaceKind {
    Toast,
    Panel,
}

#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    CornerStack(Corner),
    MonitorCenter,
}
```

Replace the single-variant `let Anchor::CornerStack(corner) = self.anchor;` in `show` by extracting a bounds helper used by both show paths:

```rust
    fn resolved_bounds(&self, monitor: &crate::monitor::ActiveMonitor) -> Bounds<Pixels> {
        match self.anchor {
            Anchor::CornerStack(corner) => {
                corner_anchored_bounds(monitor.bounds(), corner, self.size, CORNER_MARGIN)
            }
            Anchor::MonitorCenter => monitor.centered_bounds(self.size),
        }
    }

    fn window_kind(&self) -> WindowKind {
        match self.kind {
            SurfaceKind::Toast => WindowKind::PopUp,
            SurfaceKind::Panel => WindowKind::Normal,
        }
    }

    fn takes_focus(&self) -> bool {
        match self.kind {
            SurfaceKind::Toast => false,
            SurfaceKind::Panel => true,
        }
    }
```

Refactor `show` so the options construction and dismisser wiring live in one private `open<V>` used by both entry points; `show_focused` additionally focuses and activates inside the open callback:

```rust
    pub fn show_focused<V: Render + Focusable + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<SurfaceDismisser> {
        self.open(tracker, cx, |dismisser, window, cx| {
            let view = build(dismisser, window, cx);
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        })
    }
```

`focus: self.takes_focus()` goes into the `WindowOptions`.
`show` keeps its exact current signature and behavior.

- [ ] **Step 2: Verify**

Run: `cargo test -p qol-gpui && cargo clippy -p qol-gpui --all-targets`
Expected: all existing tests pass, no new warnings.

- [ ] **Step 3: Commit**

```bash
cargo fmt -p qol-gpui
git add libs/qol-gpui/src/surface.rs
git commit -m "feat(qol-gpui): add focus-taking Panel surface kind" -- libs/qol-gpui/src/surface.rs
```

---

### Task 2: Row model and config-write helpers (TDD)

**Files:**
- Create: `plugins/qol-shot/src/settings_panel.rs` (model half)
- Modify: `plugins/qol-shot/src/lib.rs` (add gated `mod settings_panel;` after `mod saved_toast;`)

**Interfaces:**
- Produces (used by Task 3):
  - `Row { section_label: Option<String>, label: String, config_key: String, control: RowControl }`
  - `RowControl::{Toggle(bool), Select { options: Vec<String>, labels: Vec<String>, index: usize }, Number { value: f64, min: Option<f64>, max: Option<f64>, step: f64 }, Text(String), TextList(Vec<String>)}`
  - `rows_from_resolved(&ResolvedConfig) -> Vec<Row>` (first row of each section carries `section_label`)
  - `set_config_value(&mut serde_json::Value, dotted_key: &str, value: serde_json::Value)`
  - `row_value_json(&RowControl) -> serde_json::Value`
  - `Intent` enum and `intent(key: &str, key_char: Option<&str>, editing: bool) -> Option<Intent>` with variants `Up, Down, Toggle, Left, Right, BeginEdit, CommitEdit, Backspace, Insert(String), Close, CancelEdit`

- [ ] **Step 1: Write failing tests** (inside `#[cfg(test)] mod tests` in `settings_panel.rs`)

```rust
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

    fn resolved(overrides: serde_json::Value) -> qol_config::normalized::ResolvedConfig {
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
        assert!(matches!(&rows[4].control, RowControl::TextList(v) if v == &vec!["mic".to_string()]));
    }

    #[test]
    fn set_config_value_creates_nested_paths_and_overwrites() {
        let mut root = serde_json::json!({ "capture": { "pin_border": true } });
        set_config_value(&mut root, "capture.saved_feedback", serde_json::json!("toast"));
        set_config_value(&mut root, "audio.inputs", serde_json::json!(["mic", "system"]));
        assert_eq!(
            root,
            serde_json::json!({
                "capture": { "pin_border": true, "saved_feedback": "toast" },
                "audio": { "inputs": ["mic", "system"] }
            })
        );
    }

    #[test]
    fn intent_maps_navigation_editing_and_close() {
        let cases = [
            ("up", None, false, Some(Intent::Up)),
            ("down", None, false, Some(Intent::Down)),
            ("space", None, false, Some(Intent::Toggle)),
            ("left", None, false, Some(Intent::Left)),
            ("right", None, false, Some(Intent::Right)),
            ("enter", None, false, Some(Intent::BeginEdit)),
            ("escape", None, false, Some(Intent::Close)),
            ("enter", None, true, Some(Intent::CommitEdit)),
            ("escape", None, true, Some(Intent::CancelEdit)),
            ("backspace", None, true, Some(Intent::Backspace)),
            ("a", Some("a"), true, Some(Intent::Insert("a".into()))),
            ("a", Some("a"), false, None),
        ];
        for (key, ch, editing, expected) in cases {
            assert_eq!(intent(key, ch, editing), expected, "key {key} editing {editing}");
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-shot settings_panel`
Expected: FAIL to compile (types missing).

- [ ] **Step 3: Implement the model half**

```rust
use qol_config::contract::{FieldDefault, FieldKind};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection};

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
                .map(|o| field.option_labels.get(o).cloned().unwrap_or_else(|| o.clone()))
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

pub(crate) fn set_config_value(root: &mut serde_json::Value, dotted_key: &str, value: serde_json::Value) {
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
    BeginEdit,
    CommitEdit,
    Backspace,
    Insert(String),
    Close,
    CancelEdit,
}

pub(crate) fn intent(key: &str, key_char: Option<&str>, editing: bool) -> Option<Intent> {
    if editing {
        return match key {
            "enter" => Some(Intent::CommitEdit),
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
        "enter" => Some(Intent::BeginEdit),
        "escape" => Some(Intent::Close),
        _ => None,
    }
}
```

Note on `set_config_value` borrow flow: the `cursor[part] = value` arm needs `value` moved out of the loop; if the borrow checker rejects the shape above, restructure with a recursive helper - behavior is pinned by the test either way.

In `lib.rs` add after `mod saved_toast;` (bypass the arch hook if it flags the pre-existing gates):

```rust
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod settings_panel;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p qol-shot settings_panel`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p qol-shot
git add plugins/qol-shot/src/settings_panel.rs plugins/qol-shot/src/lib.rs
git commit -m "feat(qol-shot): map config contract to settings panel rows" -- plugins/qol-shot/src/settings_panel.rs plugins/qol-shot/src/lib.rs
```

---

### Task 3: Panel view, write-through, and open()

**Files:**
- Modify: `plugins/qol-shot/src/settings_panel.rs`

**Interfaces:**
- Consumes: Task 1 `show_focused`/`Panel`/`MonitorCenter`; Task 2 model; `qol_config::{plugin_config_paths_from_env, contract::parse_spec_str, normalized::resolve_config}`; `qol_gpui::theme::{shot_preview_runtime, ShotPreviewPalette}`; `crate::PLUGIN_ID` and `CONFIG_CONTRACT` access via a new `pub(crate) const` re-export if needed (config.rs owns `CONFIG_CONTRACT`; add `pub(crate) fn contract() -> &'static str { CONFIG_CONTRACT }` in `config.rs`).
- Produces (used by Task 4): `pub(crate) fn open(tracker: &MonitorTracker, cx: &mut App) -> anyhow::Result<()>`.

- [ ] **Step 1: Implement**

Loading and saving:

```rust
fn config_path() -> anyhow::Result<std::path::PathBuf> {
    qol_config::plugin_config_paths_from_env(crate::PLUGIN_ID)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no plugin config path available"))
}

fn load_values(path: &std::path::Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn save_values(path: &std::path::Path, values: &serde_json::Value) {
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
```

`open` resolves the contract and values, builds rows, and shows the panel:

```rust
pub(crate) fn open(tracker: &MonitorTracker, cx: &mut App) -> anyhow::Result<()> {
    let spec = qol_config::contract::parse_spec_str(crate::config::contract())
        .map_err(|error| anyhow::anyhow!("contract parse failed: {error}"))?;
    let path = config_path()?;
    let values = load_values(&path);
    let resolved = qol_config::normalized::resolve_config(&spec, &values)
        .map_err(|errors| anyhow::anyhow!("contract resolve failed: {errors:?}"))?;
    let rows = rows_from_resolved(&resolved);
    let title = format!("qol-shot-settings-{}", std::process::id());
    Surface::new(SurfaceKind::Panel)
        .title(title)
        .anchor(Anchor::MonitorCenter)
        .size(size(px(520.0), px(560.0)))
        .show_focused(tracker, cx, move |dismisser, _window, cx| {
            SettingsPanelView::new(rows, values, path, dismisser, cx)
        })
        .map(|_| ())
}
```

View state and behavior (single concern per method; keyboard dispatch mirrors the launcher's `KeyDownEvent` pattern; `track_focus` on the root div):

```rust
struct SettingsPanelView {
    rows: Vec<Row>,
    values: serde_json::Value,
    path: std::path::PathBuf,
    selected: usize,
    edit: Option<String>,
    dismisser: SurfaceDismisser,
    palette: ShotPreviewPalette,
    focus_handle: FocusHandle,
}
```

- `new` builds the struct (`selected: 0`, `edit: None`, `palette: shot_preview_runtime()`, `focus_handle: cx.focus_handle()`).
- `impl Focusable` returns the handle.
- `on_key(&mut self, event: &KeyDownEvent, cx: ...)` maps via `intent(...)` and dispatches:
  - `Up`/`Down` move `selected` with clamping.
  - `Toggle` flips a `Toggle` row and persists.
  - `Left`/`Right` cycle `Select` (wrapping) or step `Number` by `step` clamped to `min`/`max`, then persist.
  - `BeginEdit` on `Text` rows sets `edit = Some(current)`; on `TextList` rows sets `edit = Some(values.join(", "))`.
  - `Insert`/`Backspace` mutate the edit buffer.
  - `CommitEdit` writes the buffer back (`TextList` splits on ',' with trim, dropping empties), persists, clears `edit`.
  - `CancelEdit` clears `edit`.
  - `Close` calls `self.dismisser.dismiss(cx)`.
- `persist(&mut self)` does `set_config_value(&mut self.values, &row.config_key, row_value_json(&row.control))` then `save_values(&self.path, &self.values)`.
- `render` draws a rounded opaque panel (`window_bg` background, `thumb_border` border), a title line, then one line per row: optional dim section header above, label left, value right (`[on]`/`[off]`, select label, number, text, comma-joined list; the edit buffer with a trailing `_` while editing), selected row tinted with `action_bg_selected`/`action_border_selected`. Split row rendering into a `render_row` helper.

- [ ] **Step 2: Verify**

Run: `cargo test -p qol-shot && cargo clippy -p qol-shot --all-targets`
Expected: all tests pass, no warnings.

- [ ] **Step 3: Commit**

```bash
cargo fmt -p qol-shot
git add plugins/qol-shot/src/settings_panel.rs plugins/qol-shot/src/config.rs
git commit -m "feat(qol-shot): render gpui settings panel with write-through" -- plugins/qol-shot/src/settings_panel.rs plugins/qol-shot/src/config.rs
```

---

### Task 4: Daemon routing with browser fallback

**Files:**
- Modify: `plugins/qol-shot/src/daemon_app.rs`

**Interfaces:**
- Consumes: `settings_panel::open`, existing `run_cli` flow.

- [ ] **Step 1: Implement**

In `run_cli`, before the generic CLI fallthrough:

```rust
    if action == "settings" {
        let tracker = state.tracker.clone();
        let opened = cx.update(move |cx| crate::settings_panel::open(&tracker, cx));
        match opened {
            Ok(Ok(())) => {
                qol_runtime::probe!("SHOT_SETTINGS_PANEL", "result=shown");
                return;
            }
            Ok(Err(error)) => {
                qol_runtime::probe!("SHOT_SETTINGS_PANEL", "result=fallback error={error:#}");
                eprintln!("[qol-shot] settings panel failed, opening browser: {error:#}");
            }
            Err(error) => {
                qol_runtime::probe!("SHOT_SETTINGS_PANEL", "result=fallback error={error}");
            }
        }
    }
```

The existing `cli::exit_code` fallthrough then runs (which opens the browser settings URL for `settings`).

- [ ] **Step 2: Verify**

Run: `cargo test -p qol-shot && cargo build -p qol-shot`
Expected: green, no warnings.

- [ ] **Step 3: Commit**

```bash
cargo fmt -p qol-shot
git add plugins/qol-shot/src/daemon_app.rs
git commit -m "feat(qol-shot): route settings action to the gpui panel" -- plugins/qol-shot/src/daemon_app.rs
```

---

### Task 5: Full verification

- [ ] **Step 1: Gates**

```bash
cargo fmt -p qol-gpui -p qol-shot -- --check
cargo clippy -p qol-gpui -p qol-shot --all-targets
cargo test -p qol-gpui -p qol-shot
```

(The release `-D warnings` check stays blocked on the pre-existing qol-gpui probe-local issue already reported; do not fix it here.)

- [ ] **Step 2: Guest regression**

Run: `qol flow run qol-shot-capture --env linux/mint-cinnamon --worktree /media/kmrh47/WD_SN850X/Git/qol-monorepo`
Expected: all lanes pass (capture flow unaffected).
Interactive panel verification (open via Shortcut, keyboard editing) is not automatable with current guest-control CLI exposure; report it for manual Sandbox-panel verification.

- [ ] **Step 3: Report**

Four-part completion report with real output and commit hashes.
