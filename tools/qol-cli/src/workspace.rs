use crate::host_facade;
use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value;

pub(crate) fn repo_root() -> Result<PathBuf> {
    workspace_root_from_cwd()
}

pub(crate) fn workspace_root_from_cwd() -> Result<PathBuf> {
    let mut current = env::current_dir().context("failed to read current directory")?;
    loop {
        if cargo_manifest_declares_workspace(&current)? {
            return Ok(current);
        }
        if !current.pop() {
            bail!("run this from inside a qol-tray cargo workspace");
        }
    }
}

fn cargo_manifest_declares_workspace(path: &Path) -> Result<bool> {
    let manifest = path.join("Cargo.toml");
    if !manifest.is_file() {
        return Ok(false);
    }
    let content = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let parsed: Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    Ok(parsed.get("workspace").is_some())
}

pub(crate) fn workspace_root(repo: &Path) -> Result<PathBuf> {
    for path in repo.ancestors() {
        if path.file_name().and_then(|name| name.to_str()) == Some("worktrees") {
            let parent = path
                .parent()
                .ok_or_else(|| anyhow!("worktrees directory has no parent"))?;
            return Ok(parent.to_path_buf());
        }
    }
    let parent = repo
        .parent()
        .ok_or_else(|| anyhow!("repo has no parent directory"))?;
    Ok(parent.to_path_buf())
}

pub(crate) fn sibling_crates(repo: &Path) -> Result<Vec<PathBuf>> {
    let workspace = workspace_root(repo)?;
    if !workspace.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&workspace)
        .with_context(|| format!("failed to read {}", workspace.display()))?
    {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => continue,
        };
        if name == "qol-tray" {
            continue;
        }
        if !name.starts_with("plugin-") && !name.starts_with("qol-") {
            continue;
        }
        if !path.join("Cargo.toml").is_file() {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

pub(crate) fn monorepo_plugins_dir(workspace_root: &Path) -> Option<PathBuf> {
    let candidate = workspace_root.join("plugins");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

pub(crate) fn monorepo_plugin_dirs(workspace_root: &Path) -> Result<Vec<PathBuf>> {
    let Some(plugins_dir) = monorepo_plugins_dir(workspace_root) else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(&plugins_dir)
        .with_context(|| format!("failed to read {}", plugins_dir.display()))?
    {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("plugin.toml").is_file() {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

pub(crate) fn discover_plugin_dirs(workspace_root: &Path) -> Result<Vec<PathBuf>> {
    let mono = monorepo_plugin_dirs(workspace_root)?;
    if !mono.is_empty() {
        return Ok(mono);
    }
    sibling_crates(workspace_root)
}

pub(crate) fn cargo_package_name(crate_dir: &Path) -> Result<String> {
    let manifest = crate_dir.join("Cargo.toml");
    let content = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let parsed: Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    parsed
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{} has no [package].name", manifest.display()))
}

pub(crate) struct BuildablePlugin {
    pub(crate) dir: PathBuf,
    pub(crate) package_name: String,
}

pub(crate) struct PluginScan {
    pub(crate) buildable: Vec<BuildablePlugin>,
    pub(crate) skipped_host: usize,
    pub(crate) skipped_no_runtime: usize,
}

pub(crate) fn scan_buildable_plugins(root: &Path) -> Result<PluginScan> {
    let mut scan = PluginScan {
        buildable: Vec::new(),
        skipped_host: 0,
        skipped_no_runtime: 0,
    };
    for dir in discover_plugin_dirs(root)? {
        match PluginEligibility::for_path(&dir)? {
            PluginEligibility::Buildable => {
                let package_name = cargo_package_name(&dir)
                    .with_context(|| format!("reading package name for {}", dir.display()))?;
                scan.buildable.push(BuildablePlugin { dir, package_name });
            }
            PluginEligibility::SkippedHost => scan.skipped_host += 1,
            PluginEligibility::SkippedNoRuntime => scan.skipped_no_runtime += 1,
        }
    }
    Ok(scan)
}

pub(crate) fn non_host_plugin_packages(root: &Path) -> Result<Vec<String>> {
    let mut excluded = Vec::new();
    for dir in discover_plugin_dirs(root)? {
        if matches!(
            PluginEligibility::for_path(&dir)?,
            PluginEligibility::SkippedHost
        ) {
            excluded.push(cargo_package_name(&dir)?);
        }
    }
    Ok(excluded)
}

enum PluginEligibility {
    Buildable,
    SkippedHost,
    SkippedNoRuntime,
}

impl PluginEligibility {
    fn for_path(path: &Path) -> Result<Self> {
        let manifest_path = path.join("plugin.toml");
        if !manifest_path.is_file() {
            return Ok(Self::SkippedNoRuntime);
        }
        let content = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest: Value = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        if !supports_host(&manifest) {
            return Ok(Self::SkippedHost);
        }
        if section_command(&manifest, "runtime").is_none()
            && section_command(&manifest, "daemon").is_none()
        {
            return Ok(Self::SkippedNoRuntime);
        }
        Ok(Self::Buildable)
    }
}

fn supports_host(manifest: &Value) -> bool {
    let entries = manifest
        .get("plugin")
        .and_then(|plugin| plugin.get("platforms"))
        .and_then(Value::as_array);
    let entries = match entries {
        Some(entries) => entries,
        None => return true,
    };
    if entries.is_empty() {
        return true;
    }
    entries
        .iter()
        .filter_map(Value::as_str)
        .any(|entry| entry == host_facade::os_name())
}

fn section_command<'a>(manifest: &'a Value, section: &str) -> Option<&'a str> {
    manifest
        .get(section)
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
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

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

pub(crate) fn exe_name(name: &str) -> String {
    host_facade::exe_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_workspace(dir: &Path) {
        fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/*\", \"plugins/*\"]\n",
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
    fn cargo_manifest_declares_workspace_recognizes_workspace_and_package_only() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("ws");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        let package_only = tmp.path().join("pkg");
        fs::create_dir_all(&package_only).unwrap();
        fs::write(package_only.join("Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();
        let hybrid = tmp.path().join("hybrid");
        fs::create_dir_all(&hybrid).unwrap();
        fs::write(
            hybrid.join("Cargo.toml"),
            "[package]\nname = \"a\"\n[workspace]\nmembers = [\"tools/*\"]\n",
        )
        .unwrap();
        let empty = tmp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();

        let cases = [
            (workspace.as_path(), true, "pure workspace"),
            (package_only.as_path(), false, "package only"),
            (hybrid.as_path(), true, "package + workspace (old layout)"),
            (empty.as_path(), false, "no Cargo.toml"),
        ];
        for (path, expected, label) in cases {
            assert_eq!(
                cargo_manifest_declares_workspace(path).unwrap(),
                expected,
                "case: {label}"
            );
        }
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

    fn write_manifest(dir: &Path, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("plugin.toml"), body).unwrap();
    }

    #[test]
    fn plugin_eligibility_classifies_manifests() {
        let tmp = tempfile::tempdir().unwrap();
        let buildable = tmp.path().join("buildable");
        write_manifest(
            &buildable,
            "[plugin]\nname = \"a\"\nversion = \"0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n[runtime]\ncommand = \"x\"\n",
        );
        let unsupported = tmp.path().join("unsupported");
        write_manifest(
            &unsupported,
            "[plugin]\nname = \"b\"\nversion = \"0\"\nplatforms = [\"plan9\"]\n[runtime]\ncommand = \"x\"\n",
        );
        let no_runtime = tmp.path().join("no_runtime");
        write_manifest(
            &no_runtime,
            "[plugin]\nname = \"c\"\nversion = \"0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n",
        );
        let missing = tmp.path().join("missing");
        fs::create_dir_all(&missing).unwrap();
        let daemon_only = tmp.path().join("daemon_only");
        write_manifest(
            &daemon_only,
            "[plugin]\nname = \"d\"\nversion = \"0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n[daemon]\nenabled = true\ncommand = \"x\"\n",
        );

        let cases: &[(&Path, &str)] = &[
            (&buildable, "Buildable"),
            (&unsupported, "SkippedHost"),
            (&no_runtime, "SkippedNoRuntime"),
            (&missing, "SkippedNoRuntime"),
            (&daemon_only, "Buildable"),
        ];
        for (path, want) in cases {
            let got = match PluginEligibility::for_path(path).unwrap() {
                PluginEligibility::Buildable => "Buildable",
                PluginEligibility::SkippedHost => "SkippedHost",
                PluginEligibility::SkippedNoRuntime => "SkippedNoRuntime",
            };
            assert_eq!(got, *want, "path: {}", path.display());
        }
    }
}
