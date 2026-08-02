mod artifacts;
mod runtime;

pub use runtime::{prune_runtime_generations, stage_runtime_generation, StagedRuntimeGeneration};

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::cargo_build::{spawn_piped, CargoChild};
use crate::platform::{BuildPlatform, Platform};
use crate::BuildResult;

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);

const QOL_TRAY_ID: &str = qol_conventions::artifact::TRAY_PACKAGE_NAME;
const WORKTREE_SCAN_MAX_DEPTH: u8 = 5;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub branch: String,
    pub path: PathBuf,
}

pub fn list_worktrees(anchor: &Path) -> Vec<WorktreeInfo> {
    scan_with_branch_resolver(anchor, resolve_git_branch)
}

pub fn resolve_git_branch(repo_dir: &Path) -> Option<String> {
    if !repo_dir.join(".git").exists() {
        return None;
    }

    Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD")
}

pub fn resolve_tray_root(selected: Option<&Path>, fallback: &Path) -> PathBuf {
    let root = selected.unwrap_or(fallback);
    if manifest_is_qol_tray(root) {
        return root.to_path_buf();
    }
    resolve_missing_tray_root(root, fallback)
}

pub fn build_tray<F>(root: &Path, bins: &[&str], mut on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    if bins.is_empty() {
        return failed_build("no tray binaries requested".to_string());
    }
    let manifest_path = tray_manifest_path(root);
    if let Err(error) = ensure_manifest(&manifest_path) {
        return failed_build(error);
    }
    let identity =
        match qol_build_identity::BuildIdentityEnvironment::development(&artifact_root(root)) {
            Ok(identity) => identity,
            Err(error) => {
                return failed_build(format!("Failed to resolve build identity: {error}"));
            }
        };
    let CargoChild {
        mut child,
        stdout,
        stderr,
        process_tree,
    } = match start_build(root, &manifest_path, bins, &identity, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = artifacts::spawn_readers(stdout, stderr);
    let artifacts = readers.emit_progress(&mut on_progress);
    let (actual_done, combined) = readers.join();
    let artifact_stream =
        actual_done.and_then(|actual_done| artifacts.map(|artifacts| (actual_done, artifacts)));
    finish_build(
        root,
        bins,
        &identity,
        RunningCargo {
            child: &mut child,
            process_tree: &process_tree,
        },
        artifact_stream,
        combined,
        &mut on_progress,
    )
}

pub fn debug_binary_path(root: &Path, bin: &str) -> PathBuf {
    development_target_dir(root)
        .join("debug")
        .join(Platform.executable_name(bin))
}

fn development_target_dir(root: &Path) -> PathBuf {
    artifact_root(root)
        .join("target")
        .join("qol-dev")
        .join("build")
}

pub fn tray_manifest_path(root: &Path) -> PathBuf {
    resolve_tray_root(Some(root), root).join("Cargo.toml")
}

pub fn artifact_root(root: &Path) -> PathBuf {
    qol_workspace::workspace_root_from(root).unwrap_or_else(|_| root.to_path_buf())
}

pub fn marker_path(config_dir: &Path) -> PathBuf {
    config_dir.join("dev").join("active-worktree.txt")
}

pub fn read_active_worktree_marker(config_dir: &Path) -> Option<String> {
    std::fs::read_to_string(marker_path(config_dir))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn set_active_worktree_marker(config_dir: &Path, branch: Option<&str>) -> Result<(), String> {
    match branch {
        Some(branch) => write_active_worktree_marker(config_dir, branch),
        None => clear_active_worktree_marker(config_dir),
    }
}

pub fn write_active_worktree_marker(config_dir: &Path, branch: &str) -> Result<(), String> {
    let path = marker_path(config_dir);
    let parent = path
        .parent()
        .ok_or_else(|| format!("Invalid marker path: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create dev directory: {}", e))?;
    std::fs::write(&path, branch.trim()).map_err(|e| format!("Failed to write: {}", e))
}

pub fn clear_active_worktree_marker(config_dir: &Path) -> Result<(), String> {
    let path = marker_path(config_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Failed to clear marker: {}", error)),
    }
}

fn scan_with_branch_resolver<F>(anchor: &Path, resolve_branch: F) -> Vec<WorktreeInfo>
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    let Some(root) = find_dir_in_ancestors(anchor, "worktrees") else {
        return vec![];
    };
    collect_feature_grouped(&root.join("worktrees"), resolve_branch)
}

fn collect_feature_grouped<F>(worktrees_dir: &Path, resolve_branch: F) -> Vec<WorktreeInfo>
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    let mut out = Vec::new();
    for child in read_child_dirs(worktrees_dir) {
        walk_for_any_repo(&child, resolve_branch, &mut out, WORKTREE_SCAN_MAX_DEPTH);
    }
    out
}

fn walk_for_any_repo<F>(dir: &Path, resolve_branch: F, out: &mut Vec<WorktreeInfo>, max_depth: u8)
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    let mut frontier = vec![dir.to_path_buf()];
    for depth in 0..=max_depth {
        let mut next = Vec::new();
        for dir in frontier {
            if dir.join("Cargo.toml").is_file() && dir.join(".git").exists() {
                if let Some(branch) = resolve_branch(&dir) {
                    if !out.iter().any(|w| w.branch == branch) {
                        out.push(WorktreeInfo { branch, path: dir });
                    }
                }
            } else if depth < max_depth {
                next.extend(read_child_dirs(&dir));
            }
        }
        if next.is_empty() {
            return;
        }
        frontier = next;
    }
}

fn find_dir_in_ancestors(start: &Path, dir_name: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(dir_name).is_dir())
        .map(Path::to_path_buf)
}

fn read_child_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

fn resolve_missing_tray_root(root: &Path, fallback: &Path) -> PathBuf {
    for ancestor in root.ancestors() {
        let monorepo_tray = ancestor.join("apps").join("qol-tray");
        if manifest_is_qol_tray(&monorepo_tray) {
            return monorepo_tray;
        }
    }
    if let Ok(workspace_root) = qol_workspace::workspace_root_from(root) {
        let workspace_tray = workspace_root.join("apps").join("qol-tray");
        if manifest_is_qol_tray(&workspace_tray) {
            return workspace_tray;
        }
    }
    let nested_tray = root.join("qol-tray");
    if manifest_is_qol_tray(&nested_tray) {
        return nested_tray;
    }
    if let Some(sibling) = root.parent().map(|p| p.join("qol-tray")) {
        if manifest_is_qol_tray(&sibling) {
            return sibling;
        }
    }

    log::info!(
        "[worktree] qol-tray not found in feature {}, falling back to base: {}",
        root.display(),
        fallback.display()
    );
    fallback.to_path_buf()
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
            return name == qol_conventions::artifact::TRAY_PACKAGE_NAME;
        }
    }
    false
}

fn start_build<F>(
    root: &Path,
    manifest_path: &Path,
    bins: &[&str],
    identity: &qol_build_identity::BuildIdentityEnvironment,
    on_progress: &mut F,
) -> Result<CargoChild, BuildResult>
where
    F: FnMut(u8, String),
{
    log::info!("Building qol-tray from {}", root.display());
    on_progress(2, "Preparing build".to_string());
    spawn_build(root, manifest_path, bins, identity).map_err(failed_build)
}

fn spawn_build(
    root: &Path,
    manifest_path: &Path,
    bins: &[&str],
    identity: &qol_build_identity::BuildIdentityEnvironment,
) -> Result<CargoChild, String> {
    let mut command = tray_build_command(root, manifest_path, bins);
    identity.apply_to(&mut command);
    spawn_piped(command).map_err(|error| format!("Failed to run cargo build: {}", error))
}

fn tray_build_command(root: &Path, manifest_path: &Path, bins: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--target-dir")
        .arg(development_target_dir(root));
    for bin in bins {
        command.arg("--bin").arg(bin);
    }
    command
        .args([
            "--features",
            Platform.tray_dev_features(),
            "--message-format",
            "json",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .current_dir(root);
    command
}

struct RunningCargo<'a> {
    child: &'a mut Child,
    process_tree: &'a qol_process::OwnedProcessTree,
}

fn finish_build<F>(
    root: &Path,
    bins: &[&str],
    identity: &qol_build_identity::BuildIdentityEnvironment,
    running: RunningCargo<'_>,
    artifact_stream: Result<(u32, Vec<crate::cargo_build::CargoArtifact>), String>,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    crate::cargo_build::finish_build_owned(
        QOL_TRAY_ID,
        running.child,
        running.process_tree,
        combined,
        on_progress,
        |output, progress| match artifact_stream {
            Ok((actual_done, artifacts)) => successful_build(
                root,
                bins,
                identity,
                actual_done,
                artifacts,
                output,
                progress,
            ),
            Err(error) => failed_build(error),
        },
    )
}

fn successful_build<F>(
    root: &Path,
    bins: &[&str],
    identity: &qol_build_identity::BuildIdentityEnvironment,
    actual_done: u32,
    artifacts: Vec<crate::cargo_build::CargoArtifact>,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    if let Err(error) = identity.verify_unchanged(&artifact_root(root)) {
        return failed_build(format!("Build source identity is unstable: {error}"));
    }
    for bin in bins {
        let executable = match crate::cargo_build::select_binary_executable(
            &artifacts,
            &tray_manifest_path(root),
            bin,
        ) {
            Ok(executable) => executable,
            Err(error) => return failed_build(error.to_string()),
        };
        if !executable.is_file() {
            return failed_build(format!(
                "Cargo reported missing executable {}",
                executable.display()
            ));
        }
        let Some(role) = qol_conventions::artifact::tray_binary_role(bin) else {
            return failed_build(format!("Unknown qol-tray binary role for {bin}"));
        };
        let expectation = qol_artifact::ArtifactExpectation::development_debug(
            bin,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            role,
            true,
        )
        .with_exact_source(identity.source());
        if let Err(error) = qol_artifact::verify_path(&executable, &expectation) {
            return failed_build(format!(
                "Cargo reported unverified executable {}: {error}",
                executable.display()
            ));
        }
    }
    LAST_ARTIFACT_COUNT.store(actual_done, Ordering::Relaxed);
    on_progress(100, "Build complete".to_string());
    log::info!("qol-tray build succeeded ({} artifacts)", actual_done);
    let mut result = crate::cargo_build::finished_build(QOL_TRAY_ID, combined);
    result.artifacts = artifacts;
    result
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

fn failed_build(output: String) -> BuildResult {
    crate::cargo_build::failed_build(QOL_TRAY_ID, output)
}

fn predicted_artifact_count() -> u32 {
    let previous = LAST_ARTIFACT_COUNT.load(Ordering::Relaxed);
    if previous == 0 {
        return 50;
    }
    previous
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
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
            resolve_tray_root(Some(tmp.path()), Path::new("/fallback")),
            tmp.path().join("qol-tray")
        );
    }

    #[test]
    fn resolve_missing_tray_root_falls_through_nested_plugin() {
        let tmp = TempDir::new().unwrap();
        let fallback = tmp.path().join("base").join("qol-tray");
        write_manifest(&fallback, qol_tray_manifest());
        let plugin_dir = tmp.path().join("plugin-alt-tab");
        write_manifest(&plugin_dir, &plugin_manifest("alt-tab"));
        let resolved = resolve_tray_root(Some(&plugin_dir), &fallback);
        assert_ne!(resolved, plugin_dir, "should not return the plugin dir");
        assert_eq!(resolved, fallback);
    }

    #[test]
    fn resolve_missing_tray_root_finds_monorepo_app_member() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            &tmp.path().join("plugins").join("alt-tab"),
            &plugin_manifest("alt-tab"),
        );
        write_manifest(
            &tmp.path().join("apps").join("qol-tray"),
            qol_tray_manifest(),
        );

        let resolved = resolve_tray_root(
            Some(&tmp.path().join("plugins").join("alt-tab")),
            Path::new("/fallback"),
        );
        assert_eq!(resolved, tmp.path().join("apps").join("qol-tray"));
    }

    #[test]
    fn resolve_missing_tray_root_finds_sibling_qol_tray() {
        let tmp = TempDir::new().unwrap();
        let feature_root = tmp.path();
        write_manifest(
            &feature_root.join("plugin-alt-tab"),
            &plugin_manifest("alt-tab"),
        );
        write_manifest(&feature_root.join("qol-tray"), qol_tray_manifest());
        let resolved = resolve_tray_root(
            Some(&feature_root.join("plugin-alt-tab")),
            Path::new("/fallback"),
        );
        assert_eq!(resolved, feature_root.join("qol-tray"));
    }

    #[test]
    fn scan_returns_empty_when_no_worktrees_dir() {
        let tmp = TempDir::new().unwrap();
        let result = list_worktrees(tmp.path());
        assert!(result.is_empty(), "expected empty, got: {:?}", result);
    }

    #[test]
    fn scan_finds_feature_grouped_worktree() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        let tray_worktree = create_git_worktree(&feature.join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].branch, "feat/config-contract-v1");
        assert_eq!(result[0].path, tray_worktree);
    }

    #[test]
    fn scan_surfaces_plugin_only_feature() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        create_git_worktree(&feature.join("plugin-window-actions"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].branch, "feat/config-contract-v1");
    }

    #[test]
    fn scan_dedupes_when_feature_contains_many_repos() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        for repo in ["plugin-launcher", "plugin-alt-tab", "qol-tray"] {
            create_git_worktree(&feature.join(repo));
        }
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(
            result.len(),
            1,
            "dedupe by branch must collapse multi-repo features: {:?}",
            result
        );
        assert_eq!(result[0].branch, "feat/config-contract-v1");
    }

    #[test]
    fn scan_finds_slash_layout_two_levels_deep() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let nested = tmp
            .path()
            .join("worktrees")
            .join("feat")
            .join("shortcuts-watcher");
        let tray_worktree = create_git_worktree(&nested.join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].path, tray_worktree);
    }

    #[test]
    fn scan_finds_slash_layout_three_levels_deep() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let nested = tmp
            .path()
            .join("worktrees")
            .join("team")
            .join("a")
            .join("b");
        let tray_worktree = create_git_worktree(&nested.join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].path, tray_worktree);
    }

    #[test]
    fn scan_stops_at_first_repo_anchor() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feat = tmp.path().join("worktrees").join("feat");
        let outer = create_git_worktree(&feat.join("qol-tray"));
        create_git_worktree(&feat.join("a").join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].path, outer);
    }

    #[test]
    fn scan_skips_repo_dir_without_git() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        create_worktree(&feature.join("qol-tray"), false);
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert!(result.is_empty(), "result: {:?}", result);
    }

    fn create_manifest_dir(root: &Path, repo_name: &str) -> PathBuf {
        let manifest_dir = root.join(repo_name);
        std::fs::create_dir_all(&manifest_dir).unwrap();
        std::fs::create_dir_all(root.join("worktrees")).unwrap();
        manifest_dir
    }

    fn create_git_worktree(path: &Path) -> PathBuf {
        create_worktree(path, true)
    }

    fn create_worktree(path: &Path, with_git_dir: bool) -> PathBuf {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("Cargo.toml"), "[package]").unwrap();
        if with_git_dir {
            std::fs::create_dir_all(path.join(".git")).unwrap();
        }
        path.to_path_buf()
    }

    fn fake_branch_resolver(repo_dir: &Path) -> Option<String> {
        repo_dir
            .join(".git")
            .exists()
            .then(|| "feat/config-contract-v1".to_string())
    }

    #[test]
    fn build_command_uses_isolated_target_requested_bins_dev_features_json_and_manifest_path() {
        let root = Path::new("/repo/qol");
        let manifest = root.join("Cargo.toml");
        let target = root.join("target/qol-dev/build");
        let features = Platform.tray_dev_features();
        let cases: [(&[&str], Vec<&str>); 2] = [
            (
                &["qol-tray"],
                vec![
                    "build",
                    "--target-dir",
                    target.to_str().unwrap(),
                    "--bin",
                    "qol-tray",
                    "--features",
                    features,
                    "--message-format",
                    "json",
                    "--manifest-path",
                ],
            ),
            (
                &["qol-tray", "qol-tray-doctor"],
                vec![
                    "build",
                    "--target-dir",
                    target.to_str().unwrap(),
                    "--bin",
                    "qol-tray",
                    "--bin",
                    "qol-tray-doctor",
                    "--features",
                    features,
                    "--message-format",
                    "json",
                    "--manifest-path",
                ],
            ),
        ];

        for (bins, expected_prefix) in cases {
            let command = tray_build_command(root, &manifest, bins);
            let mut expected: Vec<_> = expected_prefix.into_iter().map(OsStr::new).collect();
            expected.push(manifest.as_os_str());
            assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
            assert_eq!(command.get_current_dir(), Some(root));
            assert_eq!(command.get_program(), OsStr::new("cargo"));
        }
    }

    #[test]
    fn marker_io_round_trips_and_clears() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(read_active_worktree_marker(tmp.path()), None);

        write_active_worktree_marker(tmp.path(), " feat/x \n").unwrap();

        assert_eq!(
            read_active_worktree_marker(tmp.path()).as_deref(),
            Some("feat/x")
        );
        assert!(marker_path(tmp.path()).ends_with("dev/active-worktree.txt"));

        clear_active_worktree_marker(tmp.path()).unwrap();

        assert_eq!(read_active_worktree_marker(tmp.path()), None);
    }

    #[test]
    fn set_active_worktree_marker_dispatches_write_and_clear() {
        let tmp = TempDir::new().unwrap();
        set_active_worktree_marker(tmp.path(), Some("feat/y")).unwrap();
        assert_eq!(
            read_active_worktree_marker(tmp.path()).as_deref(),
            Some("feat/y")
        );
        set_active_worktree_marker(tmp.path(), None).unwrap();
        assert_eq!(read_active_worktree_marker(tmp.path()), None);
    }

    #[test]
    fn debug_binary_path_is_isolated_from_ordinary_workspace_builds() {
        let root = Path::new("/repo/qol");
        assert_eq!(
            debug_binary_path(root, "qol-tray"),
            root.join("target")
                .join("qol-dev")
                .join("build")
                .join("debug")
                .join(Platform.executable_name("qol-tray"))
        );
    }

    #[test]
    fn debug_binary_path_uses_workspace_development_target_for_member_roots() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("mono");
        let tray_root = workspace.join("apps").join("qol-tray");
        write_manifest(
            &workspace,
            "[workspace]\nmembers = [\"apps/qol-tray\"]\nresolver = \"2\"\n",
        );
        write_manifest(&tray_root, qol_tray_manifest());

        assert_eq!(artifact_root(&tray_root), workspace);
        assert_eq!(
            debug_binary_path(&tray_root, "qol-tray"),
            workspace
                .join("target")
                .join("qol-dev")
                .join("build")
                .join("debug")
                .join(Platform.executable_name("qol-tray"))
        );
    }

    #[test]
    fn tray_manifest_path_resolves_the_package_from_a_monorepo_root() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("mono");
        let tray_root = workspace.join("apps").join("qol-tray");
        write_manifest(
            &workspace,
            "[workspace]\nmembers = [\"apps/qol-tray\"]\nresolver = \"2\"\n",
        );
        write_manifest(&tray_root, qol_tray_manifest());

        assert_eq!(tray_manifest_path(&workspace), tray_root.join("Cargo.toml"));
        assert_eq!(tray_manifest_path(&tray_root), tray_root.join("Cargo.toml"));
    }
}
