use std::path::Path;
use std::process::Command;

use super::RevealPlan;

pub(super) use super::commands::{open, open_checked};
pub(super) use super::unsupported::{
    file_type, open_handlers, open_with_handler, open_with_supported,
};

pub(super) fn terminal_commands(_folder: &Path) -> Vec<Command> {
    Vec::new()
}

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    target.to_path_buf().into()
}
