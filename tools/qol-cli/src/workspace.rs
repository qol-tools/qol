use crate::host_facade;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) use qol_workspace::{
    display_name, monorepo_plugin_dirs, non_host_plugin_packages, scan_buildable_plugins,
    sibling_crates, BuildablePlugin,
};

#[cfg(test)]
use qol_workspace::{cargo_package_name, discover_plugin_dirs};

pub(crate) const DOCTOR_BUILD_ARGS: [&str; 7] = [
    "build",
    "-p",
    "qol-tray",
    "--features",
    "dev",
    "--bin",
    "qol-tray-doctor",
];

const DEFAULT_WORKSPACE_FILE: &str = "dev/default-workspace.txt";

pub(crate) fn doctor_binary_path(root: &Path) -> PathBuf {
    root.join("target")
        .join("debug")
        .join(exe_name("qol-tray-doctor"))
}

pub(crate) fn cargo_build_command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(args);
    command
}

pub(crate) fn repo_root() -> Result<PathBuf> {
    qol_workspace::workspace_root_from_cwd()
}

pub(crate) fn dev_repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to read current directory")?;
    dev_repo_root_from(&cwd, qol_config::config_dir().as_deref())
}

pub(crate) fn record_default_workspace(root: &Path) -> Result<()> {
    let config_dir = qol_config::config_dir().context("no user config directory is available")?;
    record_default_workspace_in(root, &config_dir)
}

fn dev_repo_root_from(cwd: &Path, config_dir: Option<&Path>) -> Result<PathBuf> {
    if let Ok(root) = qol_workspace::workspace_root_from(cwd) {
        if is_qol_cli_workspace(&root) {
            return Ok(root);
        }
    }
    let config_dir = config_dir.context(
        "qol dev has no default workspace; run `qol setup` from the qol workspace first",
    )?;
    read_default_workspace(config_dir)
}

fn record_default_workspace_in(root: &Path, config_dir: &Path) -> Result<()> {
    let root = exact_qol_cli_workspace(root)?;
    let value = root
        .to_str()
        .context("qol workspace path is not valid UTF-8")?;
    let content = format!("{value}\n");
    let path = default_workspace_path(config_dir);
    if fs::read_to_string(&path).ok().as_deref() == Some(&content) {
        return Ok(());
    }
    qol_fs::atomic_write_durable(&path, content.as_bytes())
        .with_context(|| format!("failed to record default workspace at {}", path.display()))
}

fn read_default_workspace(config_dir: &Path) -> Result<PathBuf> {
    let path = default_workspace_path(config_dir);
    let content = fs::read_to_string(&path).with_context(|| {
        format!(
            "qol dev has no default workspace; run `qol setup` from the qol workspace first ({})",
            path.display()
        )
    })?;
    let configured = content.trim_end_matches(['\r', '\n']);
    if configured.is_empty() {
        bail!("qol dev default workspace is empty; run `qol setup` from the qol workspace again");
    }
    exact_qol_cli_workspace(Path::new(configured)).with_context(|| {
        format!(
            "qol dev default workspace `{configured}` is unavailable; run `qol setup` from the qol workspace again"
        )
    })
}

fn exact_qol_cli_workspace(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", path.display()))?;
    let root = qol_workspace::workspace_root_from(&canonical)?;
    if root != canonical || !is_qol_cli_workspace(&root) {
        bail!("{} is not a qol CLI workspace root", path.display());
    }
    Ok(root)
}

fn is_qol_cli_workspace(root: &Path) -> bool {
    root.join("tools/qol-cli/Cargo.toml").is_file()
}

fn default_workspace_path(config_dir: &Path) -> PathBuf {
    config_dir.join(DEFAULT_WORKSPACE_FILE)
}

pub(crate) fn resolve_crate_target(root: &Path, name: &str) -> Result<PathBuf> {
    let mut candidates = vec![(root.to_path_buf(), "qol-tray".to_string())];
    for sibling in sibling_crates(root)? {
        let dn = display_name(&sibling);
        candidates.push((sibling, dn));
    }
    for plugin in monorepo_plugin_dirs(root)? {
        let dn = display_name(&plugin);
        candidates.push((plugin, dn));
    }
    let matches: Vec<&(PathBuf, String)> = candidates
        .iter()
        .filter(|(_, dn)| crate_name_matches(dn, name))
        .collect();
    match matches.as_slice() {
        [] => bail!("no qol-tray or sibling crate matching `{name}`"),
        [only] => Ok(only.0.clone()),
        many => {
            let names: Vec<&str> = many.iter().map(|(_, dn)| dn.as_str()).collect();
            bail!("ambiguous `{name}` - matches: {}", names.join(", "));
        }
    }
}

fn crate_name_matches(display: &str, query: &str) -> bool {
    display == query || display == format!("plugin-{query}") || display == format!("qol-{query}")
}

pub(crate) fn exe_name(name: &str) -> String {
    host_facade::exe_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_workspace(dir: &Path) {
        fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/*\", \"plugins/*\"]\n",
        )
        .unwrap();
    }

    fn write_qol_cli_workspace(dir: &Path) {
        fs::create_dir_all(dir.join("tools/qol-cli")).unwrap();
        write_workspace(dir);
        fs::write(
            dir.join("tools/qol-cli/Cargo.toml"),
            "[package]\nname = \"qol\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
    }

    fn write_package(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n"),
        )
        .unwrap();
    }

    fn write_plugin_dir(dir: &Path, pkg_name: &str, platforms: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{pkg_name}\"\n"),
        )
        .unwrap();
        fs::write(
            dir.join("plugin.toml"),
            format!("[plugin]\nname = \"{pkg_name}\"\nversion = \"0.0.0\"\nplatforms = [{platforms}]\n\n[menu]\nlabel = \"x\"\nitems = []\n"),
        )
        .unwrap();
    }

    #[test]
    fn discovers_sibling_crates_from_main_clone() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        for name in ["plugin-a", "qol-lib", "other"] {
            let dir = tmp.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        }
        let names: Vec<_> = sibling_crates(&repo)
            .unwrap()
            .into_iter()
            .map(|path| display_name(&path))
            .collect();
        assert_eq!(names, vec!["plugin-a".to_string(), "qol-lib".to_string()]);
    }

    #[test]
    fn discovers_sibling_crates_from_feature_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("worktrees").join("feat").join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let plugin = tmp.path().join("plugin-a");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let names: Vec<_> = sibling_crates(&repo)
            .unwrap()
            .into_iter()
            .map(|path| display_name(&path))
            .collect();
        assert_eq!(names, vec!["plugin-a".to_string()]);
    }

    #[test]
    fn resolve_crate_target_handles_qol_tray_and_siblings() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let plugin = tmp.path().join("plugin-alt-tab");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let lib = tmp.path().join("qol-color");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("Cargo.toml"), "[package]\nname = \"y\"\n").unwrap();

        assert_eq!(resolve_crate_target(&repo, "qol-tray").unwrap(), repo);
        assert_eq!(
            resolve_crate_target(&repo, "plugin-alt-tab").unwrap(),
            plugin
        );
        assert_eq!(resolve_crate_target(&repo, "alt-tab").unwrap(), plugin);
        assert_eq!(resolve_crate_target(&repo, "qol-color").unwrap(), lib);
        assert_eq!(resolve_crate_target(&repo, "color").unwrap(), lib);
        assert_eq!(resolve_crate_target(&repo, "tray").unwrap(), repo);
        assert!(resolve_crate_target(&repo, "missing").is_err());
    }

    #[test]
    fn resolve_crate_target_reports_ambiguity() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("qol-tray");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        for name in ["plugin-foo", "qol-foo"] {
            let dir = tmp.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("Cargo.toml"), "[package]\nname = \"z\"\n").unwrap();
        }
        let err = resolve_crate_target(&repo, "foo").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "got: {err}");
        assert!(
            err.contains("plugin-foo") && err.contains("qol-foo"),
            "got: {err}"
        );
    }

    #[test]
    fn monorepo_plugin_dirs_discovers_under_plugins_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("mono");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        let plugins = workspace.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        write_plugin_dir(&plugins.join("plugin-a"), "a-pkg", "\"linux\"");
        write_plugin_dir(&plugins.join("plugin-b"), "b-pkg", "\"linux\", \"macos\"");
        fs::create_dir_all(plugins.join("not-a-plugin")).unwrap();

        let names: Vec<_> = monorepo_plugin_dirs(&workspace)
            .unwrap()
            .into_iter()
            .map(|p| display_name(&p))
            .collect();
        assert_eq!(names, vec!["plugin-a".to_string(), "plugin-b".to_string()]);
    }

    #[test]
    fn monorepo_plugin_dirs_returns_empty_when_plugins_subdir_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("plain");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        assert!(monorepo_plugin_dirs(&workspace).unwrap().is_empty());
    }

    #[test]
    fn discover_plugin_dirs_prefers_mono_layout_over_siblings() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("mono");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        let plugins = workspace.join("plugins");
        write_plugin_dir(&plugins.join("plugin-x"), "x-pkg", "\"linux\"");
        write_package(&workspace.join("plugin-sibling"), "y-pkg");

        let names: Vec<_> = discover_plugin_dirs(&workspace)
            .unwrap()
            .into_iter()
            .map(|p| display_name(&p))
            .collect();
        assert_eq!(names, vec!["plugin-x".to_string()]);
    }

    #[test]
    fn discover_plugin_dirs_falls_back_to_siblings_when_no_mono_plugins_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("qol-tray");
        write_package(&repo, "qol-tray");
        write_package(&tmp.path().join("plugin-old"), "old-pkg");
        let names: Vec<_> = discover_plugin_dirs(&repo)
            .unwrap()
            .into_iter()
            .map(|p| display_name(&p))
            .collect();
        assert_eq!(names, vec!["plugin-old".to_string()]);
    }

    #[test]
    fn cargo_package_name_reads_real_package_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cases = [
            ("plugin-alt-tab", "alt-tab"),
            ("plugin-ide-checkout", "task-runner"),
            ("plugin-pointz", "pointzerver"),
            ("plugin-keyremap", "keyremap"),
            ("plugin-lights", "plugin-lights"),
        ];
        for (dir_name, pkg_name) in cases {
            let dir = tmp.path().join(dir_name);
            write_package(&dir, pkg_name);
            assert_eq!(
                cargo_package_name(&dir).unwrap(),
                pkg_name,
                "dir: {dir_name}"
            );
        }
    }

    #[test]
    fn cargo_package_name_errors_on_missing_field() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("broken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let err = cargo_package_name(&dir).unwrap_err().to_string();
        assert!(err.contains("[package].name"), "got: {err}");
    }

    #[test]
    fn dev_workspace_prefers_the_current_qol_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("current");
        let configured = tmp.path().join("configured");
        let config_dir = tmp.path().join("config");
        write_qol_cli_workspace(&current);
        write_qol_cli_workspace(&configured);
        record_default_workspace_in(&configured, &config_dir).unwrap();

        let resolved = dev_repo_root_from(&current.join("tools"), Some(&config_dir)).unwrap();

        assert_eq!(resolved, current);
    }

    #[test]
    fn dev_workspace_uses_the_default_outside_qol() {
        let tmp = tempfile::tempdir().unwrap();
        let configured = tmp.path().join("configured");
        let foreign = tmp.path().join("foreign");
        let config_dir = tmp.path().join("config");
        write_qol_cli_workspace(&configured);
        fs::create_dir_all(foreign.join("nested")).unwrap();
        write_workspace(&foreign);
        record_default_workspace_in(&configured, &config_dir).unwrap();

        let resolved = dev_repo_root_from(&foreign.join("nested"), Some(&config_dir)).unwrap();

        assert_eq!(resolved, configured.canonicalize().unwrap());
    }

    #[test]
    fn dev_workspace_errors_explain_how_to_repair_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        let config_dir = tmp.path().join("config");
        fs::create_dir_all(&outside).unwrap();

        let missing = dev_repo_root_from(&outside, Some(&config_dir))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("run `qol setup`"), "got: {missing}");

        fs::create_dir_all(default_workspace_path(&config_dir).parent().unwrap()).unwrap();
        fs::write(default_workspace_path(&config_dir), "/gone/qol\n").unwrap();
        let stale = dev_repo_root_from(&outside, Some(&config_dir))
            .unwrap_err()
            .to_string();
        assert!(stale.contains("run `qol setup`"), "got: {stale}");
    }

    #[test]
    fn resolve_crate_target_includes_monorepo_plugins() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("mono");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        write_plugin_dir(
            &workspace.join("plugins").join("plugin-alt-tab"),
            "alt-tab",
            "\"linux\"",
        );

        let resolved = resolve_crate_target(&workspace, "plugin-alt-tab").unwrap();
        assert_eq!(display_name(&resolved), "plugin-alt-tab");
    }
}
