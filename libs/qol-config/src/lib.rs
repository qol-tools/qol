pub mod contract;
pub mod normalized;
pub mod validation;

use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

pub const NAMESPACE: &str = "qol-tray";

fn resolve_namespaced(base: Option<PathBuf>) -> Option<PathBuf> {
    base.map(|path| path.join(NAMESPACE))
}

pub fn data_dir() -> Option<PathBuf> {
    resolve_namespaced(dirs::data_local_dir().or_else(dirs::data_dir))
}

pub fn config_dir() -> Option<PathBuf> {
    resolve_namespaced(dirs::config_dir())
}

pub fn data_subdir(name: &str) -> Option<PathBuf> {
    data_dir().map(|path| path.join(name))
}

#[doc(hidden)]
pub fn base_data_dir() -> Option<PathBuf> {
    data_dir()
}

pub fn config_roots() -> Vec<PathBuf> {
    let Some(base) = base_data_dir() else {
        return Vec::new();
    };
    let active_install_id = install_id_from_active_file(&base);
    assemble_config_roots(
        base,
        install_id_from_env(),
        active_install_id,
        dirs::config_dir().map(|p| p.join("qol-tray")),
    )
}

fn assemble_config_roots(
    base: PathBuf,
    install_id: Option<String>,
    active_install_id: Option<String>,
    user_config_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(id) = install_id {
        roots.push(base.join("installs").join(id));
    }
    if let Some(id) = active_install_id {
        let candidate = base.join("installs").join(id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
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

/// Load a plugin's config using the identity the host injects at launch.
///
/// The host sets `QOL_TRAY_PLUGIN_ID` on every plugin process it spawns, so a
/// plugin never has to hardcode its own id. `fallback_id` is used only when the
/// plugin is run standalone, outside the host (no env var). A present-but-invalid
/// injected id is a host bug and aborts loudly rather than silently loading
/// defaults.
pub fn load_plugin_config_from_env<T: DeserializeOwned + Default>(fallback_id: &str) -> T {
    let id = plugin_id_from_env(fallback_id);
    load_plugin_config(&[id.as_str()])
}

/// Config file paths for the host-injected (or standalone-fallback) plugin id.
pub fn plugin_config_paths_from_env(fallback_id: &str) -> Vec<PathBuf> {
    let id = plugin_id_from_env(fallback_id);
    plugin_config_paths(&[id.as_str()])
}

/// The canonical plugin id the host injected at launch, or `fallback_id` when
/// run standalone. Aborts loudly on a present-but-invalid injected id.
pub fn plugin_id_from_env(fallback_id: &str) -> String {
    match std::env::var("QOL_TRAY_PLUGIN_ID") {
        Ok(value) => {
            let trimmed = value.trim();
            assert!(
                valid_install_id(trimmed),
                "QOL_TRAY_PLUGIN_ID {value:?} injected by the host is not a valid plugin id"
            );
            trimmed.to_string()
        }
        Err(_) => {
            assert!(
                valid_install_id(fallback_id),
                "standalone fallback plugin id {fallback_id:?} is not a valid plugin id"
            );
            fallback_id.to_string()
        }
    }
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

pub fn install_id_from_active_file(base_data_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join("active-install-id")).ok()?;
    let trimmed = content.trim();
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
    fn namespaced_resolver_joins_base_with_namespace() {
        let cases = [
            (
                Some(PathBuf::from("/data")),
                Some(PathBuf::from("/data/qol-tray")),
            ),
            (
                Some(PathBuf::from("/home/user/.local/share")),
                Some(PathBuf::from("/home/user/.local/share/qol-tray")),
            ),
            (None, None),
        ];
        for (base, expected) in cases {
            assert_eq!(resolve_namespaced(base.clone()), expected, "base: {base:?}");
        }
    }

    #[test]
    fn data_subdir_appends_under_namespaced_data_dir() {
        let Some(data) = data_dir() else {
            return;
        };
        assert_eq!(data_subdir("emu"), Some(data.join("emu")));
    }

    #[test]
    fn base_data_dir_is_an_alias_of_data_dir() {
        assert_eq!(base_data_dir(), data_dir());
        assert_eq!(NAMESPACE, "qol-tray");
    }

    #[test]
    fn without_pinned_install_base_is_the_only_data_root() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(base.clone(), None, None, None),
            vec![base]
        );
    }

    #[test]
    fn explicit_install_env_is_searched_before_base() {
        let base = PathBuf::from("/data/qol-tray");
        let user_cfg = PathBuf::from("/home/user/.config/qol-tray");
        let roots = assemble_config_roots(
            base.clone(),
            Some("install-123".into()),
            None,
            Some(user_cfg.clone()),
        );
        assert_eq!(
            roots,
            vec![base.join("installs").join("install-123"), base, user_cfg]
        );
    }

    #[test]
    fn active_install_file_id_is_searched_after_env_and_deduped() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(
                base.clone(),
                Some("env-id".into()),
                Some("active-id".into()),
                None,
            ),
            vec![
                base.join("installs").join("env-id"),
                base.join("installs").join("active-id"),
                base.clone(),
            ]
        );
        assert_eq!(
            assemble_config_roots(
                base.clone(),
                Some("same-id".into()),
                Some("same-id".into()),
                None,
            ),
            vec![base.join("installs").join("same-id"), base]
        );
    }

    #[test]
    fn active_install_file_id_searched_when_env_absent() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(base.clone(), None, Some("active-id".into()), None),
            vec![base.join("installs").join("active-id"), base]
        );
    }

    #[test]
    fn no_installs_dir_resolved_without_explicit_env() {
        let base = PathBuf::from("/data/qol-tray");
        let roots = assemble_config_roots(base.clone(), None, None, None);
        assert!(roots
            .iter()
            .all(|root| !root.to_string_lossy().contains("installs")));
        assert_eq!(roots.first(), Some(&base));
    }

    #[test]
    fn user_config_dir_equal_to_base_is_deduped() {
        let base = PathBuf::from("/data/qol-tray");
        assert_eq!(
            assemble_config_roots(base.clone(), None, None, Some(base.clone())),
            vec![base]
        );
    }
}
