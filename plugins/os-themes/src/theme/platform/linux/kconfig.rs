use std::io;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use qol_fs::atomic_write;

const READ_TOOLS: [&str; 2] = ["kreadconfig6", "kreadconfig5"];
const WRITE_TOOLS: [&str; 2] = ["kwriteconfig6", "kwriteconfig5"];

pub(super) fn get(file: &str, group: &str, key: &str) -> Result<Option<String>> {
    for tool in READ_TOOLS {
        match Command::new(tool)
            .args(["--file", file, "--group", group, "--key", key])
            .output()
        {
            Ok(output) if output.status.success() => {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Ok((!value.is_empty()).then_some(value));
            }
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to run {tool}"));
            }
        }
    }
    read_direct(file, group, key)
}

pub(super) fn set(file: &str, group: &str, key: &str, value: &str) -> Result<()> {
    let mut last_failure: Option<String> = None;
    for tool in WRITE_TOOLS {
        let status = match Command::new(tool)
            .args(["--file", file, "--group", group, "--key", key, value])
            .status()
        {
            Ok(status) => status,
            Err(error) => {
                last_failure = Some(error.to_string());
                continue;
            }
        };
        if status.success() {
            return Ok(());
        }
        last_failure = Some(format!("{tool} exited with {status}"));
    }
    write_direct(file, group, key, value).map_err(|error| match last_failure {
        Some(reason) => anyhow!("{error:#} (KDE write tools failed: {reason})"),
        None => error,
    })
}

pub(super) fn read_direct(file: &str, group: &str, key: &str) -> Result<Option<String>> {
    let path = config_path(file);
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(read_key(&content, group, key)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub(super) fn write_direct(file: &str, group: &str, key: &str, value: &str) -> Result<()> {
    let path = config_path(file);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let updated = write_key(&content, group, key, value);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    atomic_write(&path, updated.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub(super) fn read_key(content: &str, group: &str, key: &str) -> Option<String> {
    let mut in_group = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_group = trimmed[1..trimmed.len() - 1].trim() == group;
            continue;
        }
        if in_group {
            if let Some((candidate, value)) = trimmed.split_once('=') {
                if candidate.trim() == key {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

pub(super) fn write_key(content: &str, group: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut in_group = false;
    let mut group_seen = false;
    let mut key_written = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_group && !key_written {
                out.push_str(&format!("{key}={value}\n"));
                key_written = true;
            }
            in_group = trimmed[1..trimmed.len() - 1].trim() == group;
            if in_group {
                group_seen = true;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_group && !key_written {
            if let Some((candidate, _)) = trimmed.split_once('=') {
                if candidate.trim() == key {
                    out.push_str(&format!("{key}={value}\n"));
                    key_written = true;
                    continue;
                }
            }
            if trimmed.is_empty() {
                out.push_str(&format!("{key}={value}\n"));
                key_written = true;
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !key_written {
        if !group_seen {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&format!("[{group}]\n"));
        }
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

fn config_path(file: &str) -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join(file);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join(file);
    }
    PathBuf::from(file)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::ENV_LOCK;

    struct Sandbox {
        root: std::path::PathBuf,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("os-themes-kconfig-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("bin")).expect("create sandbox bin dir");
            Sandbox { root }
        }

        fn fake_tool(&self, name: &str, exit_code: &str) {
            let path = self.root.join("bin").join(name);
            std::fs::write(&path, format!("#!/bin/sh\nexit {exit_code}\n"))
                .expect("write fake tool");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make fake tool executable");
        }

        fn with_env(&self, f: impl FnOnce()) {
            let _guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous_path = std::env::var_os("PATH");
            let previous_config = std::env::var_os("XDG_CONFIG_HOME");
            std::env::set_var("PATH", self.root.join("bin"));
            std::env::set_var("XDG_CONFIG_HOME", self.root.join("config"));
            f();
            match previous_path {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
            match previous_config {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn read_key_finds_the_value_in_the_group() {
        let content =
            "[General]\nColorScheme=BreezeDark\nName=BreezeDark\n\n[Icons]\nTheme=breeze-dark\n";
        assert_eq!(
            read_key(content, "General", "ColorScheme"),
            Some("BreezeDark".to_string())
        );
        assert_eq!(
            read_key(content, "Icons", "Theme"),
            Some("breeze-dark".to_string())
        );
    }

    #[test]
    fn read_key_misses_wrong_groups_and_missing_keys() {
        let content = "[General]\nColorScheme=BreezeDark\n";
        assert_eq!(read_key(content, "Icons", "Theme"), None);
        assert_eq!(read_key(content, "General", "Theme"), None);
        assert_eq!(read_key(content, "Missing", "ColorScheme"), None);
        assert_eq!(read_key("", "General", "ColorScheme"), None);
    }

    #[test]
    fn write_key_replaces_the_existing_value_in_place() {
        let content = "[General]\nColorScheme=Breeze\n\n[Icons]\nTheme=breeze\n";
        let updated = write_key(content, "General", "ColorScheme", "BreezeDark");
        assert_eq!(
            updated,
            "[General]\nColorScheme=BreezeDark\n\n[Icons]\nTheme=breeze\n"
        );
    }

    #[test]
    fn write_key_appends_to_the_existing_group_before_the_next_section() {
        let content = "[General]\nColorScheme=BreezeDark\n\n[Icons]\nTheme=breeze\n";
        let updated = write_key(content, "Icons", "Theme", "breeze-dark");
        assert_eq!(
            updated,
            "[General]\nColorScheme=BreezeDark\n\n[Icons]\nTheme=breeze-dark\n"
        );
    }

    #[test]
    fn write_key_creates_the_group_when_missing() {
        let content = "[General]\nColorScheme=BreezeDark\n";
        let updated = write_key(content, "Icons", "Theme", "breeze-dark");
        assert_eq!(
            updated,
            "[General]\nColorScheme=BreezeDark\n\n[Icons]\nTheme=breeze-dark\n"
        );
        assert_eq!(
            write_key("", "General", "ColorScheme", "Breeze"),
            "[General]\nColorScheme=Breeze\n"
        );
    }

    #[test]
    fn write_key_preserves_unrelated_content() {
        let content = "# comment\n[General]\nColorScheme=Breeze\nFont=Noto Sans,10\n";
        let updated = write_key(content, "General", "ColorScheme", "BreezeDark");
        assert_eq!(
            updated,
            "# comment\n[General]\nColorScheme=BreezeDark\nFont=Noto Sans,10\n"
        );
    }

    #[test]
    fn set_prefers_a_succeeding_tool_over_direct_write() {
        let sandbox = Sandbox::new("succeeding-tool");
        sandbox.fake_tool("kwriteconfig6", "0");
        sandbox.with_env(|| {
            set("kdeglobals", "General", "ColorScheme", "BreezeDark").expect("set succeeds");
        });
        assert!(
            !sandbox.root.join("config").exists(),
            "a succeeding tool must short-circuit the direct write"
        );
    }

    #[test]
    fn set_falls_back_to_direct_write_when_all_tools_fail() {
        let sandbox = Sandbox::new("failing-tools");
        sandbox.fake_tool("kwriteconfig6", "1");
        sandbox.fake_tool("kwriteconfig5", "1");
        sandbox.with_env(|| {
            set("kdeglobals", "General", "ColorScheme", "BreezeDark").expect("set succeeds");
        });
        let content = std::fs::read_to_string(sandbox.root.join("config/kdeglobals"))
            .expect("direct write happened");
        assert!(content.contains("ColorScheme=BreezeDark"));
    }

    #[test]
    fn get_falls_back_to_direct_read_when_all_tools_fail() {
        let sandbox = Sandbox::new("failing-read-tools");
        sandbox.fake_tool("kreadconfig6", "1");
        sandbox.fake_tool("kreadconfig5", "1");
        std::fs::create_dir_all(sandbox.root.join("config")).expect("create config dir");
        std::fs::write(
            sandbox.root.join("config/kdeglobals"),
            "[General]\nColorScheme=BreezeDark\n",
        )
        .expect("seed config file");
        sandbox.with_env(|| {
            assert_eq!(
                get("kdeglobals", "General", "ColorScheme").expect("get succeeds"),
                Some("BreezeDark".to_string()),
                "a failing tool must not answer for the config file"
            );
        });
    }
}
