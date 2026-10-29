use std::path::{Path, PathBuf};

use qol_agent_homes::{Harness, Registry};

pub(super) fn claude_home() -> PathBuf {
    Registry::load().current(Harness::Claude).path
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
    std::fs::canonicalize(&dir).ok().or(Some(dir))
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
    fn conversations_live_under_the_config_home_projects_folder() {
        assert_eq!(
            conversations_dir(Path::new("/h/.claude"), Path::new("/p/x")),
            Path::new("/h/.claude/projects/-p-x")
        );
    }
}
