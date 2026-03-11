use super::super::LauncherEntry;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) fn sync(entries: &[LauncherEntry], binary_path: &Path) -> Result<()> {
    let dir = apps_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create launcher apps dir {}", dir.display()))?;

    let expected: HashSet<String> = entries.iter().map(app_dirname).collect();

    for entry in entries {
        let app_dir = dir.join(app_dirname(entry));
        write_app_bundle(&app_dir, entry, binary_path)?;
    }

    clean_stale(&dir, &expected)?;
    Ok(())
}

fn apps_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join("Applications").join("QoL"))
}

fn app_dirname(entry: &LauncherEntry) -> String {
    format!("{}.app", entry.display_name)
}

fn write_app_bundle(app_dir: &Path, entry: &LauncherEntry, binary_path: &Path) -> Result<()> {
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let run_path = macos_dir.join("run");
    let plist_path = contents_dir.join("Info.plist");

    let expected_script = build_script(binary_path, entry);
    let expected_plist = build_info_plist(entry);

    let script_ok = run_path
        .is_file()
        .then(|| std::fs::read_to_string(&run_path).ok())
        .flatten()
        .is_some_and(|s| s == expected_script);
    let plist_ok = plist_path
        .is_file()
        .then(|| std::fs::read_to_string(&plist_path).ok())
        .flatten()
        .is_some_and(|s| s == expected_plist);

    if script_ok && plist_ok && is_executable(&run_path) {
        return Ok(());
    }

    std::fs::create_dir_all(&macos_dir)?;
    std::fs::write(&plist_path, &expected_plist)?;
    write_executable(&run_path, &expected_script)?;
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn build_info_plist(entry: &LauncherEntry) -> String {
    let name = xml_escape(&entry.display_name);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         <key>CFBundleExecutable</key><string>run</string>\n\
         <key>CFBundleName</key><string>{}</string>\n\
         <key>CFBundleIdentifier</key><string>{}</string>\n\
         <key>CFBundlePackageType</key><string>APPL</string>\n\
         <key>CFBundleVersion</key><string>1</string>\n\
         <key>CFBundleShortVersionString</key><string>1</string>\n\
         <key>LSUIElement</key><true/>\n\
         </dict>\n\
         </plist>\n",
        name, entry.bundle_id
    )
}

fn write_executable(path: &Path, script: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn build_script(binary_path: &Path, entry: &LauncherEntry) -> String {
    let bin = shell_escape_single_quote(&binary_path.display().to_string());
    let args: String = entry
        .exec_args
        .iter()
        .map(|a| format!("'{}'", shell_escape_single_quote(a)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("#!/bin/sh\nexec '{}' {}\n", bin, args)
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
        if !name_str.ends_with(".app") {
            continue;
        }
        if expected.contains(name_str) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
    Ok(())
}

fn shell_escape_single_quote(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
