use std::path::Path;

use super::RevealPlan;

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    target.to_path_buf().into()
}
