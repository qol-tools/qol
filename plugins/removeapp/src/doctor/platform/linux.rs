use std::path::PathBuf;

use super::directory::{inspect_directory, inspect_paths};
use super::PlatformInspection;

pub(crate) fn inspect() -> PlatformInspection {
    let inventory_roots = inspect_paths(
        qol_apps::desktop::linux_app_roots()
            .into_iter()
            .map(|root| root.path),
    );
    let home = absolute_home();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let (trash, trash_creation_anchor) = linux_trash_location(data_home, home.as_ref())
        .map(|(trash, anchor)| {
            (
                Some(inspect_directory(trash)),
                Some(inspect_directory(anchor)),
            )
        })
        .unwrap_or((None, None));

    PlatformInspection {
        name: "Linux",
        supported: true,
        inventory_roots,
        trash,
        trash_creation_anchor,
    }
}

fn linux_trash_location(
    data_home: Option<PathBuf>,
    home: Option<&PathBuf>,
) -> Option<(PathBuf, PathBuf)> {
    data_home
        .map(|path| (path.join("Trash"), path))
        .or_else(|| home.map(|home| (home.join(".local/share/Trash"), home.clone())))
}

fn absolute_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::linux_trash_location;
    use std::path::PathBuf;

    #[test]
    fn default_trash_path_is_derived_without_creating_it() {
        assert_eq!(
            linux_trash_location(None, Some(&PathBuf::from("/home/test"))),
            Some((
                PathBuf::from("/home/test/.local/share/Trash"),
                PathBuf::from("/home/test")
            ))
        );
    }

    #[test]
    fn explicit_data_home_controls_the_trash_path() {
        assert_eq!(
            linux_trash_location(
                Some(PathBuf::from("/data/test")),
                Some(&PathBuf::from("/home/test"))
            ),
            Some((
                PathBuf::from("/data/test/Trash"),
                PathBuf::from("/data/test")
            ))
        );
    }
}
