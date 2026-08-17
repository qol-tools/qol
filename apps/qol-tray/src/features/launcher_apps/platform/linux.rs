use super::super::{LauncherEntry, ResolvedEntry};
use anyhow::{Context, Result};
use qol_apps::desktop::{escape_desktop_entry_value, format_desktop_exec_command, DesktopExecArg};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DESKTOP_PREFIX: &str = "qol-";

pub(super) fn sync(entries: &[ResolvedEntry]) -> Result<()> {
    let dir = apps_dir().context("Could not determine local data directory")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create applications dir {}", dir.display()))?;

    let expected: HashSet<String> = entries
        .iter()
        .map(|resolved| desktop_filename(&resolved.entry))
        .collect();

    for resolved in entries {
        write_desktop_file(&dir, resolved)?;
    }

    clean_stale(&dir, &expected)?;
    Ok(())
}

pub(super) fn apps_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|p| p.join("applications"))
}

pub(super) fn publish_synced() {
    use qol_runtime::protocol::RuntimeEvent;

    let Some(dir) = apps_dir() else {
        log::warn!("launcher_apps: no apps dir on this platform; skipping LauncherAppsSynced");
        return;
    };
    crate::runtime::publish(&[RuntimeEvent::LauncherAppsSynced { dir }]);
}

fn desktop_filename(entry: &LauncherEntry) -> String {
    format!("{}{}.desktop", DESKTOP_PREFIX, entry.file_stem)
}

fn write_desktop_file(dir: &Path, resolved: &ResolvedEntry) -> Result<()> {
    let entry = &resolved.entry;
    super::verify_target(resolved)?;
    let exec_args = entry
        .exec_args
        .iter()
        .map(|arg| DesktopExecArg::Literal(arg.as_str()))
        .collect::<Vec<_>>();
    let exec = format_desktop_exec_command(&resolved.target, &exec_args);
    let name = escape_desktop_entry_value(&entry.display_name);
    let comment = escape_desktop_entry_value(&entry.description);

    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Comment={}\n\
         Exec={}\n\
         Terminal=false\n\
         Categories=Utility;\n\
         StartupNotify=false\n",
        name, comment, exec
    );

    let path = dir.join(desktop_filename(entry));
    write_executable(&path, &content)?;
    Ok(())
}

fn write_executable(path: &Path, content: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, content)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn clean_stale(dir: &Path, expected: &HashSet<String>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        if !name_str.starts_with(DESKTOP_PREFIX) || !name_str.ends_with(".desktop") {
            continue;
        }
        if expected.contains(name_str) {
            continue;
        }
        let _ = std::fs::remove_file(entry.path());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn resolved(entry: LauncherEntry, target: &Path) -> ResolvedEntry {
        ResolvedEntry {
            entry,
            target: target.to_path_buf(),
        }
    }

    fn entry(exec_args: &[&str]) -> LauncherEntry {
        LauncherEntry {
            file_stem: "shortcut-space".to_string(),
            display_name: "Open Space".to_string(),
            description: "QoL Shortcut: Open Space".to_string(),
            bundle_id: "com.qol-tools.shortcut.space".to_string(),
            exec_args: exec_args.iter().map(|arg| arg.to_string()).collect(),
            shortcut_action: None,
        }
    }

    #[test]
    fn desktop_file_quotes_binary_and_literal_args() {
        let tmp = TempDir::new().unwrap();
        let binary = tmp.path().join("qol-tray");
        let courier = tmp.path().join("qol-courier");
        std::fs::write(&binary, "").unwrap();
        std::fs::write(&courier, "").unwrap();

        write_desktop_file(
            tmp.path(),
            &resolved(entry(&["exec", "shortcut id", "path%to%tool"]), &courier),
        )
        .unwrap();

        let content =
            std::fs::read_to_string(tmp.path().join("qol-shortcut-space.desktop")).unwrap();
        assert!(content.lines().any(|line| {
            line == format!(
                "Exec=\"{}\" \"exec\" \"shortcut id\" \"path%%to%%tool\"",
                courier.display()
            )
        }));
    }

    #[test]
    fn desktop_file_keeps_qol_tray_for_command_entries() {
        let tmp = TempDir::new().unwrap();
        let binary = tmp.path().join("qol-tray");
        let courier = tmp.path().join("qol-courier");
        std::fs::write(&binary, "").unwrap();
        std::fs::write(&courier, "").unwrap();
        let command_entry = LauncherEntry {
            file_stem: "command-shortcuts-add".to_string(),
            display_name: "Add Shortcut".to_string(),
            description: String::new(),
            bundle_id: String::new(),
            exec_args: vec!["open".into(), "shortcuts/add".into()],
            shortcut_action: None,
        };

        write_desktop_file(tmp.path(), &resolved(command_entry, &binary)).unwrap();

        let content =
            std::fs::read_to_string(tmp.path().join("qol-command-shortcuts-add.desktop")).unwrap();
        assert!(content.lines().any(|line| {
            line == format!("Exec=\"{}\" \"open\" \"shortcuts/add\"", binary.display())
        }));
    }

    #[test]
    fn desktop_file_is_executable_so_cinnamon_runs_it_instead_of_opening_nemo() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let binary = tmp.path().join("qol-tray");
        let courier = tmp.path().join("qol-courier");
        std::fs::write(&binary, "").unwrap();
        std::fs::write(&courier, "").unwrap();

        write_desktop_file(
            tmp.path(),
            &resolved(entry(&["exec", "shortcut", "id"]), &courier),
        )
        .unwrap();

        let mode = std::fs::metadata(tmp.path().join("qol-shortcut-space.desktop"))
            .unwrap()
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "desktop entry must be executable, mode={mode:o}"
        );
    }

    #[test]
    fn desktop_file_refuses_a_missing_referenced_binary() {
        let tmp = TempDir::new().unwrap();
        let binary = tmp.path().join("qol-tray");
        let courier = tmp.path().join("qol-courier");
        std::fs::write(&binary, "").unwrap();

        let error = write_desktop_file(
            tmp.path(),
            &resolved(entry(&["exec", "shortcut", "id"]), &courier),
        )
        .unwrap_err();

        assert!(error.to_string().contains("missing binary"), "got: {error}");
        assert!(
            !tmp.path().join("qol-shortcut-space.desktop").exists(),
            "a dead desktop entry must not be written"
        );
    }
}
