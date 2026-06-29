use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone)]
pub struct AppConfig {
    pub paths: Vec<String>,
}

pub struct Config {
    pub apps: BTreeMap<String, AppConfig>,
    pub temp_dir: PathBuf,
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    apps: BTreeMap<String, RawApp>,
    #[serde(rename = "tempDir")]
    temp_dir: Option<String>,
}

#[derive(Deserialize)]
struct RawApp {
    #[serde(default)]
    paths: Vec<String>,
}

impl Config {
    pub fn load() -> Self {
        let mut config = Config::defaults();
        let Some(raw) = read_raw() else {
            return config;
        };
        for (id, app) in raw.apps {
            config.apps.insert(id, AppConfig { paths: app.paths });
        }
        if let Some(temp_dir) = raw.temp_dir {
            config.temp_dir = PathBuf::from(temp_dir);
        }
        config
    }

    pub(crate) fn defaults() -> Self {
        Config {
            apps: default_apps(),
            temp_dir: std::env::temp_dir().join("task-runner"),
        }
    }
}

fn read_raw() -> Option<RawConfig> {
    let content = std::fs::read_to_string(config_path()?).ok()?;
    serde_json::from_str(&content).ok()
}

fn config_path() -> Option<PathBuf> {
    config_dirs()
        .into_iter()
        .map(|dir| dir.join("config.json"))
        .find(|candidate| candidate.is_file())
}

fn config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(plugin_dir) = std::env::var_os("QOL_TRAY_PLUGIN_DIR") {
        dirs.push(PathBuf::from(plugin_dir));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
    {
        dirs.push(exe_dir);
    }
    dirs
}

fn default_apps() -> BTreeMap<String, AppConfig> {
    let entries: [(&str, &[&str]); 4] = [
        (
            "idea",
            &[
                "/opt/homebrew/bin/idea",
                "/usr/local/bin/idea",
                "/snap/bin/idea-ultimate",
                "/snap/bin/intellij-idea-ultimate",
                "~/.local/share/JetBrains/Toolbox/scripts/idea",
            ],
        ),
        (
            "vscode",
            &[
                "/usr/bin/code",
                "/opt/homebrew/bin/code",
                "/snap/bin/code",
                "/usr/local/bin/code",
            ],
        ),
        (
            "cursor",
            &[
                "/opt/homebrew/bin/cursor",
                "/usr/bin/cursor",
                "/usr/local/bin/cursor",
                "~/.local/bin/cursor",
            ],
        ),
        (
            "zed",
            &["/opt/homebrew/bin/zed", "/usr/bin/zed", "~/.local/bin/zed"],
        ),
    ];
    entries
        .into_iter()
        .map(|(id, paths)| {
            (
                id.to_string(),
                AppConfig {
                    paths: paths.iter().map(|path| path.to_string()).collect(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_the_built_in_ides() {
        let config = Config::defaults();
        for id in ["idea", "vscode", "cursor", "zed"] {
            assert!(config.apps.contains_key(id), "missing default app {id}");
        }
        assert!(config.temp_dir.ends_with("task-runner"));
    }
}
