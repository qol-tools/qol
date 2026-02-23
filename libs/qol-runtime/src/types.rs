use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CursorPos {
    pub x: f32,
    pub y: f32,
}

/// Monitor bounds as [x, y, width, height].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MonitorBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Focused window bounds as [x, y, width, height].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformState {
    pub cursor: Option<CursorPos>,
    pub monitors: Vec<MonitorBounds>,
    /// Index into `monitors` for the monitor containing the cursor.
    pub cursor_monitor_idx: Option<usize>,
    /// Index into `monitors` for the monitor containing the focused window.
    pub focus_monitor_idx: Option<usize>,
    /// Index into `monitors` for the "active" monitor (resolved from cursor/focus).
    pub active_monitor_idx: Option<usize>,
    /// Bounds of the currently focused window, if any.
    pub focused_window: Option<WindowBounds>,
}

impl PlatformState {
    pub fn active_monitor(&self) -> Option<MonitorBounds> {
        self.active_monitor_idx.and_then(|i| self.monitors.get(i).copied())
    }

    pub fn cursor_monitor(&self) -> Option<MonitorBounds> {
        self.cursor_monitor_idx.and_then(|i| self.monitors.get(i).copied())
    }

    pub fn focus_monitor(&self) -> Option<MonitorBounds> {
        self.focus_monitor_idx.and_then(|i| self.monitors.get(i).copied())
    }
}
