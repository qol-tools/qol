use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

use super::super::{humanize_id, sanitize_id, Environment};

pub(crate) fn discover(path: Option<&Path>, home: Option<&PathBuf>) -> Result<Vec<Environment>> {
    Ok(load_image_overrides(path, home)?
        .into_iter()
        .map(|(id, image_path)| Environment {
            name: humanize_id(&id),
            id,
            backend: "qemu".to_string(),
            arch: "x86_64".to_string(),
            image_path,
            source: "config".to_string(),
        })
        .collect())
}

fn load_image_overrides(
    path: Option<&Path>,
    home: Option<&PathBuf>,
) -> Result<HashMap<String, PathBuf>> {
    let Some(path) = path else {
        return Ok(HashMap::new());
    };
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    parse_image_overrides(&content, home)
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn parse_image_overrides(
    content: &str,
    home: Option<&PathBuf>,
) -> Result<HashMap<String, PathBuf>> {
    let parsed: TomlValue = toml::from_str(content).context("invalid emu config TOML")?;
    let Some(images) = parsed.get("images").and_then(TomlValue::as_table) else {
        return Ok(HashMap::new());
    };
    let mut overrides = HashMap::new();
    for (id, value) in images {
        let path = value
            .as_str()
            .ok_or_else(|| anyhow!("images.{id} must be a string path"))?;
        overrides.insert(sanitize_id(id), expand_home(path, home));
    }
    Ok(overrides)
}

fn expand_home(path: &str, home: Option<&PathBuf>) -> PathBuf {
    let path = PathBuf::from(path);
    let Some(path_str) = path.to_str() else {
        return path;
    };
    if path_str == "~" {
        return home.cloned().unwrap_or(path);
    }
    if let Some(rest) = path_str.strip_prefix("~/") {
        if let Some(home) = home {
            return home.join(rest);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_image_overrides_with_sanitized_ids() {
        let home = PathBuf::from("/home/me");
        let overrides = parse_image_overrides(
            r#"
[images]
"Windows 11" = "~/vm/windows.qcow2"
"#,
            Some(&home),
        )
        .unwrap();
        assert_eq!(
            overrides.get("windows-11").unwrap(),
            &PathBuf::from("/home/me/vm/windows.qcow2")
        );
    }
}
