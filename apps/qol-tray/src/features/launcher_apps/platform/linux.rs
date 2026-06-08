use super::super::LauncherEntry;
use crate::installer::desktop_entry::{format_desktop_exec_command, DesktopExecArg};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DESKTOP_PREFIX: &str = "qol-";

pub(super) fn sync(entries: &[LauncherEntry], binary_path: &Path) -> Result<()> {
    let dir = apps_dir().context("Could not determine local data directory")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create applications dir {}", dir.display()))?;

    let expected: HashSet<String> = entries.iter().map(desktop_filename).collect();

    for entry in entries {
        write_desktop_file(&dir, entry, binary_path)?;
    }

    clean_stale(&dir, &expected)?;
    Ok(())
}

pub(super) fn apps_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|p| p.join("applications"))
}

fn desktop_filename(entry: &LauncherEntry) -> String {
    format!("{}{}.desktop", DESKTOP_PREFIX, entry.file_stem)
}

fn write_desktop_file(dir: &Path, entry: &LauncherEntry, binary_path: &Path) -> Result<()> {
    let exec_args = entry
        .exec_args
        .iter()
        .map(|arg| DesktopExecArg::Literal(arg.as_str()))
        .collect::<Vec<_>>();
    let exec = format_desktop_exec_command(binary_path, &exec_args);
    let name = desktop_entry_escape(&entry.display_name);
    let comment = desktop_entry_escape(&entry.description);

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
    std::fs::write(&path, content)?;
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

fn desktop_entry_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
        let binary = Path::new("/tmp/qol tray/qol-tray");

        write_desktop_file(
            tmp.path(),
            &entry(&["exec", "shortcut id", "path%to%tool"]),
            binary,
        )
        .unwrap();

        let content =
            std::fs::read_to_string(tmp.path().join("qol-shortcut-space.desktop")).unwrap();
        assert!(content.lines().any(|line| {
            line == "Exec=\"/tmp/qol tray/qol-tray\" \"exec\" \"shortcut id\" \"path%%to%%tool\""
        }));
    }
}
