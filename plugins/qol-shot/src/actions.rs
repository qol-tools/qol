use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::{output, platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotAction {
    Copy,
    CopyPath,
}

impl ShotAction {
    pub const ALL: &'static [ShotAction] = &[ShotAction::Copy, ShotAction::CopyPath];

    pub fn perform(self, path: &Path) -> Result<()> {
        match self {
            ShotAction::Copy => platform::copy_image_to_clipboard(path),
            ShotAction::CopyPath => platform::copy_path_to_clipboard(path),
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            ShotAction::Copy => "C",
            ShotAction::CopyPath => "P",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ShotAction::Copy => "Copy",
            ShotAction::CopyPath => "Copy Path",
        }
    }

    pub fn accel(self) -> char {
        match self {
            ShotAction::Copy => 'c',
            ShotAction::CopyPath => 'p',
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
