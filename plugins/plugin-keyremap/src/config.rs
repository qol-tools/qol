use serde::{Deserialize, Serialize};

const PLUGIN_NAMES: &[&str] = &["plugin-keyremap", "keyremap"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RemapConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub excluded_apps: Vec<String>,
    pub key_rules: Vec<KeyRule>,
    pub mouse_rules: Vec<MouseRule>,
    pub scroll_rules: Vec<ScrollRule>,
}

fn default_enabled() -> bool {
    true
}

fn builtin_defaults() -> RemapConfig {
    serde_json::from_str(include_str!("../config/default.json"))
        .expect("embedded default config must parse")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyRule {
    Batch {
        from_mods: Vec<String>,
        to_mods: Vec<String>,
        keys: Vec<String>,
    },
    Single {
        from_mods: Vec<String>,
        from_key: String,
        to_mods: Vec<String>,
        to_key: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseRule {
    pub from_mods: Vec<String>,
    pub button: String,
    pub to_mods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollRule {
    pub from_mods: Vec<String>,
    pub to_mods: Vec<String>,
}

pub fn load_config() -> RemapConfig {
    let paths = qol_plugin_api::config::plugin_config_paths(PLUGIN_NAMES);
    let has_file = paths.iter().any(|p| p.exists());

    let config = if has_file {
        qol_plugin_api::config::load_plugin_config(PLUGIN_NAMES)
    } else {
        let defaults = builtin_defaults();
        if let Some(path) = paths.first() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match serde_json::to_string_pretty(&defaults) {
                Ok(json) => {
                    let _ = std::fs::write(path, json);
                    eprintln!("[keyremap] wrote default config to {}", path.display());
                }
                Err(e) => eprintln!("[keyremap] failed to write default config: {e}"),
            }
        }
        defaults
    };

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let json = include_str!("../config/default.json");
        let config: RemapConfig = serde_json::from_str(json).expect("default config should parse");
        assert!(config.enabled);
        assert!(!config.excluded_apps.is_empty());
        assert!(!config.key_rules.is_empty());
        assert!(!config.mouse_rules.is_empty());
        assert!(!config.scroll_rules.is_empty());
    }

    #[test]
    fn parse_empty_config() {
        let config: RemapConfig =
            serde_json::from_str("{}").expect("empty config should parse");
        assert!(config.enabled);
        assert!(config.excluded_apps.is_empty());
        assert!(config.key_rules.is_empty());
    }

    #[test]
    fn parse_partial_config() {
        let json = r#"{ "excluded_apps": ["com.example.app"] }"#;
        let config: RemapConfig = serde_json::from_str(json).expect("partial config should parse");
        assert_eq!(config.excluded_apps.len(), 1);
        assert!(config.key_rules.is_empty());
    }

    #[test]
    fn builtin_defaults_are_complete() {
        let config = builtin_defaults();
        assert!(config.enabled);
        assert!(!config.key_rules.is_empty());
        assert!(!config.mouse_rules.is_empty());
        assert!(!config.scroll_rules.is_empty());
    }

    #[test]
    fn parse_batch_rule() {
        let json = r#"{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c", "v", "x"] }"#;
        let rule: KeyRule = serde_json::from_str(json).expect("batch rule should parse");
        match rule {
            KeyRule::Batch { keys, .. } => assert_eq!(keys.len(), 3),
            _ => panic!("expected Batch variant"),
        }
    }

    #[test]
    fn parse_single_rule() {
        let json = r#"{ "from_mods": ["ctrl"], "from_key": "y", "to_mods": ["cmd", "shift"], "to_key": "z" }"#;
        let rule: KeyRule = serde_json::from_str(json).expect("single rule should parse");
        match rule {
            KeyRule::Single { from_key, to_key, .. } => {
                assert_eq!(from_key, "y");
                assert_eq!(to_key, "z");
            }
            _ => panic!("expected Single variant"),
        }
    }

    #[test]
    fn roundtrip_batch_rule() {
        let rule = KeyRule::Batch {
            from_mods: vec!["ctrl".into()],
            to_mods: vec!["cmd".into()],
            keys: vec!["c".into(), "v".into()],
        };
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: KeyRule = serde_json::from_str(&json).unwrap();
        match parsed {
            KeyRule::Batch { keys, .. } => assert_eq!(keys, vec!["c", "v"]),
            _ => panic!("expected Batch variant after roundtrip"),
        }
    }
}
