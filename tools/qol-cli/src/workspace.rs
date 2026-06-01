use crate::host_facade;
use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value;

pub(crate) fn repo_root() -> Result<PathBuf> {
    let mut current = env::current_dir().context("failed to read current directory")?;
    loop {
        if is_qol_tray_root(&current)? {
            return Ok(current);
        }
        if !current.pop() {
            bail!("run this from inside the qol-tray repo");
        }
    }
}

fn is_qol_tray_root(path: &Path) -> Result<bool> {
    let manifest = path.join("Cargo.toml");
    if !manifest.is_file() {
        return Ok(false);
    }
    let content = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let parsed: Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    let name = parsed
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Value::as_str);
    Ok(name == Some("qol-tray"))
}

pub(crate) fn workspace_root(repo: &Path) -> Result<PathBuf> {
    for path in repo.ancestors() {
        if path.file_name().and_then(|name| name.to_str()) == Some("worktrees") {
            let parent = path
                .parent()
                .ok_or_else(|| anyhow!("worktrees directory has no parent"))?;
            return Ok(parent.to_path_buf());
        }
    }
    let parent = repo
        .parent()
        .ok_or_else(|| anyhow!("repo has no parent directory"))?;
    Ok(parent.to_path_buf())
}

pub(crate) fn sibling_crates(repo: &Path) -> Result<Vec<PathBuf>> {
    let workspace = workspace_root(repo)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(&workspace)
        .with_context(|| format!("failed to read {}", workspace.display()))?
    {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => continue,
        };
        if name == "qol-tray" {
            continue;
        }
        if !name.starts_with("plugin-") && !name.starts_with("qol-") {
            continue;
        }
        if !path.join("Cargo.toml").is_file() {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

pub(crate) fn resolve_crate_target(root: &Path, name: &str) -> Result<PathBuf> {
    let mut candidates = vec![(root.to_path_buf(), "qol-tray".to_string())];
    for sibling in sibling_crates(root)? {
        let dn = display_name(&sibling);
        candidates.push((sibling, dn));
    }
    let matches: Vec<&(PathBuf, String)> = candidates
        .iter()
        .filter(|(_, dn)| crate_name_matches(dn, name))
        .collect();
    match matches.as_slice() {
        [] => bail!("no qol-tray or sibling crate matching `{name}`"),
        [only] => Ok(only.0.clone()),
        many => {
            let names: Vec<&str> = many.iter().map(|(_, dn)| dn.as_str()).collect();
            bail!("ambiguous `{name}` - matches: {}", names.join(", "));
        }
    }
}

fn crate_name_matches(display: &str, query: &str) -> bool {
    display == query || display == format!("plugin-{query}") || display == format!("qol-{query}")
}

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

pub(crate) fn exe_name(name: &str) -> String {
    host_facade::exe_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_sibling_crates_from_main_clone() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        for name in ["plugin-a", "qol-lib", "other"] {
            let dir = tmp.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        }
        let names: Vec<_> = sibling_crates(&repo)
            .unwrap()
            .into_iter()
            .map(|path| display_name(&path))
            .collect();
        assert_eq!(names, vec!["plugin-a".to_string(), "qol-lib".to_string()]);
    }

    #[test]
    fn discovers_sibling_crates_from_feature_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("worktrees").join("feat").join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let plugin = tmp.path().join("plugin-a");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let names: Vec<_> = sibling_crates(&repo)
            .unwrap()
            .into_iter()
            .map(|path| display_name(&path))
            .collect();
        assert_eq!(names, vec!["plugin-a".to_string()]);
    }

    #[test]
    fn resolve_crate_target_handles_qol_tray_and_siblings() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let plugin = tmp.path().join("plugin-alt-tab");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let lib = tmp.path().join("qol-color");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("Cargo.toml"), "[package]\nname = \"y\"\n").unwrap();

        assert_eq!(resolve_crate_target(&repo, "qol-tray").unwrap(), repo);
        assert_eq!(
            resolve_crate_target(&repo, "plugin-alt-tab").unwrap(),
            plugin
        );
        assert_eq!(resolve_crate_target(&repo, "alt-tab").unwrap(), plugin);
        assert_eq!(resolve_crate_target(&repo, "qol-color").unwrap(), lib);
        assert_eq!(resolve_crate_target(&repo, "color").unwrap(), lib);
        assert_eq!(resolve_crate_target(&repo, "tray").unwrap(), repo);
        assert!(resolve_crate_target(&repo, "missing").is_err());
    }

    #[test]
    fn resolve_crate_target_reports_ambiguity() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        for name in ["plugin-foo", "qol-foo"] {
            let dir = tmp.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("Cargo.toml"), "[package]\nname = \"z\"\n").unwrap();
        }
        let err = resolve_crate_target(&repo, "foo").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "got: {err}");
        assert!(
            err.contains("plugin-foo") && err.contains("qol-foo"),
            "got: {err}"
        );
    }
}
