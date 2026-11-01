use anyhow::{anyhow, Result};
use gpui::{point, px, size, Bounds, Pixels};
use qol_gpui::monitor::active_first_monitor;
use qol_runtime::{MonitorBounds, PlatformStateClient};

use crate::{platform, Rect};

pub(crate) const SCREENSHOT_MONITOR_ACTION: &str = "screenshot-monitor";
pub(crate) const RECORD_MONITOR_ACTION: &str = "record-monitor";

pub(crate) fn rect_for(monitor: Bounds<Pixels>) -> Result<Rect> {
    platform::monitor_capture_rect(monitor)
        .ok_or_else(|| anyhow!("the active monitor has no capture area"))
}

pub(crate) fn active_monitor_rect() -> Result<Rect> {
    let state = PlatformStateClient::from_env()
        .get_state()
        .ok_or_else(|| anyhow!("the active monitor is unknown; run this from qol-tray"))?;
    let monitor =
        active_first_monitor(&state).ok_or_else(|| anyhow!("the runtime reported no monitors"))?;
    rect_for(native_bounds(monitor))
}

fn native_bounds(monitor: MonitorBounds) -> Bounds<Pixels> {
    Bounds::new(
        point(px(monitor.x), px(monitor.y)),
        size(px(monitor.width), px(monitor.height)),
    )
}
