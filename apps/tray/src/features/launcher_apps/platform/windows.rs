use super::super::icon::{LauncherIcon, MarkFiles};
use super::super::LauncherEntry;
use super::link_names::{link_names, link_path, LINK_EXTENSION};
use crate::installer::boot_environment::paths_equal_canonicalized;
use crate::installer::platform::windows::ico::ico_from_pngs;
use crate::installer::platform::windows::shortcut::{self, Shortcut};
use crate::shortcuts::model::AppRef;
use anyhow::{anyhow, Context, Result};
use qol_apps::shell_link::{LinkTarget, ShellLink};
use qol_theme::Mark;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const ICONS_DIR: &str = "start-menu-icons";
const MARK_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];
const APP_ICON_PIXELS: usize = 256;
const ICON_EXTENSION: &str = "ico";

pub(super) fn sync(entries: &[LauncherEntry], target: &Path, marks: &MarkFiles) -> Result<()> {
    let icons = crate::paths::base_data_dir()?.join(ICONS_DIR);
    sync_into(&apps_dir()?, &icons, entries, target, marks)
}

pub(super) fn publish_synced() {
    use qol_runtime::protocol::RuntimeEvent;

    match apps_dir() {
        Ok(dir) => crate::runtime::publish(&[RuntimeEvent::LauncherAppsSynced { dir }]),
        Err(error) => log::warn!("launcher_apps: skipping LauncherAppsSynced: {error:#}"),
    }
}

fn apps_dir() -> Result<PathBuf> {
    Ok(crate::paths::platform::windows::start_menu_programs_dir()?.join(shortcut::LAUNCHER_FOLDER))
}

fn sync_into(
    dir: &Path,
    icons_dir: &Path,
    entries: &[LauncherEntry],
    target: &Path,
    marks: &MarkFiles,
) -> Result<()> {
    for folder in [dir, icons_dir] {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("Failed to create {}", folder.display()))?;
    }
    let mut icons = IconFiles::new(icons_dir);
    let names = link_names(entries);
    for (entry, name) in entries.iter().zip(&names) {
        super::verify_target(entry, target)?;
        let icon = icons.location(&entry.icon, marks)?;
        let arguments = qol_apps::shell_execute::join_args(&entry.exec_args);
        let Some(link) = link_path(dir, name) else {
            log::warn!(
                "launcher_apps: refusing link {name:?} outside {}",
                dir.display()
            );
            continue;
        };
        let wanted = Shortcut {
            target,
            arguments: &arguments,
            icon: &icon,
            description: &entry.description,
        };
        if !link_matches(&link, &wanted) {
            shortcut::write(&link, &wanted)?;
        }
    }
    remove_stale(dir, LINK_EXTENSION, names.iter().map(String::as_str));
    remove_stale(
        icons_dir,
        ICON_EXTENSION,
        icons.kept.iter().map(String::as_str),
    );
    Ok(())
}

fn link_matches(link: &Path, wanted: &Shortcut<'_>) -> bool {
    let Some(existing) = ShellLink::read(link) else {
        return false;
    };
    let LinkTarget::Path(target) = existing.target(link, |name| std::env::var(name).ok()) else {
        return false;
    };
    paths_equal_canonicalized(&target, wanted.target)
        && existing.arguments.as_deref().unwrap_or_default() == wanted.arguments
        && existing.description.as_deref().unwrap_or_default() == wanted.description
        && existing
            .icon_location
            .is_some_and(|icon| paths_equal_canonicalized(Path::new(&icon), wanted.icon))
}

fn remove_stale<'a>(dir: &Path, extension: &str, keep: impl IntoIterator<Item = &'a str>) {
    let keep: HashSet<String> = keep.into_iter().map(str::to_lowercase).collect();
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    for path in listing.flatten().map(|item| item.path()) {
        let ours = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(extension));
        let kept = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| keep.contains(&name.to_lowercase()));
        if ours && !kept {
            if let Err(error) = std::fs::remove_file(&path) {
                log::warn!(
                    "launcher_apps: failed to remove {}: {error}",
                    path.display()
                );
            }
        }
    }
}

struct IconFiles<'a> {
    dir: &'a Path,
    marks: HashMap<Mark, PathBuf>,
    kept: HashSet<String>,
}

impl<'a> IconFiles<'a> {
    fn new(dir: &'a Path) -> Self {
        Self {
            dir,
            marks: HashMap::new(),
            kept: HashSet::new(),
        }
    }

    fn location(&mut self, icon: &LauncherIcon, marks: &MarkFiles) -> Result<PathBuf> {
        let mark = match icon {
            LauncherIcon::Mark(mark) => *mark,
            LauncherIcon::TargetApp(app) => match self.app_icon(app)? {
                Some(path) => return Ok(path),
                None => Mark::App,
            },
        };
        if let Some(path) = self.marks.get(&mark) {
            return Ok(path.clone());
        }
        let svg = marks.svg(mark);
        let pngs = MARK_SIZES
            .iter()
            .map(|&size| {
                super::raster::svg_png(&svg, size)
                    .map(|png| (size, png))
                    .ok_or_else(|| anyhow!("Failed to render the {} mark", mark.name()))
            })
            .collect::<Result<Vec<_>>>()?;
        let path = self.write(mark.name(), &pngs)?;
        self.marks.insert(mark, path.clone());
        Ok(path)
    }

    fn app_icon(&mut self, app: &AppRef) -> Result<Option<PathBuf>> {
        let AppRef::Path { path } = app else {
            return Ok(None);
        };
        let path = Path::new(path);
        if !path.is_file() {
            return Ok(None);
        }
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            return Ok(Some(path.to_path_buf()));
        }
        let Some(png) = qol_app_icon::icon_png_for_path(path, APP_ICON_PIXELS) else {
            return Ok(None);
        };
        self.write("target-app", &[(APP_ICON_PIXELS as u32, png)])
            .map(Some)
    }

    fn write(&mut self, stem: &str, pngs: &[(u32, Vec<u8>)]) -> Result<PathBuf> {
        let images: Vec<(u32, &[u8])> = pngs
            .iter()
            .map(|(size, png)| (*size, png.as_slice()))
            .collect();
        let ico = ico_from_pngs(&images);
        let digest: String = Sha256::digest(&ico)[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let name = format!("{stem}-{digest}.{ICON_EXTENSION}");
        let path = self.dir.join(&name);
        if !path.is_file() {
            std::fs::write(&path, &ico)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }
        self.kept.insert(name);
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(file_stem: &str, display_name: &str, icon: LauncherIcon) -> LauncherEntry {
        LauncherEntry {
            file_stem: file_stem.into(),
            display_name: display_name.into(),
            description: format!("QoL: {display_name}"),
            bundle_id: String::new(),
            exec_args: vec!["exec".into(), "shortcut".into(), file_stem.into()],
            shortcut_action: None,
            icon,
            window_class: None,
        }
    }

    fn mark(file_stem: &str, display_name: &str) -> LauncherEntry {
        entry(file_stem, display_name, LauncherIcon::Mark(Mark::Shortcuts))
    }

    #[test]
    fn sync_writes_one_link_per_entry_and_prunes_only_its_own_stale_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("Programs").join("QoL");
        let icons = temp.path().join("icons");
        let target = temp.path().join("bin").join("qol-tray.exe");
        let app = temp.path().join("bin").join("app.exe");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"MZ").unwrap();
        std::fs::write(&app, b"MZ").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::write(dir.join("Retired.lnk"), b"old").unwrap();
        std::fs::write(dir.join("desktop.ini"), b"keep").unwrap();
        std::fs::write(icons.join("retired-000000000000.ico"), b"old").unwrap();
        let marks = MarkFiles::in_dir(temp.path().join("marks"), 0);
        let entries = [
            mark("command-add", "QoL \u{203a} Add Shortcut"),
            entry(
                "shortcut-app",
                "My App",
                LauncherIcon::TargetApp(AppRef::Path {
                    path: app.display().to_string(),
                }),
            ),
        ];

        sync_into(&dir, &icons, &entries, &target, &marks).unwrap();

        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap();
        let read = |name: &str| ShellLink::read(&dir.join(name)).expect(name);
        let added = read("QoL \u{203a} Add Shortcut.lnk");
        assert_eq!(
            added.arguments.as_deref(),
            Some("exec shortcut command-add")
        );
        assert_eq!(
            added.description.as_deref(),
            Some("QoL: QoL \u{203a} Add Shortcut")
        );
        let mark_icon = PathBuf::from(added.icon_location.unwrap());
        assert_eq!(canonical(mark_icon.parent().unwrap()), canonical(&icons));
        assert!(std::fs::read(&mark_icon)
            .unwrap()
            .starts_with(&[0, 0, 1, 0]));
        let app_link = read("My App.lnk");
        assert_eq!(
            canonical(Path::new(&app_link.icon_location.unwrap())),
            canonical(&app)
        );
        assert!(!dir.join("Retired.lnk").exists());
        assert!(dir.join("desktop.ini").exists());
        assert!(!icons.join("retired-000000000000.ico").exists());

        let written = std::fs::metadata(dir.join("My App.lnk"))
            .unwrap()
            .modified()
            .unwrap();
        sync_into(&dir, &icons, &entries, &target, &marks).unwrap();
        let rewritten = std::fs::metadata(dir.join("My App.lnk"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(written, rewritten);
        assert!(mark_icon.is_file());
    }
}
