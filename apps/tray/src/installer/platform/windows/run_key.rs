use anyhow::{Context, Result};
use qol_platform::native::registry::{read_binary, Hive};
use std::path::{Path, PathBuf};

use super::registry;
use crate::installer::platform::windows_rules::run_key::{
    approval_disabled, format_command, legacy_startup_file, parse_command, read_legacy_cmd,
    remove_legacy_startup_file,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "qol-tray";

pub(in crate::installer) fn location() -> PathBuf {
    PathBuf::from(format!(r"HKCU\{RUN_KEY}\{VALUE_NAME}"))
}

pub(in crate::installer) fn read() -> Result<Option<PathBuf>> {
    if let Some(command) = registry::text(RUN_KEY, VALUE_NAME)? {
        if disabled_in_startup_apps()? {
            return Ok(None);
        }
        return Ok(parse_command(&command));
    }
    let Some(legacy) = legacy_startup_path() else {
        return Ok(None);
    };
    read_legacy_cmd(&legacy)
}

pub(in crate::installer) fn write(binary: &Path) -> Result<()> {
    registry::set(
        RUN_KEY,
        VALUE_NAME,
        &registry::Value::Text(format_command(binary)),
    )?;
    remove_legacy()
}

pub(in crate::installer) fn remove() -> Result<()> {
    registry::delete_value(RUN_KEY, VALUE_NAME)?;
    remove_legacy()
}

fn disabled_in_startup_apps() -> Result<bool> {
    let state = read_binary(Hive::CurrentUser, STARTUP_APPROVED_KEY, VALUE_NAME)
        .with_context(|| format!("failed to read HKCU\\{STARTUP_APPROVED_KEY}\\{VALUE_NAME}"))?;
    Ok(state.is_some_and(|state| approval_disabled(&state)))
}

fn legacy_startup_path() -> Option<PathBuf> {
    let app_data = std::env::var_os("APPDATA")?;
    Some(legacy_startup_file(Path::new(&app_data)))
}

fn remove_legacy() -> Result<()> {
    match legacy_startup_path() {
        Some(path) => remove_legacy_startup_file(&path),
        None => Ok(()),
    }
}
