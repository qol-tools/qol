use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RemapConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub excluded_apps: Vec<String>,
    pub char_swaps: Vec<(String, String)>,
    pub char_rules: Vec<CharRule>,
    pub key_rules: Vec<KeyRule>,
    pub mouse_rules: Vec<MouseRule>,
    pub scroll_rules: Vec<ScrollRule>,
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
pub(crate) fn builtin_defaults() -> RemapConfig {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CharRule {
    ByKey {
        #[serde(default)]
        from_mods: Vec<String>,
        from_key: String,
        to_char: String,
        #[serde(default)]
        global: bool,
    },
    ByChar {
        from_char: String,
        to_char: String,
        #[serde(default)]
        global: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyRule {
    Batch {
        #[serde(default)]
        from_mods: Vec<String>,
        #[serde(default)]
        to_mods: Vec<String>,
        keys: Vec<String>,
        #[serde(default)]
        global: bool,
    },
    Single {
        #[serde(default)]
        from_mods: Vec<String>,
        from_key: String,
        #[serde(default)]
        to_mods: Vec<String>,
        to_key: String,
        #[serde(default)]
        global: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseRule {
    #[serde(default)]
    pub from_mods: Vec<String>,
    pub button: String,
    #[serde(default)]
    pub to_mods: Vec<String>,
    #[serde(default)]
    pub global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollRule {
    #[serde(default)]
    pub from_mods: Vec<String>,
    #[serde(default)]
    pub to_mods: Vec<String>,
    #[serde(default)]
    pub global: bool,
}

pub fn load_config() -> RemapConfig {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_parse_as_config() {
        qol_config::validate_contract_defaults_match_type::<RemapConfig>(CONFIG_CONTRACT).unwrap();

        let config = builtin_defaults();
        assert!(config.enabled);
        assert!(!config.excluded_apps.is_empty());
        assert!(!config.key_rules.is_empty());
        assert!(!config.mouse_rules.is_empty());
        assert!(!config.scroll_rules.is_empty());
    }

    #[test]
    fn parse_empty_config() {
        let config: RemapConfig = serde_json::from_str("{}").expect("empty config should parse");
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
            KeyRule::Single {
                from_key, to_key, ..
            } => {
                assert_eq!(from_key, "y");
                assert_eq!(to_key, "z");
            }
            _ => panic!("expected Single variant"),
        }
    }

    #[test]
    fn parse_single_rule_without_modifiers() {
        let json = r#"{ "from_key": "a", "to_key": "b" }"#;
        let rule: KeyRule = serde_json::from_str(json).expect("bare key rule should parse");
        match rule {
            KeyRule::Single {
                from_mods,
                to_mods,
                from_key,
                to_key,
                ..
            } => {
                assert!(from_mods.is_empty());
                assert!(to_mods.is_empty());
                assert_eq!(from_key, "a");
                assert_eq!(to_key, "b");
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
            global: false,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: KeyRule = serde_json::from_str(&json).unwrap();
        match parsed {
            KeyRule::Batch { keys, .. } => assert_eq!(keys, vec!["c", "v"]),
            _ => panic!("expected Batch variant after roundtrip"),
        }
    }
}
