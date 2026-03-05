use anyhow::{Context, Result};
use std::path::Path;

pub(super) fn ensure_release_build_manifest(manifest_path: &Path) -> Result<()> {
    ensure_cargo_manifest(manifest_path)?;
    strip_dev_dependencies_for_release_build(manifest_path)
}

fn ensure_cargo_manifest(manifest_path: &Path) -> Result<()> {
    if manifest_path.is_file() {
        return Ok(());
    }

    anyhow::bail!("Cargo.toml not found at {}", manifest_path.display())
}

fn strip_dev_dependencies_for_release_build(manifest_path: &Path) -> Result<()> {
    let mut value = manifest_value(manifest_path)?;
    let changed = strip_dev_dependencies(&mut value);
    write_sanitized_manifest(manifest_path, changed, value)
}

fn manifest_value(manifest_path: &Path) -> Result<toml::Value> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    toml::from_str(&content).with_context(|| format!("Failed to parse {}", manifest_path.display()))
}

fn strip_dev_dependencies(value: &mut toml::Value) -> bool {
    let Some(root) = value.as_table_mut() else {
        return false;
    };

    let mut changed = root.remove("dev-dependencies").is_some();
    strip_target_dev_dependencies(root, &mut changed);
    changed
}

fn strip_target_dev_dependencies(
    root: &mut toml::map::Map<String, toml::Value>,
    changed: &mut bool,
) {
    let Some(target) = root
        .get_mut("target")
        .and_then(|value| value.as_table_mut())
    else {
        return;
    };

    for target_entry in target.values_mut() {
        let Some(target_table) = target_entry.as_table_mut() else {
            continue;
        };
        if target_table.remove("dev-dependencies").is_some() {
            *changed = true;
        }
    }
}

fn write_sanitized_manifest(manifest_path: &Path, changed: bool, value: toml::Value) -> Result<()> {
    if !changed {
        return Ok(());
    }

    let rendered = toml::to_string(&value)
        .with_context(|| format!("Failed to render {}", manifest_path.display()))?;
    std::fs::write(manifest_path, rendered)
        .with_context(|| format!("Failed to write {}", manifest_path.display()))?;
    log::info!(
        "Stripped dev-dependencies from {} for install build",
        manifest_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn strip_dev_dependencies_for_release_build_removes_top_level_and_target_sections() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("Cargo.toml");

        std::fs::write(
            &manifest_path,
            r#"
[package]
name = "sample"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"

[dev-dependencies]
qol-tray = { path = "../../qol-tray" }
toml = "0.9"

[target.'cfg(unix)'.dev-dependencies]
tempfile = "3"
"#,
        )
        .unwrap();

        strip_dev_dependencies_for_release_build(&manifest_path).unwrap();

        let after = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(!after.contains("dev-dependencies"));
        assert!(after.contains("serde"));
    }
}
