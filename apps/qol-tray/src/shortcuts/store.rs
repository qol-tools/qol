use anyhow::Result;

use super::model::{Shortcut, ShortcutsConfig};
use super::validation;

pub fn load() -> Result<ShortcutsConfig> {
    let path = crate::paths::shortcuts_path()?;
    crate::file_io::load_json_or_default(&path)
}

pub fn save(config: &ShortcutsConfig) -> Result<()> {
    let path = crate::paths::shortcuts_path()?;
    crate::file_io::write_pretty_json(&path, config)
}

pub fn find_by_id(config: &ShortcutsConfig, id: &str) -> Option<Shortcut> {
    config.shortcuts.iter().find(|s| s.id == id).cloned()
}

pub fn add(config: &mut ShortcutsConfig, shortcut: Shortcut) -> Result<(), String> {
    validation::validate_shortcut(&shortcut)?;
    if config.shortcuts.iter().any(|s| s.id == shortcut.id) {
        return Err(format!("shortcut with id '{}' already exists", shortcut.id));
    }
    config.shortcuts.push(shortcut);
    Ok(())
}

pub fn update(config: &mut ShortcutsConfig, shortcut: Shortcut) -> Result<(), String> {
    validation::validate_shortcut(&shortcut)?;
    let existing = match config.shortcuts.iter_mut().find(|s| s.id == shortcut.id) {
        Some(e) => e,
        None => return Err(format!("shortcut '{}' not found", shortcut.id)),
    };
    *existing = shortcut;
    Ok(())
}

pub fn remove(config: &mut ShortcutsConfig, id: &str) -> Result<(), String> {
    let len_before = config.shortcuts.len();
    config.shortcuts.retain(|s| s.id != id);
    if config.shortcuts.len() == len_before {
        return Err(format!("shortcut '{}' not found", id));
    }
    Ok(())
}
