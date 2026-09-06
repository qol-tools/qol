mod navigation;
mod object_array_row;
mod persistence;
mod rows;
mod stream;
mod view;

pub mod components;
pub use components::{settings_action_spinner, settings_busy_message, settings_query_spinner};
pub use navigation::{CustomPanelInvalidator, CustomSettingsBreadcrumbs, SettingsDestination};

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::surface::{OpenedSurface, Surface, SurfaceDismisser, SurfaceKind};
use rows::{rows_from_resolved, sections_from_resolved, Row, RowControl, RowSection};
use view::{SettingsPanelState, SettingsPanelView};

const PANEL_COMPACT_WIDTH: f32 = 420.0;
const PANEL_WIDTH: f32 = 520.0;
const PANEL_WIDE_WIDTH: f32 = 760.0;
const PANEL_GAMEPAD_WIDTH: f32 = 860.0;
const PANEL_WIDE_DESCRIPTION_CHARS: usize = 90;
const PANEL_RAIL_WIDTH: f32 = 196.0;
const PANEL_RAIL_ITEM_HEIGHT: f32 = qol_theme::HEIGHT_CONTROL;
const PANEL_ROW_HEIGHT: f32 = qol_theme::HEIGHT_SETTING_ROW;
const PANEL_LIST_HEADER_HEIGHT: f32 = 24.0;
const PANEL_LIST_DESCRIPTION_HEIGHT: f32 = 20.0;
const PANEL_LIST_ITEM_HEIGHT: f32 = qol_theme::HEIGHT_RULE_ROW;
const PANEL_LIST_GAP: f32 = qol_theme::SPACE_TIGHT;
const PANEL_LIST_PADDING_Y: f32 = qol_theme::SPACE_INSET;
const PANEL_QR_CODE_HEIGHT: f32 = 180.0;
const PANEL_QR_URL_HEIGHT: f32 = 20.0;
const PANEL_SECTION_HEADER_HEIGHT: f32 = 26.0;
const PANEL_COLUMN_GAP: f32 = qol_theme::SPACE_TIGHT;
const PANEL_BAND_HEIGHT: f32 = qol_theme::HEIGHT_BAND;
const PANEL_GROUP_HEADER_HEIGHT: f32 = qol_theme::HEIGHT_CONTROL;
const PANEL_HINT_BAR_HEIGHT: f32 = qol_theme::HEIGHT_HINT_BAR;
const PANEL_FILTER_HEIGHT: f32 = qol_theme::HEIGHT_CONTROL;
const PANEL_MAX_HEIGHT: f32 = 720.0;
const PANEL_CUSTOM_WIDTH: f32 = 560.0;
const PANEL_CUSTOM_HEIGHT: f32 = PANEL_MAX_HEIGHT;

fn chrome_height(_sections: &[RowSection]) -> f32 {
    PANEL_BAND_HEIGHT + PANEL_HINT_BAR_HEIGHT
}
const PANEL_GAMEPAD_HEIGHT: f32 = 650.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelSourceGroup {
    Core,
    Plugin,
}

#[derive(Clone)]
pub struct PanelSource {
    pub plugin_id: String,
    pub contract: String,
    pub heading: String,
    pub group: PanelSourceGroup,
    pub custom: bool,
}

#[derive(Clone)]
pub struct SettingsPanel {
    pub sources: Vec<PanelSource>,
    pub heading: String,
    pub focus: Option<String>,
}

impl SettingsPanel {
    pub fn single(plugin_id: String, contract: String, heading: String) -> SettingsPanel {
        let group = if plugin_id == qol_conventions::CORE_PANEL_ID {
            PanelSourceGroup::Core
        } else {
            PanelSourceGroup::Plugin
        };
        SettingsPanel {
            sources: vec![PanelSource {
                plugin_id,
                contract,
                heading: heading.clone(),
                group,
                custom: false,
            }],
            heading,
            focus: None,
        }
    }

    fn focused_index(&self) -> usize {
        self.sources
            .iter()
            .position(|source| self.focus.as_deref() == Some(source.plugin_id.as_str()))
            .unwrap_or(0)
    }

    pub fn primary_plugin_id(&self) -> &str {
        self.sources
            .first()
            .map(|source| source.plugin_id.as_str())
            .unwrap_or_default()
    }
}

pub type CustomPanelCallback = Rc<dyn Fn(&mut Window, &mut App)>;

type CustomBreadcrumbReader = Rc<dyn Fn(&App) -> Vec<SettingsDestination>>;

pub struct CustomPanelView {
    view: AnyView,
    focus_handle: gpui::FocusHandle,
    breadcrumbs: CustomBreadcrumbReader,
    _observation: gpui::Subscription,
}

impl CustomPanelView {
    pub fn new<T>(
        entity: gpui::Entity<T>,
        on_change: CustomPanelInvalidator,
        cx: &mut gpui::App,
    ) -> Self
    where
        T: gpui::Render + gpui::Focusable + CustomSettingsBreadcrumbs + 'static,
    {
        let focus_handle = entity.read(cx).focus_handle(cx);
        let observed = entity.downgrade();
        let breadcrumbs: CustomBreadcrumbReader = Rc::new(move |cx| {
            observed
                .upgrade()
                .map(|entity| entity.read(cx).settings_breadcrumbs())
                .unwrap_or_default()
        });
        let observation = cx.observe(&entity, move |_, cx| on_change(cx));
        Self {
            view: entity.into(),
            focus_handle,
            breadcrumbs,
            _observation: observation,
        }
    }

    fn breadcrumb_labels(&self, cx: &App) -> Vec<String> {
        (self.breadcrumbs)(cx)
            .iter()
            .map(|destination| destination.label().to_string())
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CustomPanelNoticeTone {
    Success,
    Failure,
}

pub type CustomPanelNotifier = Rc<dyn Fn(CustomPanelNoticeTone, String, &mut App)>;

pub struct CustomPanelContext {
    pub dismisser: SurfaceDismisser,
    pub on_back: CustomPanelCallback,
    pub notify: CustomPanelNotifier,
    pub on_change: CustomPanelInvalidator,
}

pub type CustomPanelFactory = Rc<dyn Fn(CustomPanelContext, &mut App) -> CustomPanelView>;

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
    notify: Option<CustomPanelNotifier>,
}

impl SettingsWindowHost {
    pub fn set_custom_notifier(&mut self, notify: CustomPanelNotifier) {
        self.notify = Some(notify);
    }

    fn notifier(&self) -> CustomPanelNotifier {
        self.notify.clone().unwrap_or_else(|| Rc::new(|_, _, _| {}))
    }
}

struct ActivePanel {
    plugin_id: String,
    surface: OpenedSurface<SettingsPanelView>,
}

pub struct PreparedSettingsPanel {
    panel: SettingsPanel,
    subtitle: Option<String>,
    rows: Vec<Row>,
    sections: Vec<RowSection>,
    sources: Vec<SourceState>,
}

pub(super) struct SourceState {
    pub(super) plugin_id: String,
    pub(super) values: serde_json::Value,
    pub(super) path: Option<std::path::PathBuf>,
    pub(super) runtime: SettingsRuntime,
    pub(super) daemon_port: Option<u16>,
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
    pub fn hide_active(&mut self, cx: &mut App) {
        if let Some(active) = self.active.as_ref() {
            active.surface.dismisser.dismiss(cx);
        }
    }

    pub fn present_active(
        &mut self,
        plugin_id: &str,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> bool {
        if !self.active_is_open(cx) {
            return false;
        }
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let retargeted = active
            .surface
            .handle
            .update(cx, |root, window, cx| {
                root.inner
                    .update(cx, |view, cx| view.retarget_focus(plugin_id, window, cx))
            })
            .ok()
            .unwrap_or(false);
        if retargeted {
            return self.present(tracker, cx);
        }
        if active.plugin_id != plugin_id {
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
        self.activate_prepared_with_custom(prepared, Vec::new(), tracker, cx)
    }

    pub fn activate_prepared_with_custom(
        &mut self,
        prepared: PreparedSettingsPanel,
        custom_factories: Vec<(String, CustomPanelFactory)>,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> anyhow::Result<SettingsActivation> {
        self.activate_prepared_with_custom_mode(prepared, custom_factories, false, tracker, cx)
    }

    pub fn activate_prepared_with_custom_force(
        &mut self,
        prepared: PreparedSettingsPanel,
        custom_factories: Vec<(String, CustomPanelFactory)>,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> anyhow::Result<SettingsActivation> {
        self.activate_prepared_with_custom_mode(prepared, custom_factories, true, tracker, cx)
    }

    fn activate_prepared_with_custom_mode(
        &mut self,
        prepared: PreparedSettingsPanel,
        custom_factories: Vec<(String, CustomPanelFactory)>,
        force_replace: bool,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> anyhow::Result<SettingsActivation> {
        let plugin_id = prepared.panel.primary_plugin_id().to_string();
        let requested_focus = prepared.panel.focus.clone();
        let decision = activation_decision(
            self.active.as_ref().map(|active| active.plugin_id.as_str()),
            &plugin_id,
        );
        let can_focus = if !force_replace && decision == ActivationDecision::Focus {
            requested_focus
                .as_deref()
                .is_none_or(|focus| self.retarget_active_source(focus, cx))
        } else {
            false
        };
        if can_focus && self.present(tracker, cx) {
            return Ok(SettingsActivation::Focused);
        }

        let prepared = size_prepared_panel(prepared, tracker)?;
        if (force_replace || decision == ActivationDecision::Replace || !can_focus)
            && self.active_is_open(cx)
        {
            self.replace(prepared, custom_factories, tracker, cx)?;
            return Ok(SettingsActivation::Replaced);
        }

        self.active = Some(open_prepared(
            prepared,
            custom_factories,
            self.notifier(),
            tracker,
            cx,
        )?);
        Ok(SettingsActivation::Opened)
    }

    fn retarget_active_source(&mut self, plugin_id: &str, cx: &mut App) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        active
            .surface
            .handle
            .update(cx, |root, window, cx| {
                root.inner
                    .update(cx, |view, cx| view.retarget_focus(plugin_id, window, cx))
            })
            .ok()
            .unwrap_or(false)
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
        custom_factories: Vec<(String, CustomPanelFactory)>,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let notify = self.notifier();
        let Some(active) = self.active.as_mut() else {
            return Err(anyhow::anyhow!(
                "settings window disappeared before replacement"
            ));
        };
        let plugin_id = prepared.panel.primary_plugin_id().to_string();
        let heading = prepared.panel.heading.clone();
        let dismisser = active.surface.dismisser.clone();
        let visible = active.surface.is_visible();
        active.surface.resize(prepared.size, cx)?;
        active.surface.handle.update(cx, move |root, window, cx| {
            root.inner.update(cx, |view, _| view.pause_runtime_poll());
            dismisser.retitle(window, heading);
            let inner = cx.new(|cx| {
                SettingsPanelView::new(
                    prepared.panel,
                    prepared.state,
                    dismisser,
                    custom_factories,
                    notify,
                    cx,
                )
            });
            root.inner = inner;
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
        Self::tray_base(persistence::panel_base(&plugin_id))
    }

    pub fn tray_core() -> Self {
        Self::tray_base(persistence::panel_base(qol_conventions::CORE_PANEL_ID))
    }

    fn tray_base(base: String) -> Self {
        let query_base = base.clone();
        Self::new(move |query| persistence::query(&query_base, query)).with_input_action_result(
            move |action, input| persistence::run_action(&base, action, &input),
        )
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

pub async fn prepare_from_async(
    panel: SettingsPanel,
    runtime: SettingsRuntime,
    cx: &AsyncApp,
) -> anyhow::Result<PreparedSettingsPanel> {
    prepare_many_from_async(panel, vec![runtime], cx).await
}

pub async fn prepare_many_from_async(
    panel: SettingsPanel,
    runtimes: Vec<SettingsRuntime>,
    cx: &AsyncApp,
) -> anyhow::Result<PreparedSettingsPanel> {
    #[cfg(debug_assertions)]
    let plugin_id = panel.primary_plugin_id().to_string();
    #[cfg(not(debug_assertions))]
    let plugin_id = String::new();
    #[cfg(debug_assertions)]
    let started = Some(std::time::Instant::now());
    #[cfg(not(debug_assertions))]
    let started = None::<std::time::Instant>;
    let prepared = cx
        .background_spawn(async move { prepare_panel(panel, runtimes) })
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
        open_prepared(prepared, Vec::new(), Rc::new(|_, _, _| {}), &tracker, cx).map(|_| ())
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
        SettingsPanel::single(plugin_id.into(), contract.into(), heading.into()),
        tracker,
        runtime,
        cx,
    )
    .await
}

fn prepare_panel(
    panel: SettingsPanel,
    runtimes: Vec<SettingsRuntime>,
) -> anyhow::Result<PreparedSettingsPanel> {
    let focused_index = panel.focused_index();
    let mut subtitle = None;
    let mut prepared_sources = Vec::new();
    for (source_index, (source, runtime)) in panel.sources.iter().zip(runtimes).enumerate() {
        match prepare_source(source, runtime) {
            Ok(prepared) => {
                if source_index == focused_index {
                    subtitle = prepared.description.clone();
                }
                prepared_sources.push(prepared);
            }
            Err(error) if panel.sources.len() > 1 => {
                eprintln!(
                    "[settings] skipping broken source {}: {error:#}",
                    source.plugin_id
                );
            }
            Err(error) => return Err(error),
        }
    }
    let mut rows = Vec::new();
    let mut sections = Vec::new();
    let mut sources = Vec::new();
    for (source_index, prepared) in prepared_sources.into_iter().enumerate() {
        let base_row = rows.len();
        for mut row in prepared.rows {
            row.source = source_index;
            rows.push(row);
        }
        for mut section in prepared.sections {
            section.source = source_index;
            for index in &mut section.rows {
                *index += base_row;
            }
            sections.push(section);
        }
        sources.push(prepared.state);
    }
    Ok(PreparedSettingsPanel {
        panel,
        subtitle,
        rows,
        sections,
        sources,
    })
}

struct PreparedSource {
    rows: Vec<Row>,
    sections: Vec<RowSection>,
    description: Option<String>,
    state: SourceState,
}

fn prepare_source(
    source: &PanelSource,
    runtime: SettingsRuntime,
) -> anyhow::Result<PreparedSource> {
    if source.custom {
        return Ok(PreparedSource {
            rows: Vec::new(),
            sections: Vec::new(),
            description: None,
            state: SourceState {
                plugin_id: source.plugin_id.clone(),
                values: serde_json::json!({}),
                path: None,
                runtime,
                daemon_port: None,
            },
        });
    }
    let spec = qol_config::contract::parse_spec_str(&source.contract)
        .map_err(|error| anyhow::anyhow!("contract parse failed: {error:?}"))?;
    let base = persistence::panel_base(&source.plugin_id);
    let path = if source.plugin_id == qol_conventions::CORE_PANEL_ID {
        None
    } else {
        Some(persistence::config_path(&source.plugin_id)?)
    };
    let values = persistence::load_values(&base, path.as_deref());
    let resolved = qol_config::normalized::resolve_config(&spec, &values)
        .map_err(|errors| anyhow::anyhow!("contract resolve failed: {errors:?}"))?;
    let daemon_port = persistence::daemon_port(&base);
    let rows = rows_from_resolved(&resolved, 0);
    let sections = sections_from_resolved(&resolved, &rows, 0);
    Ok(PreparedSource {
        description: resolved.description.clone(),
        rows,
        sections,
        state: SourceState {
            plugin_id: source.plugin_id.clone(),
            values,
            path,
            runtime,
            daemon_port,
        },
    })
}

fn size_prepared_panel(
    prepared: PreparedSettingsPanel,
    tracker: &MonitorTracker,
) -> anyhow::Result<PreparedPanel> {
    let monitor = tracker
        .snapshot_monitor()
        .ok_or_else(|| anyhow::anyhow!("no monitor state available for the settings panel"))?;
    let available = (monitor.bounds().size.height.to_f64() as f32
        - 2.0 * crate::placement::CORNER_MARGIN)
        .min(PANEL_MAX_HEIGHT);
    let has_custom_source = prepared.panel.sources.iter().any(|source| source.custom);
    let body_width = panel_width(&prepared.rows).max(if has_custom_source {
        PANEL_CUSTOM_WIDTH
    } else {
        0.0
    });
    let width = body_width + rail_width(prepared.sources.len());
    let height = panel_height(&prepared.rows, &prepared.sections)
        .max(if has_custom_source {
            PANEL_CUSTOM_HEIGHT
        } else {
            0.0
        })
        .min(available);
    Ok(PreparedPanel {
        panel: prepared.panel,
        state: SettingsPanelState {
            subtitle: prepared.subtitle,
            rows: prepared.rows,
            sections: prepared.sections,
            sources: prepared.sources,
            height_cap: available,
        },
        size: size(px(width), px(height)),
    })
}

fn open_prepared(
    prepared: PreparedPanel,
    custom_factories: Vec<(String, CustomPanelFactory)>,
    notify: CustomPanelNotifier,
    tracker: &MonitorTracker,
    cx: &mut App,
) -> anyhow::Result<ActivePanel> {
    let plugin_id = prepared.panel.primary_plugin_id().to_string();
    let title = prepared.panel.heading.clone();
    let opened = Surface::new(SurfaceKind::Panel)
        .title(title)
        .app_id(qol_conventions::SETTINGS_SURFACE_APP_ID)
        .size(prepared.size)
        .retain_on_dismiss()
        .show_focused(tracker, cx, move |dismisser, _window, cx| {
            SettingsPanelView::new(
                prepared.panel,
                prepared.state,
                dismisser,
                custom_factories,
                notify,
                cx,
            )
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

fn panel_height(rows: &[Row], sections: &[RowSection]) -> f32 {
    let show_section_headers = sections.len() <= 1;
    let body = sections
        .iter()
        .map(|section| {
            let visible = section
                .rows
                .iter()
                .copied()
                .filter(|index| rows::row_is_visible(rows, *index))
                .collect::<Vec<_>>();
            if visible.is_empty() {
                return 0.0;
            }
            let rows_height = visible
                .iter()
                .map(|index| view::row_height(&rows[*index], show_section_headers))
                .sum::<f32>();
            let gaps = (visible.len().saturating_sub(1)) as f32 * PANEL_COLUMN_GAP;
            PANEL_GROUP_HEADER_HEIGHT + rows_height + gaps
        })
        .sum::<f32>();
    chrome_height(sections) + body
}

/// Sized against the same predicate the view opens the rail with, so a panel
/// never reserves rail width the body will not show. Sources that failed to
/// prepare are already gone from `source_count`, and a panel opens unfiltered.
fn rail_width(source_count: usize) -> f32 {
    if view::rail_open(source_count, false) {
        PANEL_RAIL_WIDTH
    } else {
        0.0
    }
}

fn panel_width(rows: &[Row]) -> f32 {
    if rows
        .iter()
        .any(|row| matches!(row.control, RowControl::Gamepad { .. }))
    {
        return PANEL_GAMEPAD_WIDTH;
    }
    if rows.iter().any(row_wants_a_wide_panel) {
        return PANEL_WIDE_WIDTH;
    }
    if rows.iter().all(row_fits_a_compact_panel) {
        return PANEL_COMPACT_WIDTH;
    }
    PANEL_WIDTH
}

fn row_wants_a_wide_panel(row: &Row) -> bool {
    if matches!(
        row.control,
        RowControl::TextList(_)
            | RowControl::ObjectArray(_)
            | RowControl::List { .. }
            | RowControl::QrCode { .. }
    ) {
        return true;
    }
    row.description
        .as_deref()
        .is_some_and(|text| text.chars().count() > PANEL_WIDE_DESCRIPTION_CHARS)
}

fn row_fits_a_compact_panel(row: &Row) -> bool {
    row.description.is_none()
        && matches!(
            row.control,
            RowControl::Toggle(_) | RowControl::Number { .. } | RowControl::Select { .. }
        )
}

#[cfg(test)]
mod tests {
    use super::{
        activation_decision, panel_height, panel_width, rail_width, ActivationDecision,
        PanelSource, PanelSourceGroup, Row, RowSection, SettingsPanel, SettingsRuntime,
        PANEL_GAMEPAD_WIDTH, PANEL_ROW_HEIGHT, PANEL_WIDTH,
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
            source: 0,
        }
    }

    #[test]
    fn overlay_controls_do_not_change_parent_panel_height() {
        let toggle = vec![row(RowControl::Toggle(false))];
        let color = vec![row(RowControl::Color("#ffffff".into()))];
        let sections = vec![RowSection {
            label: "General".into(),
            description: None,
            rows: vec![0],
            source: 0,
        }];
        let expected =
            super::chrome_height(&sections) + super::PANEL_GROUP_HEADER_HEIGHT + PANEL_ROW_HEIGHT;
        assert_eq!(panel_height(&toggle, &sections), expected);
        assert_eq!(panel_height(&color, &sections), expected);
    }

    #[test]
    fn the_filter_overlay_never_changes_the_window_height() {
        let rows = vec![row(RowControl::Toggle(false))];
        let sections = vec![RowSection {
            label: "General".into(),
            description: None,
            rows: vec![0],
            source: 0,
        }];
        assert_eq!(
            panel_height(&rows, &sections),
            super::PANEL_BAND_HEIGHT
                + super::PANEL_HINT_BAR_HEIGHT
                + super::PANEL_GROUP_HEADER_HEIGHT
                + PANEL_ROW_HEIGHT
        );
        assert_eq!(
            super::chrome_height(&sections),
            super::PANEL_BAND_HEIGHT + super::PANEL_HINT_BAR_HEIGHT,
            "the chrome must not carry a filter row, so the window height cannot differ between filter states"
        );
    }

    #[test]
    fn gamepad_fields_expand_only_their_shared_settings_surface() {
        let regular = vec![row(RowControl::Toggle(false))];
        let gamepad = vec![row(RowControl::Gamepad {
            query: "controller_input".into(),
            monitor: GamepadMonitor::default(),
        })];

        assert_eq!(panel_width(&regular), super::PANEL_COMPACT_WIDTH);
        assert_eq!(panel_width(&gamepad), PANEL_GAMEPAD_WIDTH);
    }

    #[test]
    fn gamepad_rows_no_longer_resize_the_window() {
        let plain = vec![row(RowControl::Toggle(false))];
        let gamepad = vec![row(RowControl::Gamepad {
            query: "controller_input".into(),
            monitor: GamepadMonitor::default(),
        })];
        let sections = vec![RowSection {
            label: "General".into(),
            description: None,
            rows: vec![0],
            source: 0,
        }];
        assert_eq!(
            panel_height(&gamepad, &sections),
            panel_height(&plain, &sections)
        );
    }

    #[test]
    fn panel_width_steps_up_for_the_content_it_has_to_hold() {
        let bare = vec![row(RowControl::Toggle(false))];
        let mut described = row(RowControl::Toggle(false));
        described.description = Some("a short one".into());
        let mut verbose = row(RowControl::Toggle(false));
        verbose.description = Some("x".repeat(super::PANEL_WIDE_DESCRIPTION_CHARS + 1));
        let listy = vec![row(RowControl::TextList(Vec::new()))];

        assert_eq!(panel_width(&bare), super::PANEL_COMPACT_WIDTH);
        assert_eq!(panel_width(&[described]), PANEL_WIDTH);
        assert_eq!(panel_width(&[verbose]), super::PANEL_WIDE_WIDTH);
        assert_eq!(panel_width(&listy), super::PANEL_WIDE_WIDTH);
    }

    #[test]
    fn every_section_stacks_onto_one_page() {
        let rows = vec![
            row(RowControl::Toggle(false)),
            row(RowControl::Toggle(false)),
            row(RowControl::Toggle(false)),
        ];
        let plain = |label: &str, rows: Vec<usize>| RowSection {
            label: label.into(),
            description: None,
            rows,
            source: 0,
        };
        let sections = vec![
            plain("One", vec![0]),
            plain("Two", vec![1]),
            plain("Three", vec![2]),
        ];

        assert_eq!(
            panel_height(&rows, &sections),
            super::chrome_height(&sections)
                + 3.0 * (super::PANEL_GROUP_HEADER_HEIGHT + PANEL_ROW_HEIGHT)
        );
        assert_eq!(rail_width(2), super::PANEL_RAIL_WIDTH);
        assert_eq!(rail_width(1), 0.0);
    }

    #[test]
    fn a_section_with_no_visible_rows_takes_no_room() {
        let rows = vec![row(RowControl::Toggle(false))];
        let plain = |label: &str, rows: Vec<usize>| RowSection {
            label: label.into(),
            description: None,
            rows,
            source: 0,
        };
        let sections = vec![plain("One", vec![0]), plain("Empty", Vec::new())];

        assert_eq!(
            panel_height(&rows, &sections),
            super::chrome_height(&sections) + super::PANEL_GROUP_HEADER_HEIGHT + PANEL_ROW_HEIGHT
        );
    }

    #[test]
    fn stacked_sections_each_carry_their_own_header() {
        let rows = (0..12)
            .map(|_| row(RowControl::Toggle(false)))
            .collect::<Vec<_>>();
        let plain = |label: &str, rows: Vec<usize>| RowSection {
            label: label.into(),
            description: None,
            rows,
            source: 0,
        };
        let sections = vec![
            plain("One", vec![0]),
            plain("Two", (1..12).collect::<Vec<_>>()),
        ];

        let first = super::PANEL_GROUP_HEADER_HEIGHT + PANEL_ROW_HEIGHT;
        let second = super::PANEL_GROUP_HEADER_HEIGHT + 11.0 * PANEL_ROW_HEIGHT + 10.0 * 4.0;
        assert_eq!(
            panel_height(&rows, &sections),
            super::chrome_height(&sections) + first + second
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

    #[test]
    fn custom_sources_do_not_require_a_settings_contract() {
        let source = PanelSource {
            plugin_id: "__core-shortcuts".into(),
            contract: String::new(),
            heading: "Shortcuts".into(),
            group: PanelSourceGroup::Core,
            custom: true,
        };
        let prepared = super::prepare_source(&source, SettingsRuntime::empty()).unwrap();

        assert!(prepared.rows.is_empty());
        assert!(prepared.sections.is_empty());
        assert_eq!(prepared.state.plugin_id, "__core-shortcuts");
    }

    #[test]
    fn a_skipped_source_reserves_no_rail_width() {
        let panel = SettingsPanel {
            sources: vec![
                PanelSource {
                    plugin_id: "__core-shortcuts".into(),
                    contract: String::new(),
                    heading: "Shortcuts".into(),
                    group: PanelSourceGroup::Core,
                    custom: true,
                },
                PanelSource {
                    plugin_id: "plugin-broken".into(),
                    contract: "this is not a contract".into(),
                    heading: "Broken".into(),
                    group: PanelSourceGroup::Plugin,
                    custom: false,
                },
            ],
            heading: "Settings".into(),
            focus: None,
        };

        let prepared = super::prepare_panel(
            panel,
            vec![SettingsRuntime::empty(), SettingsRuntime::empty()],
        )
        .expect("a broken source is skipped, not fatal");

        assert_eq!(prepared.sources.len(), 1, "the broken source is dropped");
        assert_eq!(
            rail_width(prepared.sources.len()),
            0.0,
            "sizing must not reserve a rail the body will not open"
        );
    }

    #[test]
    fn settings_body_scroll_region_lets_content_exceed_viewport() {
        use taffy::geometry::Point;
        use taffy::style::{Dimension, LengthPercentage, Overflow};
        use taffy::{AvailableSpace, FlexDirection, NodeId, TaffyTree};

        fn dim(w: f32, h: f32) -> taffy::geometry::Size<Dimension> {
            taffy::geometry::Size {
                width: Dimension::length(w),
                height: Dimension::length(h),
            }
        }

        fn wide(h: f32) -> taffy::geometry::Size<Dimension> {
            taffy::geometry::Size {
                width: Dimension::percent(1.0),
                height: Dimension::length(h),
            }
        }

        fn gap4() -> taffy::geometry::Size<LengthPercentage> {
            taffy::geometry::Size {
                width: LengthPercentage::length(4.0),
                height: LengthPercentage::length(4.0),
            }
        }

        fn layout(rows_in_flex_viewport: bool) -> f32 {
            let mut tree: TaffyTree<()> = TaffyTree::new();
            let body = tree
                .new_leaf(if rows_in_flex_viewport {
                    taffy::style::Style {
                        size: wide(400.0),
                        overflow: Point {
                            x: Overflow::Visible,
                            y: Overflow::Scroll,
                        },
                        flex_direction: FlexDirection::Column,
                        gap: gap4(),
                        ..Default::default()
                    }
                } else {
                    taffy::style::Style {
                        display: taffy::style::Display::Block,
                        size: wide(400.0),
                        overflow: Point {
                            x: Overflow::Visible,
                            y: Overflow::Scroll,
                        },
                        ..Default::default()
                    }
                })
                .unwrap();

            let row_nodes: Vec<NodeId> = (0..12)
                .map(|_| {
                    tree.new_leaf(taffy::style::Style {
                        size: dim(500.0, 40.0),
                        ..Default::default()
                    })
                    .unwrap()
                })
                .collect();

            let list: Vec<NodeId> = if rows_in_flex_viewport {
                row_nodes
            } else {
                let wrapper = tree
                    .new_leaf(taffy::style::Style {
                        flex_direction: FlexDirection::Column,
                        gap: gap4(),
                        ..Default::default()
                    })
                    .unwrap();
                tree.set_children(wrapper, &row_nodes).unwrap();
                vec![wrapper]
            };
            tree.set_children(body, &list).unwrap();
            let root = tree
                .new_leaf(taffy::style::Style {
                    overflow: Point {
                        x: Overflow::Hidden,
                        y: Overflow::Hidden,
                    },
                    ..Default::default()
                })
                .unwrap();
            tree.set_children(root, &[body]).unwrap();
            tree.compute_layout(
                root,
                taffy::geometry::Size {
                    width: AvailableSpace::Definite(500.0),
                    height: AvailableSpace::Definite(500.0),
                },
            )
            .unwrap();
            let layout = tree.layout(body).unwrap();
            layout.content_size.height - layout.size.height
        }

        let direct_overflow = layout(true);
        let wrapped_overflow = layout(false);
        assert_eq!(
            direct_overflow, 0.0,
            "rows attached to a flex_col scroll viewport shrink to fit and lose all scroll room"
        );
        assert!(
            wrapped_overflow > 0.0,
            "a block scroll viewport over a flex_col wrapper must keep scroll room"
        );
    }
}

#[cfg(test)]
mod design_conformance {
    use super::{PANEL_RAIL_WIDTH, PANEL_WIDE_WIDTH};

    #[test]
    fn panel_wide_width_matches_deck() {
        assert_eq!(PANEL_WIDE_WIDTH, 760.0, "deck settings mock is w760");
    }

    #[test]
    fn rail_width_matches_deck() {
        assert_eq!(PANEL_RAIL_WIDTH, 196.0, "deck rail is 196");
    }
}
