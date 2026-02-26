use serde::Deserialize;

const CONFIG_DIR: &str = "plugin-keyremap";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RemapConfig {
    pub excluded_apps: Vec<String>,
    pub key_rules: Vec<KeyRule>,
    pub mouse_rules: Vec<MouseRule>,
    pub scroll_rules: Vec<ScrollRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyRule {
    pub from_mods: Vec<String>,
    pub from_key: String,
    pub to_mods: Vec<String>,
    pub to_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MouseRule {
    pub from_mods: Vec<String>,
    pub button: String,
    pub to_mods: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScrollRule {
    pub from_mods: Vec<String>,
    pub to_mods: Vec<String>,
}

pub fn load_config() -> RemapConfig {
    if let Some(path) = config_path() {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => return config,
                Err(e) => eprintln!("[keyremap] config parse error: {e}"),
            },
            Err(e) => eprintln!("[keyremap] config read error: {e}"),
        }
    }
    RemapConfig::default()
}

fn config_path() -> Option<std::path::PathBuf> {
    let config_dir = dirs::config_dir()?.join("qol-tray").join("plugins").join(CONFIG_DIR);
    let path = config_dir.join(CONFIG_FILE);
    if path.exists() {
        return Some(path);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml = include_str!("../config/default.toml");
        let config: RemapConfig = toml::from_str(toml).expect("default config should parse");
        assert!(!config.excluded_apps.is_empty());
        assert!(!config.key_rules.is_empty());
        assert!(!config.mouse_rules.is_empty());
        assert!(!config.scroll_rules.is_empty());
    }

    #[test]
    fn parse_empty_config() {
        let config: RemapConfig = toml::from_str("").expect("empty config should parse");
        assert!(config.excluded_apps.is_empty());
        assert!(config.key_rules.is_empty());
        assert!(config.mouse_rules.is_empty());
        assert!(config.scroll_rules.is_empty());
    }

    #[test]
    fn parse_partial_config() {
        let toml = r#"
            excluded_apps = ["com.example.app"]
        "#;
        let config: RemapConfig = toml::from_str(toml).expect("partial config should parse");
        assert_eq!(config.excluded_apps.len(), 1);
        assert!(config.key_rules.is_empty());
    }
}
