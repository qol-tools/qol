use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub(crate) fn rules_dir() -> PathBuf {
    PathBuf::from(super::RULES_DIR)
}

pub(crate) fn grant(_rule_path: &Path, _rule_content: &str) -> Result<()> {
    bail!("udev uaccess grants are not implemented on Windows")
}

pub(crate) fn restore_rule(_rule_path: &Path, _rule_content: &str) -> Result<()> {
    bail!("udev uaccess grants are not implemented on Windows")
}
