use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::{output, platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotAction {
    Copy,
    CopyPath,
}

impl ShotAction {
    pub fn perform(self, path: &Path) -> Result<()> {
        match self {
            ShotAction::Copy => platform::copy_image_to_clipboard(path),
            ShotAction::CopyPath => platform::copy_path_to_clipboard(path),
        }
    }

    pub fn done_message(self) -> &'static str {
        match self {
            ShotAction::Copy => "Copied screenshot to clipboard",
            ShotAction::CopyPath => "Copied screenshot path to clipboard",
        }
    }
}

pub fn perform_on_latest(action: ShotAction) -> Result<PathBuf> {
    let path = output::latest_screenshot()?;
    action.perform(&path)?;
    Ok(path)
}
