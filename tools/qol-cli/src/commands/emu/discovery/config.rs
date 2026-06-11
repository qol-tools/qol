use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

use super::super::{arch::GuestArch, humanize_id, sanitize_id, Environment};

pub(crate) fn discover(path: Option<&Path>, home: Option<&PathBuf>) -> Result<Vec<Environment>> {
    Ok(load_image_overrides(path, home)?
        .into_iter()
        .map(|(id, (image_path, arch))| Environment {
            name: humanize_id(&id),
            id,
            backend: "qemu".to_string(),
            arch,
            image_path,
            source: "config".to_string(),
        })
        .collect())
}

fn load_image_overrides(
    path: Option<&Path>,
    home: Option<&PathBuf>,
) -> Result<HashMap<String, (PathBuf, GuestArch)>> {
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
) -> Result<HashMap<String, (PathBuf, GuestArch)>> {
    let parsed: TomlValue = toml::from_str(content).context("invalid emu config TOML")?;
    let Some(images) = parsed.get("images").and_then(TomlValue::as_table) else {
        return Ok(HashMap::new());
    };
    let mut overrides = HashMap::new();
    for (id, value) in images {
        let (path, arch) = match value {
            TomlValue::String(path) => (path.as_str(), GuestArch::X86_64),
            TomlValue::Table(table) => {
                let path = table
                    .get("path")
                    .and_then(TomlValue::as_str)
                    .ok_or_else(|| anyhow!("images.{id}.path must be a string path"))?;
                let arch = match table.get("arch") {
                    None => GuestArch::X86_64,
                    Some(value) => value.as_str().and_then(GuestArch::parse).ok_or_else(|| {
                        anyhow!("images.{id}.arch must be one of: x86_64, aarch64")
                    })?,
                };
                (path, arch)
            }
            _ => bail!("images.{id} must be a string path or a table with path/arch"),
        };
        overrides.insert(sanitize_id(id), (expand_home(path, home), arch));
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
        let (path, arch) = overrides.get("windows-11").unwrap();
        assert_eq!(path, &PathBuf::from("/home/me/vm/windows.qcow2"));
        assert_eq!(*arch, GuestArch::X86_64);
    }

    #[test]
    fn parses_table_form_with_arch() {
        let overrides = parse_image_overrides(
            r#"
[images.foo]
path = "/a/b/foo.qcow2"
arch = "aarch64"
"#,
            None,
        )
        .unwrap();
        let (path, arch) = overrides.get("foo").unwrap();
        assert_eq!(path, &PathBuf::from("/a/b/foo.qcow2"));
        assert_eq!(*arch, GuestArch::Aarch64);
    }

    #[test]
    fn rejects_unknown_arch() {
        let error = parse_image_overrides(
            r#"
[images.foo]
path = "/a/b/foo.qcow2"
arch = "sparc"
"#,
            None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("images.foo.arch"),
            "error: {error}"
        );
    }

    #[test]
    fn rejects_non_string_non_table_entries() {
        let error = parse_image_overrides(
            r#"
[images]
foo = 42
"#,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("images.foo"), "error: {error}");
    }
}
