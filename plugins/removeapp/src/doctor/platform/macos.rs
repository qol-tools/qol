use std::path::PathBuf;

use super::directory::{inspect_directory, inspect_paths};
use super::PlatformInspection;

pub(crate) fn inspect() -> PlatformInspection {
    let home = absolute_home();
    let inventory_roots = inspect_paths(
        std::iter::once(PathBuf::from("/Applications"))
            .chain(home.as_ref().map(|home| home.join("Applications"))),
    );
    let trash = home
        .as_ref()
        .map(|home| inspect_directory(home.join(".Trash")));
    let trash_creation_anchor = home.map(inspect_directory);

    PlatformInspection {
        name: "macOS",
        supported: true,
        inventory_roots,
        trash,
        trash_creation_anchor,
    }
}

fn absolute_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}
