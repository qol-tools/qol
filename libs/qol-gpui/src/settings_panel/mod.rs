mod object_array_row;
mod persistence;
mod rows;
mod stream;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::surface::{OpenedSurface, Surface, SurfaceKind};
use rows::{rows_from_resolved, sections_from_resolved, Row, RowControl, RowSection};
use view::{SettingsPanelState, SettingsPanelView};

const PANEL_WIDTH: f32 = 520.0;
const PANEL_GAMEPAD_WIDTH: f32 = 820.0;
const PANEL_ROW_HEIGHT: f32 = 40.0;
const PANEL_DESCRIBED_ROW_HEIGHT: f32 = 56.0;
const PANEL_DESCRIPTION_LINE_HEIGHT: f32 = 14.0;
const PANEL_LIST_HEADER_HEIGHT: f32 = 24.0;
const PANEL_LIST_DESCRIPTION_HEIGHT: f32 = 16.0;
const PANEL_LIST_ITEM_HEIGHT: f32 = 48.0;
const PANEL_LIST_GAP: f32 = 4.0;
const PANEL_LIST_PADDING_Y: f32 = 8.0;
const PANEL_LIST_HEIGHT: f32 = PANEL_LIST_PADDING_Y
    + PANEL_LIST_HEADER_HEIGHT
    + rows::LIST_MAX_VISIBLE as f32 * (PANEL_LIST_ITEM_HEIGHT + PANEL_LIST_GAP);
const PANEL_OBJECT_ROW_HEIGHT: f32 = 30.0;
const PANEL_QR_CODE_HEIGHT: f32 = 180.0;
const PANEL_QR_URL_HEIGHT: f32 = 20.0;
const PANEL_SECTION_HEADER_HEIGHT: f32 = 26.0;
const PANEL_COLUMN_GAP: f32 = 4.0;
const PANEL_SECTION_MENU_ITEM_PADDING_Y: f32 = 8.0;
const PANEL_SECTION_MENU_LABEL_LINE_HEIGHT: f32 = 23.0;
const PANEL_SECTION_MENU_DESCRIPTION_LINE_HEIGHT: f32 = 19.0;
const PANEL_CHROME_HEIGHT: f32 = 62.0;
const PANEL_GAMEPAD_HEIGHT: f32 = 650.0;

#[derive(Clone)]
pub struct SettingsPanel {
    pub plugin_id: String,
    pub contract: String,
    pub heading: String,
}

type QueryHandler = dyn Fn(&str) -> Result<serde_json::Value, String> + Send + Sync;
type ActionHandler =
    dyn Fn(&str, serde_json::Value) -> Result<Option<serde_json::Value>, String> + Send + Sync;

#[derive(Clone)]
pub struct SettingsRuntime {
    query: Arc<QueryHandler>,
    action: Arc<ActionHandler>,
    poll_interval: Duration,
    query_intervals: std::collections::HashMap<String, Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsActivation {
    Focused,
    Opened,
    Replaced,
}

#[derive(Default)]
pub struct SettingsWindowHost {
    active: Option<ActivePanel>,
}

struct ActivePanel {
    plugin_id: String,
    surface: OpenedSurface<SettingsPanelView>,
}

pub struct PreparedSettingsPanel {
    panel: SettingsPanel,
    rows: Vec<Row>,
    sections: Vec<RowSection>,
    values: serde_json::Value,
    path: std::path::PathBuf,
    runtime: SettingsRuntime,
    daemon_port: Option<u16>,
}

struct PreparedPanel {
    panel: SettingsPanel,
    state: SettingsPanelState,
    size: Size<Pixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivationDecision {
    Focus,
    Open,
    Replace,
}

impl SettingsWindowHost {
    pub fn present_active(
        &mut self,
        plugin_id: &str,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> bool {
        if self.active.as_ref().map(|active| active.plugin_id.as_str()) != Some(plugin_id) {
            return false;
        }
        self.present(tracker, cx)
    }

    pub fn activate_prepared(
        &mut self,
        prepared: PreparedSettingsPanel,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> anyhow::Result<SettingsActivation> {
        let plugin_id = prepared.panel.plugin_id.clone();
        let decision = activation_decision(
            self.active.as_ref().map(|active| active.plugin_id.as_str()),
            &plugin_id,
        );
        if decision == ActivationDecision::Focus && self.present(tracker, cx) {
            return Ok(SettingsActivation::Focused);
        }

        let prepared = size_prepared_panel(prepared, tracker)?;
        if decision == ActivationDecision::Replace && self.active_is_open(cx) {
            self.replace(prepared, tracker, cx)?;
            return Ok(SettingsActivation::Replaced);
        }

        self.active = Some(open_prepared(prepared, tracker, cx)?);
        Ok(SettingsActivation::Opened)
    }

    fn present(&mut self, tracker: &MonitorTracker, cx: &mut App) -> bool {
        if !self.active_is_open(cx) {
            return false;
        }
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let resume_runtime_poll = !active.surface.is_visible();
        let updated = active.surface.present(tracker, cx);
        if updated && resume_runtime_poll {
            let _ = active.surface.handle.update(cx, |root, _, cx| {
                root.inner
                    .update(cx, |view, cx| view.resume_runtime_poll(cx));
            });
        }
        if updated {
            cx.activate(true);
        }
        updated
    }

    fn active_is_open(&mut self, cx: &mut App) -> bool {
        let Some(active) = self.active.as_ref() else {
            return false;
        };
        if active.surface.handle.update(cx, |_, _, _| ()).is_ok() {
            return true;
        }
        self.active = None;
        false
    }

    fn replace(
        &mut self,
        prepared: PreparedPanel,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let Some(active) = self.active.as_mut() else {
            return Err(anyhow::anyhow!(
                "settings window disappeared before replacement"
            ));
        };
        let plugin_id = prepared.panel.plugin_id.clone();
        let heading = prepared.panel.heading.clone();
        let dismisser = active.surface.dismisser.clone();
        let visible = active.surface.is_visible();
        active.surface.resize(prepared.size, cx)?;
        active.surface.handle.update(cx, move |root, window, cx| {
            root.inner.update(cx, |view, _| view.pause_runtime_poll());
            dismisser.retitle(window, heading);
            let inner =
                cx.new(|cx| SettingsPanelView::new(prepared.panel, prepared.state, dismisser, cx));
            let focus = inner.read(cx).focus_handle(cx);
            root.inner = inner;
            window.focus(&focus);
            if visible {
                window.activate_window();
            }
            cx.notify();
        })?;
        active.plugin_id = plugin_id;
        if !active.surface.present(tracker, cx) {
            return Err(anyhow::anyhow!("settings window could not be presented"));
        }
        cx.activate(true);
        Ok(())
    }
}

impl SettingsRuntime {
    pub fn new(
        query: impl Fn(&str) -> Result<serde_json::Value, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            query: Arc::new(query),
            action: Arc::new(|action, _| Err(format!("action `{action}` is unavailable"))),
            poll_interval: Duration::from_secs(2),
            query_intervals: std::collections::HashMap::new(),
        }
    }

    pub fn empty() -> Self {
        Self::new(|_| Ok(serde_json::Value::Null))
    }

    pub fn tray(plugin_id: impl Into<String>) -> Self {
        let plugin_id = plugin_id.into();
        let query_plugin_id = plugin_id.clone();
        Self::new(move |query| persistence::query(&query_plugin_id, query))
            .with_input_action_result(move |action, input| {
                persistence::run_action(&plugin_id, action, &input)
            })
    }

    pub fn with_input_action(
        mut self,
        action: impl Fn(&str, serde_json::Value) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.action = Arc::new(move |name, input| action(name, input).map(|()| None));
        self
    }

    pub fn with_input_action_result(
        mut self,
        action: impl Fn(&str, serde_json::Value) -> Result<Option<serde_json::Value>, String>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.action = Arc::new(action);
        self
    }

    pub fn poll_every(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn poll_query_every(mut self, name: impl Into<String>, interval: Duration) -> Self {
        self.query_intervals.insert(name.into(), interval);
        self
    }

    fn query_interval(&self, name: &str) -> Duration {
        self.query_intervals
            .get(name)
            .copied()
            .unwrap_or(self.poll_interval)
    }

    fn query(&self, name: &str) -> Result<serde_json::Value, String> {
        (self.query)(name)
    }

    fn run_action(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, String> {
        (self.action)(name, input)
    }
}

pub fn run_plugin_settings(
    plugin_id: impl Into<String>,
    heading: impl Into<String>,
    contract: impl Into<String>,
    runtime: SettingsRuntime,
) -> anyhow::Result<()> {
    run_standalone(
        SettingsPanel {
            plugin_id: plugin_id.into(),
            contract: contract.into(),
            heading: heading.into(),
        },
        runtime,
    )
}

pub fn run_standalone(panel: SettingsPanel, runtime: SettingsRuntime) -> anyhow::Result<()> {
    let failure = Rc::new(RefCell::new(None));
    let reported_failure = failure.clone();
    Application::new().run(move |cx: &mut App| {
        crate::platform::set_accessory_policy();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let tracker = MonitorTracker::start(cx);
        cx.spawn(async move |cx: &mut AsyncApp| {
            if let Err(error) = open_from_async(panel, tracker, runtime, cx).await {
                reported_failure.borrow_mut().replace(error);
                let _ = cx.update(|cx| cx.quit());
            }
        })
        .detach();
    });
    let failure = failure.borrow_mut().take();
    failure.map_or(Ok(()), Err)
}

pub async fn prepare_from_async(
    panel: SettingsPanel,
    runtime: SettingsRuntime,
    cx: &AsyncApp,
) -> anyhow::Result<PreparedSettingsPanel> {
    #[cfg(debug_assertions)]
    let plugin_id = panel.plugin_id.clone();
    #[cfg(not(debug_assertions))]
    let plugin_id = String::new();
    #[cfg(debug_assertions)]
    let started = Some(std::time::Instant::now());
    #[cfg(not(debug_assertions))]
    let started = None::<std::time::Instant>;
    let prepared = cx
        .background_spawn(async move { prepare_panel(panel, runtime) })
        .await;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=prepared outcome={} elapsed_ms={}",
        if prepared.is_ok() { "ready" } else { "failed" },
        started
            .map(|started| started.elapsed().as_millis())
            .unwrap_or(0)
    );
    prepared
}

pub async fn open_from_async(
    panel: SettingsPanel,
    tracker: MonitorTracker,
    runtime: SettingsRuntime,
    cx: &AsyncApp,
) -> anyhow::Result<()> {
    let prepared = prepare_from_async(panel, runtime, cx).await?;
    cx.update(move |cx| {
        let prepared = size_prepared_panel(prepared, &tracker)?;
        open_prepared(prepared, &tracker, cx).map(|_| ())
    })?
}

pub async fn open_plugin_settings(
    plugin_id: impl Into<String>,
    heading: impl Into<String>,
    contract: impl Into<String>,
    tracker: MonitorTracker,
    runtime: SettingsRuntime,
    cx: &AsyncApp,
) -> anyhow::Result<()> {
    open_from_async(
        SettingsPanel {
            plugin_id: plugin_id.into(),
            contract: contract.into(),
            heading: heading.into(),
        },
        tracker,
        runtime,
        cx,
    )
    .await
}

fn prepare_panel(
    panel: SettingsPanel,
    runtime: SettingsRuntime,
) -> anyhow::Result<PreparedSettingsPanel> {
    let spec = qol_config::contract::parse_spec_str(&panel.contract)
        .map_err(|error| anyhow::anyhow!("contract parse failed: {error:?}"))?;
    let path = persistence::config_path(&panel.plugin_id)?;
    let values = persistence::load_values(&panel.plugin_id, &path);
    let resolved = qol_config::normalized::resolve_config(&spec, &values)
        .map_err(|errors| anyhow::anyhow!("contract resolve failed: {errors:?}"))?;
    let rows = rows_from_resolved(&resolved);
    let sections = sections_from_resolved(&resolved, &rows);
    let daemon_port = persistence::daemon_port(&panel.plugin_id);
    Ok(PreparedSettingsPanel {
        panel,
        rows,
        sections,
        values,
        path,
        runtime,
        daemon_port,
    })
}

fn size_prepared_panel(
    prepared: PreparedSettingsPanel,
    tracker: &MonitorTracker,
) -> anyhow::Result<PreparedPanel> {
    let monitor = tracker
        .snapshot_monitor()
        .ok_or_else(|| anyhow::anyhow!("no monitor state available for the settings panel"))?;
    let available =
        monitor.bounds().size.height.to_f64() as f32 - 2.0 * crate::placement::CORNER_MARGIN;
    let height = panel_height(&prepared.rows, &prepared.sections).min(available);
    let width = panel_width(&prepared.rows);
    let body_max = available - PANEL_CHROME_HEIGHT;
    Ok(PreparedPanel {
        panel: prepared.panel,
        state: SettingsPanelState {
            rows: prepared.rows,
            sections: prepared.sections,
            values: prepared.values,
            path: prepared.path,
            body_max,
            height_cap: available,
            runtime: prepared.runtime,
            daemon_port: prepared.daemon_port,
        },
        size: size(px(width), px(height)),
    })
}

fn open_prepared(
    prepared: PreparedPanel,
    tracker: &MonitorTracker,
    cx: &mut App,
) -> anyhow::Result<ActivePanel> {
    let plugin_id = prepared.panel.plugin_id.clone();
    let title = prepared.panel.heading.clone();
    let opened = Surface::new(SurfaceKind::Panel)
        .title(title)
        .app_id(qol_conventions::SETTINGS_SURFACE_APP_ID)
        .size(prepared.size)
        .retain_on_dismiss()
        .show_focused(tracker, cx, move |dismisser, _window, cx| {
            SettingsPanelView::new(prepared.panel, prepared.state, dismisser, cx)
        })?;
    Ok(ActivePanel {
        plugin_id,
        surface: opened,
    })
}

fn activation_decision(active: Option<&str>, requested: &str) -> ActivationDecision {
    match active {
        None => ActivationDecision::Open,
        Some(active) if active == requested => ActivationDecision::Focus,
        Some(_) => ActivationDecision::Replace,
    }
}

fn section_menu_item_height(description: Option<&str>) -> f32 {
    let base = 2.0 * PANEL_SECTION_MENU_ITEM_PADDING_Y + PANEL_SECTION_MENU_LABEL_LINE_HEIGHT;
    if description.is_some() {
        base + PANEL_COLUMN_GAP + PANEL_SECTION_MENU_DESCRIPTION_LINE_HEIGHT
    } else {
        base
    }
}

fn panel_height(rows: &[Row], sections: &[RowSection]) -> f32 {
    if sections.len() > 1 {
        let items = sections
            .iter()
            .map(|section| section_menu_item_height(section.description.as_deref()))
            .sum::<f32>();
        let gaps = (sections.len() - 1) as f32 * PANEL_COLUMN_GAP;
        return PANEL_CHROME_HEIGHT + items + gaps;
    }
    let section_content = sections
        .iter()
        .map(|section| {
            let rows_height = section
                .rows
                .iter()
                .map(|index| view::row_height(&rows[*index]))
                .sum::<f32>();
            let gaps = (section.rows.len().saturating_sub(1)) as f32 * PANEL_COLUMN_GAP;
            rows_height + gaps
        })
        .fold(0.0, f32::max);
    PANEL_CHROME_HEIGHT + section_content
}

fn panel_width(rows: &[Row]) -> f32 {
    if rows
        .iter()
        .any(|row| matches!(row.control, RowControl::Gamepad { .. }))
    {
        return PANEL_GAMEPAD_WIDTH;
    }
    PANEL_WIDTH
}

#[cfg(test)]
mod tests {
    use super::{
        activation_decision, panel_height, panel_width, section_menu_item_height,
        ActivationDecision, Row, RowSection, PANEL_CHROME_HEIGHT, PANEL_GAMEPAD_WIDTH,
        PANEL_ROW_HEIGHT,
    };
    use crate::gamepad::GamepadMonitor;
    use crate::settings_panel::rows::RowControl;

    fn row(control: RowControl) -> Row {
        Row {
            id: "field".into(),
            section_id: None,
            section_label: None,
            label: "Label".into(),
            description: None,
            placeholder: None,
            variant: None,
            config_key: "key".into(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            control,
        }
    }

    #[test]
    fn overlay_controls_do_not_change_parent_panel_height() {
        let toggle = vec![row(RowControl::Toggle(false))];
        let color = vec![row(RowControl::Color("#ffffff".into()))];
        let expected = PANEL_CHROME_HEIGHT + PANEL_ROW_HEIGHT;

        let sections = vec![RowSection {
            label: "General".into(),
            description: None,
            rows: vec![0],
        }];
        assert_eq!(panel_height(&toggle, &sections), expected);
        assert_eq!(panel_height(&color, &sections), expected);
    }

    #[test]
    fn gamepad_fields_expand_only_their_shared_settings_surface() {
        let regular = vec![row(RowControl::Toggle(false))];
        let gamepad = vec![row(RowControl::Gamepad {
            query: "controller_input".into(),
            monitor: GamepadMonitor::default(),
        })];

        assert_eq!(panel_width(&regular), super::PANEL_WIDTH);
        assert_eq!(panel_width(&gamepad), PANEL_GAMEPAD_WIDTH);
    }

    #[test]
    fn section_menu_items_grow_for_descriptions() {
        assert_eq!(section_menu_item_height(None), 39.0);
        assert_eq!(section_menu_item_height(Some("desc")), 62.0);
    }

    #[test]
    fn sectioned_panels_size_for_the_section_menu() {
        let rows = vec![
            row(RowControl::Toggle(false)),
            row(RowControl::Toggle(false)),
            row(RowControl::Toggle(false)),
        ];
        let plain = |label: &str, rows: Vec<usize>| RowSection {
            label: label.into(),
            description: None,
            rows,
        };
        let sections = vec![
            plain("One", vec![0]),
            plain("Two", vec![1]),
            RowSection {
                label: "Three".into(),
                description: Some("described".into()),
                rows: vec![2],
            },
        ];

        assert_eq!(
            panel_height(&rows, &sections),
            PANEL_CHROME_HEIGHT + 39.0 + 39.0 + 62.0 + 2.0 * 4.0
        );
    }

    #[test]
    fn query_intervals_override_only_their_declared_query() {
        let runtime = super::SettingsRuntime::empty()
            .poll_every(std::time::Duration::from_secs(3))
            .poll_query_every("controller_input", std::time::Duration::from_millis(16));

        assert_eq!(
            runtime.query_interval("controller_input"),
            std::time::Duration::from_millis(16)
        );
        assert_eq!(
            runtime.query_interval("controllers_snapshot"),
            std::time::Duration::from_secs(3)
        );
    }

    #[test]
    fn activation_reuses_the_single_panel_window() {
        let cases = [
            (None, "plugin-a", ActivationDecision::Open),
            (Some("plugin-a"), "plugin-a", ActivationDecision::Focus),
            (Some("plugin-a"), "plugin-b", ActivationDecision::Replace),
        ];
        for (active, requested, expected) in cases {
            assert_eq!(
                activation_decision(active, requested),
                expected,
                "active={active:?} requested={requested}"
            );
        }
    }
}
