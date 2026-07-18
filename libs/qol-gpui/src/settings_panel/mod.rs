mod color_wheel;
mod persistence;
mod rows;
mod view;

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::surface::{Anchor, Surface, SurfaceKind};
use rows::{rows_from_resolved, Row, RowControl};
use view::SettingsPanelView;

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

pub fn open_from_async(
    panel: SettingsPanel,
    tracker: MonitorTracker,
    provider: impl Fn(&str) -> Vec<(String, String)> + 'static,
    cx: &AsyncApp,
) -> anyhow::Result<()> {
    cx.update(move |cx| open(panel, &tracker, &provider, cx))?
}

pub fn open(
    panel: SettingsPanel,
    tracker: &MonitorTracker,
    provider: &QueryOptions,
    cx: &mut App,
) -> anyhow::Result<()> {
    let spec = qol_config::contract::parse_spec_str(panel.contract)
        .map_err(|error| anyhow::anyhow!("contract parse failed: {error:?}"))?;
    let path = persistence::config_path(panel.plugin_id)?;
    let values = persistence::load_values(panel.plugin_id, &path);
    let resolved = qol_config::normalized::resolve_config(&spec, &values)
        .map_err(|errors| anyhow::anyhow!("contract resolve failed: {errors:?}"))?;
    let rows = rows_from_resolved(&resolved, provider);
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
            SettingsPanelView::new(panel, rows, values, path, body_max, dismisser, cx)
        })
        .map(|_| ())
}

fn panel_height(rows: &[Row]) -> f32 {
    let headers = rows
        .iter()
        .filter(|row| row.section_label.is_some())
        .count() as f32;
    let content_height = PANEL_CHROME_HEIGHT
        + rows.len() as f32 * PANEL_ROW_HEIGHT
        + headers * PANEL_SECTION_HEADER_HEIGHT;
    if rows
        .iter()
        .any(|row| matches!(&row.control, RowControl::Color(_)))
    {
        return content_height.max(color_wheel::MIN_HOST_HEIGHT);
    }
    content_height
}

#[cfg(test)]
mod tests {
    use super::{panel_height, Row, RowControl, PANEL_CHROME_HEIGHT, PANEL_ROW_HEIGHT};

    fn row(control: RowControl) -> Row {
        Row {
            section_label: None,
            label: "Label".into(),
            config_key: "key".into(),
            control,
        }
    }

    #[test]
    fn color_controls_reserve_enough_height_for_the_wheel() {
        let plain = vec![row(RowControl::Toggle(false))];
        let color = vec![row(RowControl::Color("#ffffff".into()))];

        assert_eq!(panel_height(&plain), PANEL_CHROME_HEIGHT + PANEL_ROW_HEIGHT);
        assert_eq!(panel_height(&color), super::color_wheel::MIN_HOST_HEIGHT);
    }
}
