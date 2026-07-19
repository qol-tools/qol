mod color_wheel;
mod persistence;
mod rows;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::surface::{Anchor, Surface, SurfaceKind};
use rows::{rows_from_resolved, Row};
use view::{SettingsPanelState, SettingsPanelView};

const PANEL_WIDTH: f32 = 520.0;
const PANEL_ROW_HEIGHT: f32 = 36.0;
const PANEL_LIST_HEADER_HEIGHT: f32 = 24.0;
const PANEL_LIST_ITEM_HEIGHT: f32 = 48.0;
const PANEL_LIST_GAP: f32 = 4.0;
const PANEL_LIST_PADDING_Y: f32 = 8.0;
const PANEL_LIST_HEIGHT: f32 = PANEL_LIST_PADDING_Y
    + PANEL_LIST_HEADER_HEIGHT
    + rows::LIST_MAX_VISIBLE as f32 * (PANEL_LIST_ITEM_HEIGHT + PANEL_LIST_GAP);
const PANEL_SECTION_HEADER_HEIGHT: f32 = 26.0;
const PANEL_CHROME_HEIGHT: f32 = 72.0;

#[derive(Clone, Copy)]
pub struct SettingsPanel {
    pub plugin_id: &'static str,
    pub contract: &'static str,
    pub heading: &'static str,
}

type QueryHandler = dyn Fn(&str) -> Result<serde_json::Value, String> + Send + Sync;
type ActionHandler = dyn Fn(&str, serde_json::Value) -> Result<(), String> + Send + Sync;

#[derive(Clone)]
pub struct SettingsRuntime {
    query: Arc<QueryHandler>,
    action: Arc<ActionHandler>,
    poll_interval: Duration,
}

impl SettingsRuntime {
    pub fn new(
        query: impl Fn(&str) -> Result<serde_json::Value, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            query: Arc::new(query),
            action: Arc::new(|action, _| Err(format!("action `{action}` is unavailable"))),
            poll_interval: Duration::from_secs(2),
        }
    }

    pub fn empty() -> Self {
        Self::new(|_| Ok(serde_json::Value::Null))
    }

    pub fn with_action(
        mut self,
        action: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.action = Arc::new(move |name, _| action(name));
        self
    }

    pub fn with_input_action(
        mut self,
        action: impl Fn(&str, serde_json::Value) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.action = Arc::new(action);
        self
    }

    pub fn poll_every(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    fn query(&self, name: &str) -> Result<serde_json::Value, String> {
        (self.query)(name)
    }

    fn run_action(&self, name: &str, input: serde_json::Value) -> Result<(), String> {
        (self.action)(name, input)
    }
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
        if let Err(error) = open(panel, &tracker, runtime.clone(), cx) {
            reported_failure.borrow_mut().replace(error);
            cx.quit();
        }
    });
    let failure = failure.borrow_mut().take();
    failure.map_or(Ok(()), Err)
}

pub fn open_from_async(
    panel: SettingsPanel,
    tracker: MonitorTracker,
    runtime: SettingsRuntime,
    cx: &AsyncApp,
) -> anyhow::Result<()> {
    cx.update(move |cx| open(panel, &tracker, runtime, cx))?
}

pub fn open(
    panel: SettingsPanel,
    tracker: &MonitorTracker,
    runtime: SettingsRuntime,
    cx: &mut App,
) -> anyhow::Result<()> {
    let spec = qol_config::contract::parse_spec_str(panel.contract)
        .map_err(|error| anyhow::anyhow!("contract parse failed: {error:?}"))?;
    let path = persistence::config_path(panel.plugin_id)?;
    let values = persistence::load_values(panel.plugin_id, &path);
    let resolved = qol_config::normalized::resolve_config(&spec, &values)
        .map_err(|errors| anyhow::anyhow!("contract resolve failed: {errors:?}"))?;
    let rows = rows_from_resolved(&resolved, &runtime);
    let monitor = tracker
        .snapshot_monitor()
        .ok_or_else(|| anyhow::anyhow!("no monitor state available for the settings panel"))?;
    let available =
        monitor.bounds().size.height.to_f64() as f32 - 2.0 * crate::surface::CORNER_MARGIN;
    let height = panel_height(&rows).min(available);
    let body_max = height - PANEL_CHROME_HEIGHT;
    let title = format!("{}-settings-{}", panel.plugin_id, std::process::id());
    Surface::new(SurfaceKind::Panel)
        .title(title)
        .anchor(Anchor::MonitorCenter)
        .size(size(px(PANEL_WIDTH), px(height)))
        .show_focused(tracker, cx, move |dismisser, _window, cx| {
            SettingsPanelView::new(
                panel,
                SettingsPanelState {
                    rows,
                    values,
                    path,
                    body_max,
                    runtime,
                },
                dismisser,
                cx,
            )
        })
        .map(|_| ())
}

fn panel_height(rows: &[Row]) -> f32 {
    PANEL_CHROME_HEIGHT + rows.iter().map(view::row_height).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::{panel_height, Row, PANEL_CHROME_HEIGHT, PANEL_ROW_HEIGHT};
    use crate::settings_panel::rows::RowControl;

    fn row(control: RowControl) -> Row {
        Row {
            section_label: None,
            label: "Label".into(),
            config_key: "key".into(),
            control,
        }
    }

    #[test]
    fn overlay_controls_do_not_change_parent_panel_height() {
        let toggle = vec![row(RowControl::Toggle(false))];
        let color = vec![row(RowControl::Color("#ffffff".into()))];
        let expected = PANEL_CHROME_HEIGHT + PANEL_ROW_HEIGHT;

        assert_eq!(panel_height(&toggle), expected);
        assert_eq!(panel_height(&color), expected);
    }
}
