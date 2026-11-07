use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::device_fields::{self, PluginFields};
use crate::fs_util::{current_os_subdir, list_profile_dirs};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_138ToV3_139SyncCodeChannels;

const NAME: &str = "v3.138-to-v3.139-sync-code-channels";

const TASK_RUNNER_FILE: &str = "task-runner.json";

const DEVICE_FIELDS: PluginFields = &[(
    "qol-voice",
    &["recognition.provider", "recognition.websocket_endpoint"],
)];

impl FileMigration for V3_138ToV3_139SyncCodeChannels {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        Ok(device_fields::applies(config_dir, DEVICE_FIELDS)?
            || !pending_task_runner_copies(config_dir)?.is_empty())
    }

    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport> {
        let mut report = device_fields::migrate(config_dir, archive_dir, NAME, DEVICE_FIELDS)?;
        for (source, destination) in pending_task_runner_copies(config_dir)? {
            std::fs::create_dir_all(destination.parent().unwrap_or(config_dir))
                .with_context(|| format!("creating the folder for {}", destination.display()))?;
            std::fs::copy(&source, &destination).with_context(|| {
                format!("copying {} to {}", source.display(), destination.display())
            })?;
            report.archived.push(destination);
        }
        Ok(report)
    }
}

fn pending_task_runner_copies(config_dir: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut pending = Vec::new();
    for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
        let source = profile_dir
            .join("os")
            .join(current_os_subdir())
            .join(TASK_RUNNER_FILE);
        let destination = profile_dir.join("device").join(TASK_RUNNER_FILE);
        if source.is_file() && !destination.exists() {
            pending.push((source, destination));
        }
    }
    Ok(pending)
}

#[cfg(test)]
mod tests;
