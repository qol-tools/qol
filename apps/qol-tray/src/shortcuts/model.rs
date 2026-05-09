use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShortcutsConfig {
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shortcut {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub export_to_launcher: bool,
    pub action: ShortcutAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ShortcutAction {
    #[serde(rename = "open_url")]
    OpenUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        browser_override: Option<AppRef>,
    },
    #[serde(rename = "launch_app")]
    LaunchApp { app: AppRef },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppRef {
    #[serde(rename = "bundle_id")]
    BundleId { id: String },
    #[serde(rename = "path")]
    Path { path: String },
    #[serde(rename = "name")]
    Name { name: String },
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_url_action_round_trips_with_browser_override() {
        let action = ShortcutAction::OpenUrl {
            url: "https://x.io".into(),
            browser_override: Some(AppRef::BundleId {
                id: "com.firefox".into(),
            }),
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["type"], "open_url");
        assert_eq!(json["url"], "https://x.io");
        assert_eq!(json["browser_override"]["type"], "bundle_id");
        let parsed: ShortcutAction = serde_json::from_value(json).unwrap();
        match parsed {
            ShortcutAction::OpenUrl {
                url,
                browser_override: Some(AppRef::BundleId { id }),
            } => {
                assert_eq!(url, "https://x.io");
                assert_eq!(id, "com.firefox");
            }
            _ => panic!("variant or override lost in round-trip"),
        }
    }

    #[test]
    fn open_url_action_omits_none_browser_override() {
        let action = ShortcutAction::OpenUrl {
            url: "https://x.io".into(),
            browser_override: None,
        };
        let json = serde_json::to_value(&action).unwrap();
        assert!(
            json.get("browser_override").is_none(),
            "None must serialize as field-omitted, got: {json:?}",
        );
    }

    #[test]
    fn launch_app_round_trips_for_each_app_ref_variant() {
        let cases = [
            AppRef::BundleId {
                id: "com.example".into(),
            },
            AppRef::Path {
                path: "/Applications/Foo.app".into(),
            },
            AppRef::Name { name: "Foo".into() },
        ];
        for app in cases {
            let action = ShortcutAction::LaunchApp { app: app.clone() };
            let json = serde_json::to_value(&action).unwrap();
            assert_eq!(json["type"], "launch_app");
            let parsed: ShortcutAction = serde_json::from_value(json).unwrap();
            let after = match parsed {
                ShortcutAction::LaunchApp { app } => app,
                _ => panic!("variant lost"),
            };
            let before = serde_json::to_value(&app).unwrap();
            let after_json = serde_json::to_value(&after).unwrap();
            assert_eq!(before, after_json, "AppRef variant lost in round-trip");
        }
    }

    #[test]
    fn shortcut_enabled_defaults_to_true_when_field_omitted() {
        let json = serde_json::json!({
            "id": "x",
            "name": "X",
            "action": {
                "type": "launch_app",
                "app": { "type": "name", "name": "X" }
            }
        });
        let parsed: Shortcut = serde_json::from_value(json).unwrap();
        assert!(parsed.enabled, "enabled default must be true");
        assert!(
            !parsed.export_to_launcher,
            "export_to_launcher default false"
        );
    }

    #[test]
    fn shortcuts_config_default_is_empty_list() {
        let cfg = ShortcutsConfig::default();
        assert!(cfg.shortcuts.is_empty());
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json, serde_json::json!({ "shortcuts": [] }));
    }
}
