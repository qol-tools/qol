use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .context("Could not determine config directory")
        .map(|p| p.join("qol-tray"))
}

pub fn plugins_dir() -> Result<PathBuf> {
    config_dir().map(|p| p.join("plugins"))
}

pub fn hotkeys_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join("hotkeys.json"))
}

pub fn plugin_configs_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join("plugin-configs.json"))
}

pub fn github_token_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join(".github-token"))
}

pub fn plugin_cache_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join(".plugin-cache.json"))
}

pub fn open_url(url: &str) -> Result<()> {
    use std::process::{Command, Stdio};

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    #[cfg(target_os = "windows")]
    Command::new("cmd")
        .args(["/C", "start", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_have_correct_suffixes() {
        let cases: Vec<(Result<PathBuf>, &str)> = vec![
            (config_dir(), "qol-tray"),
            (plugins_dir(), "qol-tray/plugins"),
            (hotkeys_path(), "hotkeys.json"),
            (plugin_configs_path(), "plugin-configs.json"),
            (github_token_path(), ".github-token"),
            (plugin_cache_path(), ".plugin-cache.json"),
        ];

        for (result, expected_suffix) in cases {
            let path = result.unwrap();
            assert!(path.ends_with(expected_suffix), "path {:?} should end with {}", path, expected_suffix);
        }
    }
}
