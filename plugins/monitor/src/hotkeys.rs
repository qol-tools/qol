use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use qol_headless::DoctorCheckResult;
use qol_hotkeys::grammar;

pub const PLUGIN_ID: &str = "plugin-monitor";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostHotkeyBinding {
    pub id: String,
    pub key: String,
    #[serde(alias = "plugin_id")]
    pub plugin_uid: String,
    pub action: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostHotkeyConfig {
    #[serde(default)]
    pub hotkeys: Vec<HostHotkeyBinding>,
}

pub fn load_host_hotkeys(path: &Path) -> Result<Option<HostHotkeyConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read(path)
        .with_context(|| format!("failed to read host hotkey config {}", path.display()))?;
    let config: HostHotkeyConfig = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse host hotkey config {}", path.display()))?;
    Ok(Some(config))
}

pub fn monitor_bindings(config: &HostHotkeyConfig) -> Vec<&HostHotkeyBinding> {
    config
        .hotkeys
        .iter()
        .filter(|binding| binding.plugin_uid == PLUGIN_ID)
        .collect()
}

pub fn duplicate_enabled_chord(config: &HostHotkeyConfig) -> Option<String> {
    let mut registered = HashSet::new();
    config
        .hotkeys
        .iter()
        .filter(|binding| binding.enabled)
        .filter_map(|binding| grammar::parse(&binding.key).map(|hotkey| (binding, hotkey)))
        .find_map(|(binding, hotkey)| {
            let canonical = grammar::format(&hotkey).unwrap_or_else(|| binding.key.clone());
            (!registered.insert(canonical)).then(|| binding.key.clone())
        })
}

pub fn hotkey_registration_result(config_root: &Path) -> Result<DoctorCheckResult> {
    let Some(config) = load_host_hotkeys(&crate::config::hotkeys_path(config_root)?)? else {
        return Ok(DoctorCheckResult::warn(
            "hotkey_bindings",
            "no host hotkey config exists; brightness hotkeys are unbound",
        )
        .with_fix("Bind brightness-up and brightness-down in the qol-tray hotkey settings."));
    };
    if let Some(duplicate) = duplicate_enabled_chord(&config) {
        return Ok(DoctorCheckResult::fail(
            "hotkey_bindings",
            format!("the enabled chord `{duplicate}` is bound to more than one action"),
        )
        .with_fix("Reassign the duplicate chord in the qol-tray hotkey settings."));
    }
    let monitor = monitor_bindings(&config);
    let continuous = monitor
        .iter()
        .filter(|binding| {
            binding.enabled
                && (binding.action == "brightness-up" || binding.action == "brightness-down")
        })
        .count();
    match continuous {
        2 => Ok(DoctorCheckResult::ok(
            "hotkey_bindings",
            format!(
                "brightness-up and brightness-down are bound ({} of {} bindings belong to this plugin)",
                monitor.len(),
                config.hotkeys.len()
            ),
        )),
        1 => Ok(DoctorCheckResult::warn(
            "hotkey_bindings",
            "only one of brightness-up or brightness-down is bound",
        )
        .with_fix("Bind both brightness hotkeys in the qol-tray hotkey settings.")),
        _ => Ok(DoctorCheckResult::warn(
            "hotkey_bindings",
            "no enabled brightness hotkey bindings found",
        )
        .with_fix("Bind brightness-up and brightness-down in the qol-tray hotkey settings.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        id: &str,
        key: &str,
        plugin_uid: &str,
        action: &str,
        enabled: bool,
    ) -> HostHotkeyBinding {
        HostHotkeyBinding {
            id: id.into(),
            key: key.into(),
            plugin_uid: plugin_uid.into(),
            action: action.into(),
            enabled,
        }
    }

    #[test]
    fn parses_the_host_hotkey_config_shape() {
        let json = r#"{
            "hotkeys": [
                {"id": "h1", "key": "ctrl+shift+b", "plugin_uid": "plugin-monitor", "action": "brightness-up", "enabled": true},
                {"id": "h2", "key": "super+f9", "plugin_id": "plugin-monitor", "action": "brightness-down", "enabled": true}
            ]
        }"#;
        let config: HostHotkeyConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.hotkeys.len(), 2);
        assert!(config.hotkeys[1].plugin_uid == "plugin-monitor");
    }

    #[test]
    fn duplicate_chords_are_detected_across_aliases_and_case() {
        let config = HostHotkeyConfig {
            hotkeys: vec![
                binding(
                    "h1",
                    "ctrl+shift+b",
                    "plugin-monitor",
                    "brightness-up",
                    true,
                ),
                binding(
                    "h2",
                    "CTRL + SHIFT + B",
                    "plugin-other",
                    "other-action",
                    true,
                ),
            ],
        };
        assert_eq!(
            duplicate_enabled_chord(&config),
            Some("CTRL + SHIFT + B".to_string())
        );
    }

    #[test]
    fn disabled_bindings_do_not_collide() {
        let config = HostHotkeyConfig {
            hotkeys: vec![
                binding(
                    "h1",
                    "ctrl+shift+b",
                    "plugin-monitor",
                    "brightness-up",
                    true,
                ),
                binding("h2", "ctrl+shift+b", "plugin-other", "other-action", false),
            ],
        };
        assert_eq!(duplicate_enabled_chord(&config), None);
    }

    #[test]
    fn monitor_bindings_filter_by_plugin_id() {
        let config = HostHotkeyConfig {
            hotkeys: vec![
                binding(
                    "h1",
                    "ctrl+shift+b",
                    "plugin-monitor",
                    "brightness-up",
                    true,
                ),
                binding("h2", "ctrl+shift+b", "plugin-other", "other-action", true),
            ],
        };
        let monitor = monitor_bindings(&config);
        assert_eq!(monitor.len(), 1);
        assert_eq!(monitor[0].action, "brightness-up");
    }

    #[test]
    fn registration_results_cover_bound_partial_and_missing() {
        let root = tempfile::tempdir().unwrap();
        let hotkeys = crate::config::hotkeys_path(root.path()).unwrap();
        std::fs::create_dir_all(hotkeys.parent().unwrap()).unwrap();

        std::fs::write(
            &hotkeys,
            serde_json::to_string(&HostHotkeyConfig {
                hotkeys: vec![
                    binding(
                        "h1",
                        "ctrl+shift+b",
                        "plugin-monitor",
                        "brightness-up",
                        true,
                    ),
                    binding(
                        "h2",
                        "ctrl+shift+d",
                        "plugin-monitor",
                        "brightness-down",
                        true,
                    ),
                ],
            })
            .unwrap(),
        )
        .unwrap();
        let result = hotkey_registration_result(root.path()).unwrap();
        assert_eq!(result.id, "hotkey_bindings");
        assert_eq!(result.status, qol_headless::DoctorStatus::Ok);
        assert!(result.message.contains("brightness-up"));

        std::fs::write(
            &hotkeys,
            serde_json::to_string(&HostHotkeyConfig {
                hotkeys: vec![binding(
                    "h1",
                    "ctrl+shift+b",
                    "plugin-monitor",
                    "brightness-up",
                    true,
                )],
            })
            .unwrap(),
        )
        .unwrap();
        let result = hotkey_registration_result(root.path()).unwrap();
        assert_eq!(result.status, qol_headless::DoctorStatus::Warn);

        std::fs::remove_file(&hotkeys).unwrap();
        let result = hotkey_registration_result(root.path()).unwrap();
        assert_eq!(result.status, qol_headless::DoctorStatus::Warn);
        assert!(result.message.contains("unbound"));
    }

    #[test]
    fn duplicate_chord_is_a_doctor_failure() {
        let root = tempfile::tempdir().unwrap();
        let hotkeys = crate::config::hotkeys_path(root.path()).unwrap();
        std::fs::create_dir_all(hotkeys.parent().unwrap()).unwrap();
        std::fs::write(
            &hotkeys,
            serde_json::to_string(&HostHotkeyConfig {
                hotkeys: vec![
                    binding(
                        "h1",
                        "ctrl+shift+b",
                        "plugin-monitor",
                        "brightness-up",
                        true,
                    ),
                    binding("h2", "ctrl+shift+b", "plugin-other", "other-action", true),
                ],
            })
            .unwrap(),
        )
        .unwrap();
        let result = hotkey_registration_result(root.path()).unwrap();
        assert_eq!(result.status, qol_headless::DoctorStatus::Fail);
        assert!(result.message.contains("ctrl+shift+b"));
        assert!(result.fix.is_some());
    }
}
