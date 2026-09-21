use std::collections::HashSet;
use std::path::PathBuf;

use super::DevConfig;

pub(super) fn effective_search_paths(config: &DevConfig) -> Vec<PathBuf> {
    unique_paths(configured_or_default_paths(config))
}

fn configured_or_default_paths(config: &DevConfig) -> Vec<PathBuf> {
    if !config.search_paths.is_empty() {
        return config.search_paths.clone();
    }

    default_search_paths()
}

fn default_search_paths() -> Vec<PathBuf> {
    let mut paths = home_search_paths();
    let Some(parent) = workspace_parent() else {
        return paths;
    };

    paths.push(parent);
    paths
}

fn home_search_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };

    existing_common_dev_dirs(home)
}

fn existing_common_dev_dirs(home: PathBuf) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for name in common_dev_dirs() {
        let path = home.join(name);
        if !path.is_dir() {
            continue;
        }
        paths.push(path);
    }

    paths
}

fn workspace_parent() -> Option<PathBuf> {
    git_main_repo_parent().or_else(|| {
        let cwd = std::env::current_dir().ok()?;
        cwd.parent().map(|p| p.to_path_buf())
    })
}

fn git_main_repo_parent() -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let git_dir = PathBuf::from(&raw);
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        std::env::current_dir().ok()?.join(git_dir)
    };
    git_dir.parent()?.parent().map(|p| p.to_path_buf())
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique_paths = Vec::new();
    let mut seen = HashSet::new();

    for path in paths {
        let abs_path = path.canonicalize().unwrap_or(path);
        if !seen.insert(abs_path.clone()) {
            continue;
        }
        unique_paths.push(abs_path);
    }

    unique_paths
}

fn common_dev_dirs() -> &'static [&'static str] {
    &[
        "Developer",
        "Projects",
        "repos",
        "src",
        "code",
        "dev",
        "Git",
        "GitHub",
        "work",
        "workspace",
        "Documents/GitHub",
        "Documents/Projects",
    ]
}
