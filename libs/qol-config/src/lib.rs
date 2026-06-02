pub mod contract;
pub mod normalized;
pub mod validation;

use serde::de::DeserializeOwned;
use std::fs;
use std::path::PathBuf;

pub fn base_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|path| path.join("qol-tray"))
}

pub fn config_roots() -> Vec<PathBuf> {
    let Some(base) = base_data_dir() else {
        return Vec::new();
    };
    assemble_config_roots(
        base,
        install_id_from_env(),
        dirs::config_dir().map(|p| p.join("qol-tray")),
    )
}

fn assemble_config_roots(
    base: PathBuf,
    install_id: Option<String>,
    user_config_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(id) = install_id {
        roots.push(base.join("installs").join(id));
    }
    if !roots.contains(&base) {
        roots.push(base);
    }
    if let Some(config_dir) = user_config_dir {
        if !roots.contains(&config_dir) {
            roots.push(config_dir);
        }
    }
    roots
}

pub fn plugin_config_paths(names: &[&str]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in config_roots() {
        for name in names {
            let candidate = root.join("plugins").join(name).join("config.json");
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    paths
}

pub fn load_plugin_config<T: DeserializeOwned + Default>(names: &[&str]) -> T {
    for path in plugin_config_paths(names) {
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(_) => continue,
        };
        match serde_json::from_str::<T>(&contents) {
            Ok(config) => {
                eprintln!("[config] loaded from {}", path.display());
                return config;
            }
            Err(e) => {
                eprintln!("[config] failed to parse {}: {}", path.display(), e);
            }
        }
    }
    T::default()
}

pub fn install_id_from_env() -> Option<String> {
    let value = std::env::var("QOL_TRAY_INSTALL_ID").ok()?;
    let trimmed = value.trim();
    if !valid_install_id(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_pinned_install_base_is_the_only_data_root() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(assemble_config_roots(base.clone(), None, None), vec![base]);
    }

    #[test]
    fn explicit_install_env_is_searched_before_base() {
        let base = PathBuf::from("/data/qol-tray");
        let user_cfg = PathBuf::from("/home/user/.config/qol-tray");
        let roots = assemble_config_roots(
            base.clone(),
            Some("install-123".into()),
            Some(user_cfg.clone()),
        );
        assert_eq!(
            roots,
            vec![base.join("installs").join("install-123"), base, user_cfg]
        );
    }

    #[test]
    fn no_installs_dir_resolved_without_explicit_env() {
        let base = PathBuf::from("/data/qol-tray");
        let roots = assemble_config_roots(base.clone(), None, None);
        assert!(roots
            .iter()
            .all(|root| !root.to_string_lossy().contains("installs")));
        assert_eq!(roots.first(), Some(&base));
    }

    #[test]
    fn user_config_dir_equal_to_base_is_deduped() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(base.clone(), None, Some(base.clone())),
            vec![base]
        );
    }
}
