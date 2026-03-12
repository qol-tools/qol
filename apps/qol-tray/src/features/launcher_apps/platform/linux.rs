use super::super::LauncherEntry;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DESKTOP_PREFIX: &str = "qol-";

pub(super) fn sync(entries: &[LauncherEntry], binary_path: &Path) -> Result<()> {
    let dir = apps_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create applications dir {}", dir.display()))?;

    let expected: HashSet<String> = entries.iter().map(desktop_filename).collect();

    for entry in entries {
        write_desktop_file(&dir, entry, binary_path)?;
    }

    clean_stale(&dir, &expected)?;
    Ok(())
}

fn apps_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .context("Could not determine local data directory")
        .map(|p| p.join("applications"))
}

fn desktop_filename(entry: &LauncherEntry) -> String {
    format!("{}{}.desktop", DESKTOP_PREFIX, entry.file_stem)
}

fn write_desktop_file(dir: &Path, entry: &LauncherEntry, binary_path: &Path) -> Result<()> {
    let escaped_bin = exec_escape_path(&binary_path.display().to_string());
    let name = desktop_entry_escape(&entry.display_name);
    let comment = desktop_entry_escape(&entry.description);
    let args: String = entry.exec_args.join(" ");

    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Comment={}\n\
         Exec=\"{}\" {}\n\
         Terminal=false\n\
         Categories=Utility;\n\
         StartupNotify=false\n",
        name, comment, escaped_bin, args
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

fn exec_escape_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '%' => out.push_str("%%"),
            _ => out.push(ch),
        }
    }
    out
}
