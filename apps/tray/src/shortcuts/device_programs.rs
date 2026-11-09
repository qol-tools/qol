use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::model::{AppRef, Shortcut, ShortcutAction, ShortcutsConfig};

pub type DevicePrograms = BTreeMap<String, DeviceProgram>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceProgram {
    App(AppRef),
    BrowserOverride(AppRef),
}

pub fn load() -> Result<DevicePrograms> {
    load_from(&crate::paths::shortcut_programs_path()?)
}

pub fn load_from(path: &Path) -> Result<DevicePrograms> {
    crate::file_io::load_json_or_default(path)
}

pub fn save_to(path: &Path, programs: &DevicePrograms) -> Result<()> {
    crate::file_io::ensure_parent_dir(path)?;
    let content = serde_json::to_vec_pretty(programs)?;
    crate::file_io::atomic_write_private(path, &content)
}

pub fn record(config: &ShortcutsConfig) -> Result<()> {
    record_to(&crate::paths::shortcut_programs_path()?, config)
}

pub fn record_to(path: &Path, config: &ShortcutsConfig) -> Result<()> {
    save_to(path, &programs_of(config))
}

pub fn record_shortcut_to(path: &Path, config: &ShortcutsConfig, id: &str) -> Result<()> {
    let mut programs = load_from(path).unwrap_or_else(|error| {
        log::warn!(
            "Shortcut programs unreadable, starting empty: path={} error={:#}",
            path.display(),
            error
        );
        DevicePrograms::new()
    });
    match config
        .shortcuts
        .iter()
        .find(|shortcut| shortcut.id == id)
        .and_then(program_of)
    {
        Some(program) => {
            programs.insert(id.to_string(), program);
        }
        None => {
            programs.remove(id);
        }
    }
    save_to(path, &programs)
}

pub fn programs_of(config: &ShortcutsConfig) -> DevicePrograms {
    config
        .shortcuts
        .iter()
        .filter_map(|shortcut| Some((shortcut.id.clone(), program_of(shortcut)?)))
        .collect()
}

pub fn carries_program(shortcut: &Shortcut) -> bool {
    program_of(shortcut).is_some()
}

pub fn program_matches(shortcut: &Shortcut, device: &DevicePrograms) -> bool {
    let Some(program) = program_of(shortcut) else {
        return true;
    };
    device
        .get(&shortcut.id)
        .is_some_and(|trusted| program_key(trusted) == program_key(&program))
}

fn program_of(shortcut: &Shortcut) -> Option<DeviceProgram> {
    match &shortcut.action {
        ShortcutAction::OpenUrl {
            browser_override: Some(app),
            ..
        } => Some(DeviceProgram::BrowserOverride(app.clone())),
        ShortcutAction::LaunchApp { app } => Some(DeviceProgram::App(app.clone())),
        ShortcutAction::OpenUrl {
            browser_override: None,
            ..
        }
        | ShortcutAction::PluginAction { .. } => None,
    }
}

fn program_key(program: &DeviceProgram) -> (&'static str, (&'static str, &str)) {
    match program {
        DeviceProgram::App(app) => ("app", app_key(app)),
        DeviceProgram::BrowserOverride(app) => ("browser_override", app_key(app)),
    }
}

fn app_key(app: &AppRef) -> (&'static str, &str) {
    match app {
        AppRef::BundleId { id } => ("bundle_id", id),
        AppRef::Path { path } => ("path", path),
        AppRef::Name { name } => ("name", name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(id: &str, app: AppRef) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: id.to_string(),
            enabled: true,
            export_to_launcher: false,
            source: None,
            action: ShortcutAction::LaunchApp { app },
        }
    }

    fn open(id: &str, browser_override: Option<AppRef>) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: id.to_string(),
            enabled: true,
            export_to_launcher: false,
            source: None,
            action: ShortcutAction::OpenUrl {
                url: "https://x.io".to_string(),
                browser_override,
            },
        }
    }

    fn plugin_action(id: &str) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: id.to_string(),
            enabled: true,
            export_to_launcher: false,
            source: None,
            action: ShortcutAction::PluginAction {
                plugin_id: "qol-lights".to_string(),
                action: "toggle".to_string(),
            },
        }
    }

    fn name(value: &str) -> AppRef {
        AppRef::Name {
            name: value.to_string(),
        }
    }

    #[test]
    fn programs_of_lists_only_shortcuts_that_carry_a_program() {
        let config = ShortcutsConfig {
            shortcuts: vec![
                launch("app", name("firefox")),
                open("browser", Some(name("chromium"))),
                open("plain", None),
                plugin_action("action"),
            ],
        };

        let json = serde_json::to_value(programs_of(&config)).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "app": { "app": { "type": "name", "name": "firefox" } },
                "browser": { "browser_override": { "type": "name", "name": "chromium" } },
            })
        );
    }

    #[test]
    fn save_and_load_round_trip_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device/shortcut-programs.json");
        assert!(load_from(&path).unwrap().is_empty());
        let config = ShortcutsConfig {
            shortcuts: vec![launch("app", name("firefox"))],
        };

        record_to(&path, &config).unwrap();

        let loaded = load_from(&path).unwrap();
        assert!(program_matches(&config.shortcuts[0], &loaded));
    }

    #[test]
    fn a_shortcut_without_a_program_always_matches() {
        let device = DevicePrograms::new();

        assert!(program_matches(&open("plain", None), &device));
        assert!(program_matches(&plugin_action("action"), &device));
    }

    #[test]
    fn a_program_matches_only_an_equal_entry_of_the_same_kind() {
        let trusted = ShortcutsConfig {
            shortcuts: vec![
                launch("app", name("firefox")),
                open("browser", Some(name("firefox"))),
            ],
        };
        let device = programs_of(&trusted);
        let cases = [
            (launch("app", name("firefox")), true),
            (launch("app", name("npx")), false),
            (
                launch(
                    "app",
                    AppRef::Path {
                        path: "firefox".to_string(),
                    },
                ),
                false,
            ),
            (launch("missing", name("firefox")), false),
            (open("browser", Some(name("firefox"))), true),
            (open("browser", Some(name("npx"))), false),
            (launch("browser", name("firefox")), false),
        ];

        for (shortcut, expected) in cases {
            assert_eq!(
                program_matches(&shortcut, &device),
                expected,
                "id: {} action: {:?}",
                shortcut.id,
                shortcut.action
            );
        }
    }
}
