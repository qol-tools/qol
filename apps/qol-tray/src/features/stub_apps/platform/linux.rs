use super::super::StubInput;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) fn sync(stubs: &[StubInput], binary_path: &Path) -> Result<()> {
    let dir = apps_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create applications dir {}", dir.display()))?;

    let expected: HashSet<String> = stubs.iter().map(|s| desktop_filename(s)).collect();

    for stub in stubs {
        write_desktop_file(&dir, stub, binary_path)?;
    }

    clean_stale(&dir, &expected)?;
    Ok(())
}

fn apps_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .context("Could not determine local data directory")
        .map(|p| p.join("applications"))
}

fn desktop_filename(stub: &StubInput) -> String {
    format!("qol-action-{}-{}.desktop", stub.plugin_id, stub.action_id)
}

fn write_desktop_file(dir: &Path, stub: &StubInput, binary_path: &Path) -> Result<()> {
    let escaped_path = exec_escape_path(&binary_path.display().to_string());
    let name = desktop_entry_escape(&stub.action_label);
    let comment = desktop_entry_escape(&format!(
        "QoL Tray: {} - {}",
        stub.plugin_name, stub.action_label
    ));

    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Comment={}\n\
         Exec=\"{}\" exec {} {}\n\
         Terminal=false\n\
         Categories=Utility;\n\
         StartupNotify=false\n",
        name, comment, escaped_path, stub.plugin_id, stub.action_id
    );

    let path = dir.join(desktop_filename(stub));
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
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !name_str.starts_with("qol-action-") || !name_str.ends_with(".desktop") {
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
