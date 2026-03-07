use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::manifest;

#[derive(Debug, Clone)]
pub struct ClassifiedSource {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub already_linked: bool,
    pub installed_not_linked: bool,
}

pub(super) struct LinkState<'a> {
    plugins_dir: &'a Path,
    dev_links: &'a HashMap<String, PathBuf>,
}

impl<'a> LinkState<'a> {
    pub(super) fn new(plugins_dir: &'a Path, dev_links: &'a HashMap<String, PathBuf>) -> Self {
        Self {
            plugins_dir,
            dev_links,
        }
    }
}

pub(super) fn classify_sources(
    plugin_dirs: &[PathBuf],
    link_state: &LinkState<'_>,
) -> Vec<ClassifiedSource> {
    let mut seen_paths = HashSet::new();
    let mut sources = Vec::new();

    for dir in plugin_dirs {
        let Some(source) = classify_source(dir, link_state, &mut seen_paths) else {
            continue;
        };
        sources.push(source);
    }

    sources
}

fn classify_source(
    path: &Path,
    link_state: &LinkState<'_>,
    seen_paths: &mut HashSet<PathBuf>,
) -> Option<ClassifiedSource> {
    if !mark_source_path(path, seen_paths) {
        return None;
    }

    let source = read_source(path)?;
    let (already_linked, installed_not_linked) = install_status(link_state, &source);

    Some(ClassifiedSource {
        id: source.id,
        name: source.name,
        path: source.path,
        already_linked,
        installed_not_linked,
    })
}

#[derive(Debug, Clone)]
struct SourceInfo {
    id: String,
    name: String,
    path: PathBuf,
}

fn read_source(path: &Path) -> Option<SourceInfo> {
    if !is_plugin_dir(path) {
        return None;
    }

    let id = path.file_name()?.to_string_lossy().to_string();
    if id == "plugin-template" {
        return None;
    }

    let name = manifest::read_plugin_name(&path.join("plugin.toml")).unwrap_or_else(|| id.clone());

    Some(SourceInfo {
        id,
        name,
        path: path.to_path_buf(),
    })
}

fn mark_source_path(path: &Path, seen_paths: &mut HashSet<PathBuf>) -> bool {
    seen_paths.insert(canonical_path(path))
}

fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_plugin_dir(path: &Path) -> bool {
    path.is_dir() && path.join("plugin.toml").exists()
}

fn install_status(link_state: &LinkState<'_>, source: &SourceInfo) -> (bool, bool) {
    if let Some(linked_path) = link_state.dev_links.get(&source.id) {
        let is_same = linked_path == &source.path
            || linked_path.canonicalize().ok() == source.path.canonicalize().ok();
        return (is_same, false);
    }

    let install_path = link_state.plugins_dir.join(&source.id);
    if install_path.exists() {
        return (false, true);
    }

    (false, false)
}
