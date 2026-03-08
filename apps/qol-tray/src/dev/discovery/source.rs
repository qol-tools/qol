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
    resolved_dev_links: &'a HashMap<String, PathBuf>,
}

impl<'a> LinkState<'a> {
    pub(super) fn new(
        plugins_dir: &'a Path,
        dev_links: &'a HashMap<String, PathBuf>,
        resolved_dev_links: &'a HashMap<String, PathBuf>,
    ) -> Self {
        Self {
            plugins_dir,
            dev_links,
            resolved_dev_links,
        }
    }
}

pub(super) fn classify_sources(
    plugin_dirs: &[PathBuf],
    link_state: &LinkState<'_>,
) -> Vec<ClassifiedSource> {
    let mut seen_paths = HashSet::new();
    plugin_dirs
        .iter()
        .filter_map(|dir| classify_source(dir, link_state, &mut seen_paths))
        .collect()
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
    let canonical_source = source.path.canonicalize().ok();

    let matches_linked = |links: &HashMap<String, PathBuf>| {
        links.get(&source.id).is_some_and(|linked_path| {
            linked_path == &source.path
                || (canonical_source.is_some()
                    && linked_path.canonicalize().ok() == canonical_source)
        })
    };

    if matches_linked(link_state.dev_links) || matches_linked(link_state.resolved_dev_links) {
        return (true, false);
    }

    if link_state.dev_links.contains_key(&source.id) {
        return (false, false);
    }

    (false, link_state.plugins_dir.join(&source.id).exists())
}
