mod store;

use serde::{Deserialize, Serialize};

pub use qol_windowing::{MonitorBounds, WindowRect};

pub use store::WindowStateStore;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub version: u32,
    pub key: String,
    pub monitor: MonitorRef,
    pub bounds_on_monitor: WindowRect,
    pub scale: f32,
    pub open: bool,
    pub owner_pid: u32,
    pub default_placement: String,
    pub reopen: Option<Reopen>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorRef {
    pub id: String,
    pub connector: String,
    pub bounds: MonitorBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reopen {
    PluginAction { plugin: String, action: String },
    Settings { page: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub id: String,
    pub connector: String,
    pub bounds: MonitorBounds,
    pub work_area: MonitorBounds,
    pub primary: bool,
}

impl Monitor {
    fn reference(&self) -> MonitorRef {
        MonitorRef {
            id: self.id.clone(),
            connector: self.connector.clone(),
            bounds: self.bounds,
        }
    }
}

pub fn locate(window: WindowRect, monitors: &[Monitor]) -> Option<(MonitorRef, WindowRect)> {
    let centre = centre_of(window);
    let monitor = monitors
        .iter()
        .min_by(|a, b| distance(a.bounds, centre).total_cmp(&distance(b.bounds, centre)))?;
    let on_monitor = WindowRect {
        x: window.x - f64::from(monitor.bounds.x),
        y: window.y - f64::from(monitor.bounds.y),
        ..window
    };
    Some((monitor.reference(), on_monitor))
}

pub fn resolve(saved: &WindowState, monitors: &[Monitor]) -> Option<WindowRect> {
    let monitor = monitors
        .iter()
        .find(|monitor| monitor.id == saved.monitor.id)
        .or_else(|| {
            monitors
                .iter()
                .find(|monitor| monitor.connector == saved.monitor.connector)
        })
        .or_else(|| nearest_to_saved(saved, monitors))?;
    let window = WindowRect {
        x: f64::from(monitor.bounds.x) + saved.bounds_on_monitor.x,
        y: f64::from(monitor.bounds.y) + saved.bounds_on_monitor.y,
        ..saved.bounds_on_monitor
    };
    Some(clamp_into(window, monitor.work_area))
}

fn nearest_to_saved<'a>(saved: &WindowState, monitors: &'a [Monitor]) -> Option<&'a Monitor> {
    let was = centre_of(WindowRect {
        x: f64::from(saved.monitor.bounds.x) + saved.bounds_on_monitor.x,
        y: f64::from(saved.monitor.bounds.y) + saved.bounds_on_monitor.y,
        ..saved.bounds_on_monitor
    });
    monitors.iter().min_by(|a, b| {
        distance(a.bounds, was)
            .total_cmp(&distance(b.bounds, was))
            .then_with(|| b.primary.cmp(&a.primary))
    })
}

fn centre_of(window: WindowRect) -> (f64, f64) {
    (
        window.x + window.width / 2.0,
        window.y + window.height / 2.0,
    )
}

fn distance(bounds: MonitorBounds, (x, y): (f64, f64)) -> f64 {
    let left = f64::from(bounds.x);
    let top = f64::from(bounds.y);
    let dx = (left - x)
        .max(0.0)
        .max(x - (left + f64::from(bounds.width)));
    let dy = (top - y).max(0.0).max(y - (top + f64::from(bounds.height)));
    dx.hypot(dy)
}

pub fn clamp_into(window: WindowRect, area: MonitorBounds) -> WindowRect {
    let left = f64::from(area.x);
    let top = f64::from(area.y);
    let width = window.width.min(f64::from(area.width)).max(0.0);
    let height = window.height.min(f64::from(area.height)).max(0.0);
    WindowRect {
        x: window.x.clamp(left, left + f64::from(area.width) - width),
        y: window.y.clamp(top, top + f64::from(area.height) - height),
        width,
        height,
    }
}

pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

#[cfg(test)]
mod tests;
