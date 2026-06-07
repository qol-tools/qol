use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use anyhow::{Context, Result};
use std::path::PathBuf;

const PLUGINS_DIR_ID: &str = "plugins_dir";

pub(super) struct PluginsDirCheck;

impl DoctorCheck for PluginsDirCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(PLUGINS_DIR_ID, "Plugins directory", CheckCategory::Runtime)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let plugins_dir = match crate::paths::plugins_dir() {
            Ok(path) => path,
            Err(error) => return CheckReport::error(plugins_dir_error(error), PLUGINS_DIR_ID),
        };
        plugins_dir_diagnosis(plugins_dir)
    }
}

pub(super) fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("failed to resolve current executable")
}

fn plugins_dir_diagnosis(plugins_dir: PathBuf) -> CheckReport {
    if plugins_dir.exists() && plugins_dir.is_dir() {
        return CheckReport::ok(format!(
            "plugins directory exists ({})",
            plugins_dir.display()
        ));
    }

    if plugins_dir.exists() {
        return CheckReport::error(
            format!(
                "plugins path exists but is not a directory ({})",
                plugins_dir.display()
            ),
            PLUGINS_DIR_ID,
        );
    }

    CheckReport::warn(
        format!("plugins directory missing ({})", plugins_dir.display()),
        PLUGINS_DIR_ID,
        vec![FixAction::EnsurePluginsDir { path: plugins_dir }],
    )
}

fn plugins_dir_error(error: anyhow::Error) -> String {
    format!("failed to resolve plugins directory: {}", error)
}
