use std::path::Path;

pub(super) use crate::capture::geometry::{monitor_label, rect_label};

pub(super) fn path_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("none")
        .to_string()
}
