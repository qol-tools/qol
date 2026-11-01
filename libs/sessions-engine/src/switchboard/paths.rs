use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use qol_agent_homes::{Harness, Registry};

const CONFIG_DIR_RECORD: &str = "claude-config-dir";

pub(super) fn claude_home() -> PathBuf {
    Registry::load().current(Harness::Claude).path
}

pub(super) fn caller_config_dir() -> Option<PathBuf> {
    std::env::var_os(qol_agent_launch::account::CONFIG_DIR_ENV)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

pub(super) fn config_dir(switchboard: &Path, caller: Option<PathBuf>) -> Option<PathBuf> {
    let record = switchboard.join(CONFIG_DIR_RECORD);
    match caller {
        Some(caller) => {
            if let Err(error) = remember_config_dir(&record, &caller) {
                crate::cli::warn(format!(
                    "the switchboard will not remember its Claude account for the next launch from the tray: {error:#}"
                ));
            }
            Some(caller)
        }
        None => recorded_config_dir(&record),
    }
}

fn remember_config_dir(record: &Path, config_dir: &Path) -> Result<()> {
    let text = config_dir
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", config_dir.display()))?;
    if std::fs::read_to_string(record).is_ok_and(|current| current == text) {
        return Ok(());
    }
    qol_fs::atomic_write(record, text.as_bytes())
        .with_context(|| format!("failed to write {}", record.display()))
}

fn recorded_config_dir(record: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(record).ok()?;
    let dir = PathBuf::from(text.trim());
    (!text.trim().is_empty() && dir.is_dir()).then_some(dir)
}

pub(super) fn project_dir_name(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub(super) fn conversations_dir(config_home: &Path, project: &Path) -> PathBuf {
    config_home.join("projects").join(project_dir_name(project))
}

pub(super) fn switchboard_dir() -> Option<PathBuf> {
    let dir = qol_config::data_subdir("sessions")?.join(super::SWITCHBOARD_KEY);
    std::fs::canonicalize(&dir)
        .map(qol_platform::windows_path::strip_verbatim)
        .ok()
        .or(Some(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_folder_name_replaces_every_non_alphanumeric_character() {
        assert_eq!(
            project_dir_name(Path::new("/media/kmrh47/WD_SN850X/Git/qol-monorepo")),
            "-media-kmrh47-WD-SN850X-Git-qol-monorepo"
        );
        assert_eq!(project_dir_name(Path::new("/a b/ü.c")), "-a-b---c");
    }

    #[test]
    fn a_launch_from_a_terminal_names_the_account_a_later_launch_from_the_tray_reuses() {
        let switchboard = tempfile::tempdir().unwrap();
        let account = tempfile::tempdir().unwrap();
        assert_eq!(config_dir(switchboard.path(), None), None);
        assert_eq!(
            config_dir(switchboard.path(), Some(account.path().to_path_buf())),
            Some(account.path().to_path_buf())
        );
        assert_eq!(
            config_dir(switchboard.path(), None),
            Some(account.path().to_path_buf()),
            "a caller without CLAUDE_CONFIG_DIR, such as the tray, reuses the recorded account"
        );
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            config_dir(switchboard.path(), Some(other.path().to_path_buf())),
            Some(other.path().to_path_buf())
        );
        assert_eq!(
            config_dir(switchboard.path(), None),
            Some(other.path().to_path_buf()),
            "the newest terminal launch replaces the record"
        );
    }

    #[test]
    fn a_recorded_account_whose_folder_is_gone_or_blank_is_ignored() {
        let switchboard = tempfile::tempdir().unwrap();
        let record = switchboard.path().join(CONFIG_DIR_RECORD);
        std::fs::write(&record, "  \n").unwrap();
        assert_eq!(config_dir(switchboard.path(), None), None);
        std::fs::write(&record, switchboard.path().join("gone").to_str().unwrap()).unwrap();
        assert_eq!(config_dir(switchboard.path(), None), None);
    }

    #[test]
    fn conversations_live_under_the_config_home_projects_folder() {
        assert_eq!(
            conversations_dir(Path::new("/h/.claude"), Path::new("/p/x")),
            Path::new("/h/.claude/projects/-p-x")
        );
    }
}
