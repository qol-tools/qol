use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::fs_util::{current_os_subdir, list_profile_dirs, write_json_atomic};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_139ToV3_140ShortcutDevicePrograms;

const NAME: &str = "v3.139-to-v3.140-shortcut-device-programs";

const SHORTCUTS_FILE: &str = "shortcuts.json";
const PROGRAMS_FILE: &str = "shortcut-programs.json";

impl FileMigration for V3_139ToV3_140ShortcutDevicePrograms {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        Ok(!pending_seeds(config_dir)?.is_empty())
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let mut report = MigrationReport {
            name: NAME.to_string(),
            archived: Vec::new(),
        };
        for (source, destination) in pending_seeds(config_dir)? {
            let programs = programs_of(&read_shortcuts(&source)?);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            write_json_atomic(&destination, &Value::Object(programs))?;
            report.archived.push(destination);
        }
        Ok(report)
    }
}

fn pending_seeds(config_dir: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut pending = Vec::new();
    for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
        let source = profile_dir
            .join("os")
            .join(current_os_subdir())
            .join(SHORTCUTS_FILE);
        let destination = profile_dir.join("device").join(PROGRAMS_FILE);
        if source.is_file() && !destination.exists() {
            pending.push((source, destination));
        }
    }
    Ok(pending)
}

fn read_shortcuts(path: &Path) -> Result<Vec<Value>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let Ok(mut document) = serde_json::from_str::<Value>(&content) else {
        return Ok(Vec::new());
    };
    match document.get_mut("shortcuts").map(Value::take) {
        Some(Value::Array(shortcuts)) => Ok(shortcuts),
        Some(_) | None => Ok(Vec::new()),
    }
}

fn programs_of(shortcuts: &[Value]) -> Map<String, Value> {
    shortcuts.iter().filter_map(program_entry).collect()
}

fn program_entry(shortcut: &Value) -> Option<(String, Value)> {
    let id = shortcut.get("id")?.as_str()?;
    let action = shortcut.get("action")?;
    let (key, app) = match action.get("type")?.as_str()? {
        "launch_app" => ("app", action.get("app")?),
        "open_url" => ("browser_override", action.get("browser_override")?),
        _ => return None,
    };
    if !app.is_object() {
        return None;
    }
    let mut entry = Map::new();
    entry.insert(key.to_string(), app.clone());
    Some((id.to_string(), Value::Object(entry)))
}

#[cfg(test)]
mod tests;
