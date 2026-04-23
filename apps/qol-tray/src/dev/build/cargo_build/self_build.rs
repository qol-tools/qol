mod artifacts;

use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};

use super::super::types::BuildResult;
use super::{spawn_piped, CargoChild};
use crate::paths;

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);

const QOL_TRAY_ID: &str = "qol-tray";

pub(super) fn build_qol_tray_self_with_progress<F>(
    repo_root: Option<&Path>,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let mut repo_root = repo_root
        .map(Path::to_path_buf)
        .unwrap_or_else(paths::repo_root_from_manifest_dir);

    if !manifest_is_qol_tray(&repo_root) {
        repo_root = resolve_missing_tray_root(&repo_root);
    }

    let manifest_path = repo_root.join("Cargo.toml");
    if let Err(error) = ensure_manifest(&manifest_path) {
        return failed_build(error);
    }
    let CargoChild {
        mut child,
        stdout,
        stderr,
    } = match start_build(&repo_root, &manifest_path, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = artifacts::spawn_readers(stdout, stderr);
    readers.emit_progress(&mut on_progress);
    let (actual_done, combined) = readers.join();
    finish_build(&mut child, actual_done, combined, &mut on_progress)
}

fn resolve_missing_tray_root(repo_root: &Path) -> std::path::PathBuf {
    let nested_tray = repo_root.join("qol-tray");
    if manifest_is_qol_tray(&nested_tray) {
        return nested_tray;
    }
    let sibling_tray = repo_root.parent().map(|p| p.join("qol-tray"));
    if let Some(sibling) = sibling_tray {
        if manifest_is_qol_tray(&sibling) {
            return sibling;
        }
    }

    let base = paths::repo_root_from_manifest_dir();
    log::info!(
        "[worktree] qol-tray not found in feature {}, falling back to base: {}",
        repo_root.display(),
        base.display()
    );
    base
}

fn manifest_is_qol_tray(dir: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
        return false;
    };
    let mut in_package = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let name = rest
                .trim_start()
                .trim_start_matches('=')
                .trim()
                .trim_matches(|c| c == '"' || c == '\'');
            return name == "qol-tray";
        }
    }
    false
}

fn start_build<F>(
    repo_root: &Path,
    manifest_path: &Path,
    on_progress: &mut F,
) -> Result<CargoChild, BuildResult>
where
    F: FnMut(u8, String),
{
    log::info!("Building qol-tray from {}", repo_root.display());
    on_progress(2, "Preparing build".to_string());
    spawn_build(repo_root, manifest_path).map_err(failed_build)
}

fn predicted_artifact_count() -> u32 {
    let previous = LAST_ARTIFACT_COUNT.load(Ordering::Relaxed);
    if previous == 0 {
        return 50;
    }
    previous
}

fn ensure_manifest(manifest_path: &Path) -> Result<(), String> {
    if manifest_path.is_file() {
        return Ok(());
    }
    Err(format!(
        "Cargo.toml not found at {}",
        manifest_path.display()
    ))
}

fn spawn_build(repo_root: &Path, manifest_path: &Path) -> Result<CargoChild, String> {
    let mut command = Command::new("cargo");
    command
        .args([
            "build",
            "--bin",
            "qol-tray",
            "--features",
            "dev",
            "--message-format",
            "json",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .current_dir(repo_root);
    spawn_piped(&mut command).map_err(|error| format!("Failed to run cargo build: {}", error))
}

fn finish_build<F>(
    child: &mut Child,
    actual_done: u32,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    super::finish_build(
        QOL_TRAY_ID,
        child,
        combined,
        on_progress,
        |output, progress| successful_build(actual_done, output, progress),
    )
}

fn successful_build<F>(actual_done: u32, combined: String, on_progress: &mut F) -> BuildResult
where
    F: FnMut(u8, String),
{
    LAST_ARTIFACT_COUNT.store(actual_done, Ordering::Relaxed);
    on_progress(100, "Build complete".to_string());
    log::info!("qol-tray build succeeded ({} artifacts)", actual_done);
    finished_build(combined)
}

fn failed_build(output: String) -> BuildResult {
    super::failed_build(QOL_TRAY_ID, output)
}

fn finished_build(output: String) -> BuildResult {
    super::finished_build(QOL_TRAY_ID, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_manifest(dir: &Path, contents: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), contents).unwrap();
    }

    fn qol_tray_manifest() -> &'static str {
        "[package]\nname = \"qol-tray\"\nversion = \"3.7.1\"\nedition = \"2021\"\n\n[features]\ndev = []\n"
    }

    fn plugin_manifest(name: &str) -> String {
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nserde = \"1.0\"\n")
    }

    #[test]
    fn manifest_is_qol_tray_accepts_qol_tray_package() {
        let tmp = TempDir::new().unwrap();
        write_manifest(tmp.path(), qol_tray_manifest());
        assert!(manifest_is_qol_tray(tmp.path()));
    }

    #[test]
    fn manifest_is_qol_tray_rejects_plugin_packages() {
        for plugin in [
            "alt-tab",
            "launcher",
            "keyremap",
            "pointz",
            "window-actions",
            "plugin-lights",
        ] {
            let tmp = TempDir::new().unwrap();
            write_manifest(tmp.path(), &plugin_manifest(plugin));
            assert!(
                !manifest_is_qol_tray(tmp.path()),
                "{plugin} manifest should not be detected as qol-tray",
            );
        }
    }

    #[test]
    fn manifest_is_qol_tray_rejects_missing_manifest() {
        let tmp = TempDir::new().unwrap();
        assert!(!manifest_is_qol_tray(tmp.path()));
    }

    #[test]
    fn manifest_is_qol_tray_ignores_name_in_non_package_section() {
        let tmp = TempDir::new().unwrap();
        let contents = "\
[package]
name = \"alt-tab\"
version = \"0.1.0\"

[[bin]]
name = \"qol-tray\"
path = \"src/main.rs\"
";
        write_manifest(tmp.path(), contents);
        assert!(!manifest_is_qol_tray(tmp.path()));
    }

    #[test]
    fn manifest_is_qol_tray_handles_whitespace_and_single_quotes() {
        for contents in [
            "[package]\n  name   =   \"qol-tray\"\nversion=\"3.7.1\"\n",
            "[package]\nname='qol-tray'\nversion='3.7.1'\n",
            "[package]\n\tname\t=\t\"qol-tray\"\n",
        ] {
            let tmp = TempDir::new().unwrap();
            write_manifest(tmp.path(), contents);
            assert!(
                manifest_is_qol_tray(tmp.path()),
                "should accept manifest: {contents:?}",
            );
        }
    }

    #[test]
    fn resolve_missing_tray_root_prefers_nested_qol_tray() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp.path().join("qol-tray"), qol_tray_manifest());
        assert_eq!(
            resolve_missing_tray_root(tmp.path()),
            tmp.path().join("qol-tray")
        );
    }

    #[test]
    fn resolve_missing_tray_root_falls_through_nested_plugin() {
        // Simulates a plugin-only worktree: <worktree>/plugin-alt-tab with its Cargo.toml.
        // Neither the worktree root nor a nested qol-tray/ contain qol-tray,
        // so we expect the sibling probe (which also misses here) or the base fallback.
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-alt-tab");
        write_manifest(&plugin_dir, &plugin_manifest("alt-tab"));
        let resolved = resolve_missing_tray_root(&plugin_dir);
        assert_ne!(resolved, plugin_dir, "should not return the plugin dir");
        assert!(
            manifest_is_qol_tray(&resolved),
            "fallback must point at real qol-tray manifest"
        );
    }

    #[test]
    fn resolve_missing_tray_root_finds_sibling_qol_tray() {
        // Simulates: <feature-root>/plugin-alt-tab and <feature-root>/qol-tray.
        let tmp = TempDir::new().unwrap();
        let feature_root = tmp.path();
        write_manifest(
            &feature_root.join("plugin-alt-tab"),
            &plugin_manifest("alt-tab"),
        );
        write_manifest(&feature_root.join("qol-tray"), qol_tray_manifest());
        let resolved = resolve_missing_tray_root(&feature_root.join("plugin-alt-tab"));
        assert_eq!(resolved, feature_root.join("qol-tray"));
    }
}
