use std::path::{Path, PathBuf};

use qol_plugin_api::manifest::PluginManifest;

pub fn plugin_binary_exists(plugin_dir: &Path) -> bool {
    match plugin_binary_path(plugin_dir) {
        Some(binary) => binary.is_file(),
        None => true,
    }
}

pub fn plugin_binary_path(plugin_dir: &Path) -> Option<PathBuf> {
    let command = declared_binary_command(plugin_dir)?;
    if Path::new(&command).components().count() != 1 {
        return None;
    }
    let candidates = binary_candidates(plugin_dir, &command);
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

fn declared_binary_command(plugin_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(plugin_dir.join("plugin.toml")).ok()?;
    let manifest: PluginManifest = toml::from_str(&content).ok()?;
    manifest
        .daemon
        .as_ref()
        .filter(|daemon| daemon.enabled)
        .map(|daemon| daemon.command.clone())
        .or_else(|| {
            manifest
                .runtime
                .as_ref()
                .map(|runtime| runtime.command.clone())
        })
}

fn binary_candidates(plugin_dir: &Path, command: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(root) = qol_workspace::workspace_root_from(plugin_dir) {
        candidates.push(root.join("target").join("debug").join(command));
    }
    candidates.push(plugin_dir.join("target").join("debug").join(command));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_plugin_toml(dir: &Path, sections: &str) {
        fs::write(
            dir.join("plugin.toml"),
            format!(
                "[plugin]\nid = \"test-plugin\"\nname = \"Test\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n{sections}"
            ),
        )
        .unwrap();
    }

    fn write_cargo_plugin(dir: &Path) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"test-plugin\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    }

    #[test]
    fn binary_path_points_into_workspace_target_even_before_first_build() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"plugin-a\"]\n",
        )
        .unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(
            &plugin_dir,
            "\n[daemon]\nenabled = true\ncommand = \"plugin-a\"\n",
        );

        assert_eq!(
            plugin_binary_path(&plugin_dir),
            Some(tmp.path().join("target").join("debug").join("plugin-a"))
        );
    }

    #[test]
    fn no_declared_binary_has_no_path() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(&plugin_dir, "");

        assert_eq!(plugin_binary_path(&plugin_dir), None);
    }

    #[test]
    fn missing_declared_binary_is_not_fresh() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(
            &plugin_dir,
            "\n[daemon]\nenabled = true\ncommand = \"plugin-a\"\n",
        );

        assert!(!plugin_binary_exists(&plugin_dir));
    }

    #[test]
    fn declared_binary_present_in_workspace_target_is_fresh() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"plugin-a\"]\n",
        )
        .unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(
            &plugin_dir,
            "\n[daemon]\nenabled = true\ncommand = \"plugin-a\"\n",
        );
        let binary = tmp.path().join("target").join("debug").join("plugin-a");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "binary").unwrap();

        assert!(plugin_binary_exists(&plugin_dir));
    }

    #[test]
    fn declared_binary_present_in_standalone_plugin_target_is_fresh() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(
            &plugin_dir,
            "\n[daemon]\nenabled = true\ncommand = \"plugin-a\"\n",
        );
        let binary = plugin_dir.join("target").join("debug").join("plugin-a");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "binary").unwrap();

        assert!(plugin_binary_exists(&plugin_dir));
    }

    #[test]
    fn no_executable_declared_is_treated_fresh() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(&plugin_dir, "");

        assert!(plugin_binary_exists(&plugin_dir));
    }

    #[test]
    fn disabled_daemon_falls_back_to_runtime_command() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(
            &plugin_dir,
            "\n[daemon]\nenabled = false\ncommand = \"plugin-a\"\n\n[runtime]\ncommand = \"plugin-runner\"\n",
        );
        let binary = plugin_dir
            .join("target")
            .join("debug")
            .join("plugin-runner");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "binary").unwrap();

        assert!(plugin_binary_exists(&plugin_dir));
    }

    #[test]
    fn enabled_daemon_wins_over_runtime_command() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_cargo_plugin(&plugin_dir);
        write_plugin_toml(
            &plugin_dir,
            "\n[daemon]\nenabled = true\ncommand = \"plugin-daemon\"\n\n[runtime]\ncommand = \"plugin-runner\"\n",
        );
        let wrong_binary = plugin_dir
            .join("target")
            .join("debug")
            .join("plugin-runner");
        fs::create_dir_all(wrong_binary.parent().unwrap()).unwrap();
        fs::write(&wrong_binary, "binary").unwrap();

        assert!(
            !plugin_binary_exists(&plugin_dir),
            "only the enabled daemon command is the artifact the tray spawns"
        );
    }
}
