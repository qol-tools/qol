use std::path::Path;

use crate::{Monitor, Rect};

pub(super) fn rect_label(rect: Rect) -> String {
    format!("{}x{}+{},{}", rect.w, rect.h, rect.x, rect.y)
}

pub(super) fn monitor_label(monitor: Monitor) -> String {
    format!("{}x{}+{},{}", monitor.w, monitor.h, monitor.x, monitor.y)
}

pub(super) fn path_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("none")
        .to_string()
}
