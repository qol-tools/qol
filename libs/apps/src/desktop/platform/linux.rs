use std::path::PathBuf;

use crate::AppRoot;

use super::DesktopPlatform;

const XDG_ROOT_DEPTH: usize = 1;
const LOOSE_ROOT_DEPTH: usize = 2;

pub(super) struct Platform;

impl DesktopPlatform for Platform {
    fn cache_dir(&self) -> Option<PathBuf> {
        std::env::var("XDG_CACHE_HOME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(|home| PathBuf::from(format!("{home}/.cache")))
            })
    }

    fn app_roots(&self) -> Vec<AppRoot> {
        let mut roots = xdg_app_dirs()
            .into_iter()
            .map(|path| AppRoot {
                path,
                max_depth: XDG_ROOT_DEPTH,
            })
            .collect::<Vec<_>>();
        roots.extend(loose_install_dirs().into_iter().map(|path| AppRoot {
            path,
            max_depth: LOOSE_ROOT_DEPTH,
        }));
        roots.sort_by(|left, right| left.path.cmp(&right.path));
        roots.dedup_by(|left, right| left.path == right.path);
        roots.retain(|root| root.path.is_dir());
        roots
    }
}

fn xdg_app_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{home}/.local/share"));
    let mut dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from(format!("{data_home}/applications")),
        PathBuf::from(format!(
            "{home}/.local/share/flatpak/exports/share/applications"
        )),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
    ];
    if let Ok(extra) = std::env::var("XDG_DATA_DIRS") {
        dirs.extend(
            extra
                .split(':')
                .map(str::trim)
                .filter(|segment| !segment.is_empty())
                .map(|segment| PathBuf::from(format!("{segment}/applications"))),
        );
    }
    dirs
}

fn loose_install_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs = vec![PathBuf::from("/opt")];
    if home.is_empty() {
        return dirs;
    }
    dirs.push(PathBuf::from(format!("{home}/.local")));
    dirs.push(PathBuf::from(format!("{home}/Applications")));
    dirs
}
