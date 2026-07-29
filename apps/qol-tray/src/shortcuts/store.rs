use anyhow::Result;
use std::fmt;
use std::path::Path;

use super::model::{Shortcut, ShortcutsConfig};
use super::plugin_manifest;
use super::validation;

pub fn load() -> Result<ShortcutsConfig> {
    let path = crate::paths::shortcuts_path()?;
    load_from(&path)
}

fn load_from(path: &Path) -> Result<ShortcutsConfig> {
    if !path.exists() {
        log::debug!(
            "Shortcuts config missing; using default: {}",
            path.display()
        );
        return Ok(ShortcutsConfig::default());
    }
    match crate::file_io::load_json_or_default::<ShortcutsConfig>(&path) {
        Ok(config) => {
            log::debug!(
                "Shortcuts config loaded: count={} path={}",
                config.shortcuts.len(),
                path.display()
            );
            Ok(config)
        }
        Err(error) => {
            log::error!(
                "Shortcuts config load failed: path={} error={:#}",
                path.display(),
                error
            );
            Err(error)
        }
    }
}

pub fn save(config: &ShortcutsConfig) -> Result<()> {
    let path = crate::paths::shortcuts_path()?;
    save_to(&path, config)
}

fn save_to(path: &Path, config: &ShortcutsConfig) -> Result<()> {
    match crate::file_io::write_pretty_json(path, config) {
        Ok(()) => {
            log::info!(
                "Shortcuts config saved: count={} path={}",
                config.shortcuts.len(),
                path.display()
            );
            Ok(())
        }
        Err(error) => {
            log::error!(
                "Shortcuts config save failed: path={} error={:#}",
                path.display(),
                error
            );
            Err(error)
        }
    }
}

#[derive(Debug)]
pub enum MutationError {
    Load(anyhow::Error),
    Rejected(String),
    Save(anyhow::Error),
}

impl fmt::Display for MutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => write!(formatter, "failed to load shortcuts: {error}"),
            Self::Rejected(error) => formatter.write_str(error),
            Self::Save(error) => write!(formatter, "failed to save shortcuts: {error}"),
        }
    }
}

impl std::error::Error for MutationError {}

pub fn create_persisted(shortcut: Shortcut) -> Result<ShortcutsConfig, MutationError> {
    let path = crate::paths::shortcuts_path().map_err(MutationError::Load)?;
    create_at(&path, shortcut)
}

pub fn update_persisted(shortcut: Shortcut) -> Result<ShortcutsConfig, MutationError> {
    let path = crate::paths::shortcuts_path().map_err(MutationError::Load)?;
    mutate_at(&path, |config| update(config, shortcut))
}

pub fn remove_persisted(id: &str) -> Result<ShortcutsConfig, MutationError> {
    let path = crate::paths::shortcuts_path().map_err(MutationError::Load)?;
    mutate_at(&path, |config| remove(config, id))
}

fn create_at(path: &Path, shortcut: Shortcut) -> Result<ShortcutsConfig, MutationError> {
    mutate_at(path, |config| add(config, shortcut))
}

fn mutate_at(
    path: &Path,
    mutate: impl FnOnce(&mut ShortcutsConfig) -> Result<(), String>,
) -> Result<ShortcutsConfig, MutationError> {
    let mut config = load_from(path).map_err(MutationError::Load)?;
    mutate(&mut config).map_err(MutationError::Rejected)?;
    save_to(path, &config).map_err(MutationError::Save)?;
    Ok(config)
}

pub fn find_by_id(config: &ShortcutsConfig, id: &str) -> Option<Shortcut> {
    config.shortcuts.iter().find(|s| s.id == id).cloned()
}

pub fn add(config: &mut ShortcutsConfig, shortcut: Shortcut) -> Result<(), String> {
    validation::validate_shortcut(&shortcut)?;
    if config.shortcuts.iter().any(|s| s.id == shortcut.id) {
        return Err(format!("shortcut with id '{}' already exists", shortcut.id));
    }
    log::info!(
        "Shortcut added: id={} action={}",
        shortcut.id,
        shortcut.action.kind()
    );
    config.shortcuts.push(shortcut);
    Ok(())
}

pub fn update(config: &mut ShortcutsConfig, shortcut: Shortcut) -> Result<(), String> {
    validation::validate_shortcut(&shortcut)?;
    let existing = match config.shortcuts.iter_mut().find(|s| s.id == shortcut.id) {
        Some(e) => e,
        None => return Err(format!("shortcut '{}' not found", shortcut.id)),
    };
    log::info!(
        "Shortcut updated: id={} action={} enabled={}",
        shortcut.id,
        shortcut.action.kind(),
        shortcut.enabled
    );
    *existing = shortcut;
    Ok(())
}

pub fn remove(config: &mut ShortcutsConfig, id: &str) -> Result<(), String> {
    let len_before = config.shortcuts.len();
    config.shortcuts.retain(|s| s.id != id);
    if config.shortcuts.len() == len_before {
        return Err(format!("shortcut '{}' not found", id));
    }
    log::info!("Shortcut removed: id={}", id);
    Ok(())
}

pub fn reconcile_plugin_shortcuts<'a>(
    plugins: impl IntoIterator<Item = &'a crate::plugins::Plugin>,
) -> Result<bool> {
    let mut config = load()?;
    let changed = plugin_manifest::reconcile(&mut config, plugins);
    if changed {
        save(&config)?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::super::model::ShortcutAction;
    use super::*;
    use std::sync::{Arc, Barrier};

    fn url_shortcut(id: &str, name: &str, url: &str) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            export_to_launcher: false,
            source: None,
            action: ShortcutAction::OpenUrl {
                url: url.to_string(),
                browser_override: None,
            },
        }
    }

    #[test]
    fn add_pushes_a_validated_shortcut() {
        let mut cfg = ShortcutsConfig::default();
        add(&mut cfg, url_shortcut("docs", "Docs", "https://docs.io")).unwrap();
        assert_eq!(cfg.shortcuts.len(), 1);
        assert_eq!(cfg.shortcuts[0].id, "docs");
    }

    #[test]
    fn add_rejects_duplicate_id() {
        let mut cfg = ShortcutsConfig::default();
        add(&mut cfg, url_shortcut("docs", "Docs", "https://x.io")).unwrap();
        let err = add(&mut cfg, url_shortcut("docs", "Other", "https://y.io")).unwrap_err();
        assert!(err.contains("already exists"), "err: {err}");
        assert_eq!(cfg.shortcuts.len(), 1, "duplicate must not be appended");
    }

    #[test]
    fn add_rejects_invalid_payload_without_mutating_config() {
        let mut cfg = ShortcutsConfig::default();
        let err = add(&mut cfg, url_shortcut("ok", "", "https://x.io")).unwrap_err();
        assert!(err.contains("name"), "err: {err}");
        assert!(
            cfg.shortcuts.is_empty(),
            "invalid add must not mutate state"
        );
    }

    #[test]
    fn update_replaces_matching_id() {
        let mut cfg = ShortcutsConfig::default();
        add(&mut cfg, url_shortcut("docs", "Docs", "https://docs.io")).unwrap();
        update(
            &mut cfg,
            url_shortcut("docs", "Manual", "https://manual.io"),
        )
        .unwrap();
        assert_eq!(cfg.shortcuts[0].name, "Manual");
        match &cfg.shortcuts[0].action {
            ShortcutAction::OpenUrl { url, .. } => assert_eq!(url, "https://manual.io"),
            _ => panic!("unexpected action"),
        }
    }

    #[test]
    fn update_rejects_unknown_id() {
        let mut cfg = ShortcutsConfig::default();
        let err = update(&mut cfg, url_shortcut("nope", "X", "https://x.io")).unwrap_err();
        assert!(err.contains("not found"), "err: {err}");
    }

    #[test]
    fn update_rejects_invalid_payload_without_mutating() {
        let mut cfg = ShortcutsConfig::default();
        add(&mut cfg, url_shortcut("docs", "Docs", "https://docs.io")).unwrap();
        let err = update(&mut cfg, url_shortcut("docs", "", "https://docs.io")).unwrap_err();
        assert!(err.contains("name"), "err: {err}");
        assert_eq!(
            cfg.shortcuts[0].name, "Docs",
            "state must not be partially updated"
        );
    }

    #[test]
    fn remove_drops_matching_id() {
        let mut cfg = ShortcutsConfig::default();
        add(&mut cfg, url_shortcut("a", "A", "https://x.io")).unwrap();
        add(&mut cfg, url_shortcut("b", "B", "https://y.io")).unwrap();
        remove(&mut cfg, "a").unwrap();
        assert_eq!(cfg.shortcuts.len(), 1);
        assert_eq!(cfg.shortcuts[0].id, "b");
    }

    #[test]
    fn remove_rejects_unknown_id() {
        let mut cfg = ShortcutsConfig::default();
        let err = remove(&mut cfg, "ghost").unwrap_err();
        assert!(err.contains("not found"), "err: {err}");
    }

    #[test]
    fn find_by_id_returns_clone_for_match_and_none_for_miss() {
        let mut cfg = ShortcutsConfig::default();
        add(&mut cfg, url_shortcut("docs", "Docs", "https://docs.io")).unwrap();
        assert_eq!(
            find_by_id(&cfg, "docs").map(|s| s.id),
            Some("docs".to_string())
        );
        assert!(find_by_id(&cfg, "missing").is_none());
    }

    #[test]
    fn concurrent_creates_preserve_every_successful_write() {
        const COUNT: usize = 40;
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("shortcuts.json"));
        let barrier = Arc::new(Barrier::new(COUNT));
        let handles = (0..COUNT)
            .map(|index| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let id = format!("race-{index}");
                    create_at(&path, url_shortcut(&id, &id, "https://x.io"))
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().unwrap().unwrap();
        }

        let config = load_from(&path).unwrap();
        assert_eq!(config.shortcuts.len(), COUNT);
    }
}
