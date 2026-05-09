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

#[cfg(test)]
mod tests {
    use super::super::model::ShortcutAction;
    use super::*;

    fn url_shortcut(id: &str, name: &str, url: &str) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            export_to_launcher: false,
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
}
