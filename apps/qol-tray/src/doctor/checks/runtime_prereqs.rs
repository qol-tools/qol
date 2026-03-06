use super::super::diagnosis::{error_outcome, ok_outcome, warn_outcome, Diagnosis, FixAction};
use anyhow::{Context, Result};
use std::path::PathBuf;

const PLUGINS_DIR_ID: &str = "plugins_dir";

pub(super) fn check_plugins_dir() -> Diagnosis {
    let plugins_dir = match crate::paths::plugins_dir() {
        Ok(path) => path,
        Err(error) => return error_outcome(PLUGINS_DIR_ID, plugins_dir_error(error)),
    };

    plugins_dir_diagnosis(plugins_dir)
}

pub(super) fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("failed to resolve current executable")
}

fn plugins_dir_diagnosis(plugins_dir: PathBuf) -> Diagnosis {
    if plugins_dir.exists() && plugins_dir.is_dir() {
        return ok_outcome(
            PLUGINS_DIR_ID,
            format!("plugins directory exists ({})", plugins_dir.display()),
        );
    }

    if plugins_dir.exists() {
        return error_outcome(
            PLUGINS_DIR_ID,
            format!(
                "plugins path exists but is not a directory ({})",
                plugins_dir.display()
            ),
        );
    }

    warn_outcome(
        PLUGINS_DIR_ID,
        format!("plugins directory missing ({})", plugins_dir.display()),
        Some(FixAction::EnsurePluginsDir { path: plugins_dir }),
    )
}

fn plugins_dir_error(error: anyhow::Error) -> String {
    format!("failed to resolve plugins directory: {}", error)
}
