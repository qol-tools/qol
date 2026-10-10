use std::fs;
use std::path::{Path, PathBuf};

use windows_sys::Win32::UI::Shell::{FOLDERID_CommonPrograms, FOLDERID_Programs};

use crate::known_folder::platform::windows::known_folder;
use crate::start_menu::is_start_menu_shortcut;
use crate::AppRoot;

use super::StartMenuPlatform;

const MAX_DEPTH: usize = 4;
const SKIPPED_FOLDER: &str = "Startup";

pub(super) struct Platform;

impl StartMenuPlatform for Platform {
    fn program_roots(&self) -> Vec<AppRoot> {
        [&FOLDERID_Programs, &FOLDERID_CommonPrograms]
            .into_iter()
            .filter_map(known_folder)
            .map(|path| AppRoot {
                path,
                max_depth: MAX_DEPTH,
            })
            .collect()
    }

    fn shortcut_paths(&self, root: &AppRoot) -> Vec<PathBuf> {
        let mut shortcuts = Vec::new();
        walk_for_shortcuts(&root.path, 0, root.max_depth, &mut shortcuts);
        shortcuts
    }
}

fn walk_for_shortcuts(dir: &Path, depth: usize, max_depth: usize, shortcuts: &mut Vec<PathBuf>) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let skipped = depth == 0
                && path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case(SKIPPED_FOLDER));
            if depth < max_depth && !skipped {
                walk_for_shortcuts(&path, depth + 1, max_depth, shortcuts);
            }
        } else if is_start_menu_shortcut(&path) {
            shortcuts.push(path);
        }
    }
}
