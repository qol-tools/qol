use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

pub fn find_plugin_dirs(search_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut plugins = Vec::new();

    for search_path in search_paths {
        collect_search_path_plugins(search_path, &mut plugins);
    }

    plugins
}

fn collect_search_path_plugins(search_path: &Path, plugins: &mut Vec<PathBuf>) {
    if !search_path.exists() {
        return;
    }

    let mut entries = WalkDir::new(search_path)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_visit_entry);

    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();

        if !is_plugin_dir(path) {
            continue;
        }

        plugins.push(path.to_path_buf());
        entries.skip_current_dir();
    }
}

fn should_visit_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    let name = entry.file_name().to_string_lossy();
    !name.starts_with('.') && name != "node_modules" && name != "target" && name != "vendor"
}

fn is_plugin_dir(path: &Path) -> bool {
    path.is_dir() && path.join("plugin.toml").exists()
}
