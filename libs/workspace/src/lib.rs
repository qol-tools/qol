use anyhow::{anyhow, bail, Context, Result};
use qol_plugin_api::manifest::PluginManifest;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const PLUGIN_DELIVERY_EXCLUDED_DIRECTORIES: [&str; 8] = [
    ".git",
    ".github",
    "benches",
    "examples",
    "node_modules",
    "reports",
    "src",
    "target",
];

pub fn workspace_root_from_cwd() -> Result<PathBuf> {
    let cwd = env::current_dir().context("failed to read current directory")?;
    workspace_root_from(&cwd)
}

pub fn workspace_root_from(start: &Path) -> Result<PathBuf> {
    for path in start.ancestors() {
        if cargo_manifest_declares_workspace(path)? {
            return Ok(path.to_path_buf());
        }
    }
    bail!("run this from inside a qol-tools cargo workspace")
}

fn cargo_manifest_declares_workspace(path: &Path) -> Result<bool> {
    let manifest = path.join("Cargo.toml");
    if !manifest.is_file() {
        return Ok(false);
    }
    let content = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let parsed: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    Ok(parsed.get("workspace").is_some())
}

pub fn plugins_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("plugins")
}

pub fn monorepo_plugin_dirs(workspace_root: &Path) -> Result<Vec<PathBuf>> {
    let plugins_dir = plugins_dir(workspace_root);
    if !plugins_dir.is_dir() {
        return Ok(Vec::new());
    }
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

pub fn worktrees_dir(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .find(|dir| dir.join("worktrees").is_dir())
        .map(|dir| dir.join("worktrees"))
}

pub fn workspace_parent(repo: &Path) -> Result<PathBuf> {
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

pub fn sibling_crates(repo: &Path) -> Result<Vec<PathBuf>> {
    let workspace = workspace_parent(repo)?;
    if !workspace.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&workspace)
        .with_context(|| format!("failed to read {}", workspace.display()))?
    {
        let path = entry?.path();
        if sibling_crate(&path) {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn sibling_crate(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name == "qol-tray" {
        return false;
    }
    if !path.join("plugin.toml").is_file() && !name.starts_with("qol-") {
        return false;
    }
    path.join("Cargo.toml").is_file()
}

pub fn discover_plugin_dirs(workspace_root: &Path) -> Result<Vec<PathBuf>> {
    let mono = monorepo_plugin_dirs(workspace_root)?;
    if !mono.is_empty() {
        return Ok(mono);
    }
    sibling_crates(workspace_root)
}

pub fn find_plugin_dirs(search_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut plugins = Vec::new();
    for search_path in search_paths {
        collect_search_path_plugins(search_path, &mut plugins);
    }
    plugins
}

fn collect_search_path_plugins(search_path: &Path, plugins: &mut Vec<PathBuf>) {
    if !search_path.exists() {
        return;
    }
    let mut entries = WalkDir::new(search_path)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_visit_entry);
    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !is_plugin_dir(path) {
            continue;
        }
        plugins.push(path.to_path_buf());
        entries.skip_current_dir();
    }
}

fn should_visit_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !name.starts_with('.')
        && name != "node_modules"
        && name != "target"
        && name != "vendor"
        && name != "worktrees"
}

pub fn is_plugin_dir(path: &Path) -> bool {
    path.is_dir() && path.join("plugin.toml").is_file()
}

pub fn cargo_package_name(crate_dir: &Path) -> Result<String> {
    let manifest = crate_dir.join("Cargo.toml");
    let content = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let parsed: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    parsed
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{} has no [package].name", manifest.display()))
}

pub fn cargo_bin_name(crate_dir: &Path) -> Result<String> {
    let manifest = crate_dir.join("Cargo.toml");
    let content = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let parsed: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    parsed
        .get("bin")
        .and_then(toml::Value::as_array)
        .and_then(|bins| bins.first())
        .and_then(|bin| bin.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            parsed
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| {
            anyhow!(
                "{} has no [[bin]] name or [package] name",
                manifest.display()
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSource {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

pub fn read_plugin_source(path: &Path) -> Option<PluginSource> {
    if !is_plugin_dir(path) {
        return None;
    }
    let manifest = read_manifest_slice(path)?;
    let id = manifest.plugin.id.or_else(|| path_file_name(path))?;
    if qol_conventions::is_reserved_plugin_id(&id) {
        return None;
    }
    Some(PluginSource {
        id,
        name: manifest.plugin.name,
        path: path.to_path_buf(),
    })
}

fn path_file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

#[derive(Deserialize)]
struct ManifestSlice {
    plugin: PluginInfoSlice,
}

#[derive(Deserialize)]
struct PluginInfoSlice {
    id: Option<String>,
    name: String,
}

fn read_manifest_slice(path: &Path) -> Option<ManifestSlice> {
    let content = fs::read_to_string(path.join("plugin.toml")).ok()?;
    toml::from_str(&content).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildablePlugin {
    pub dir: PathBuf,
    pub package_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginScan {
    pub buildable: Vec<BuildablePlugin>,
    pub skipped_host: usize,
    pub skipped_no_runtime: usize,
    pub skipped_reserved: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginEligibility {
    Buildable,
    SkippedHost,
    SkippedNoRuntime,
    SkippedReserved,
}

impl PluginEligibility {
    pub fn for_path(path: &Path) -> Result<Self> {
        if !path.join("plugin.toml").is_file() {
            return Ok(Self::SkippedNoRuntime);
        }
        let manifest = PluginManifest::read_from_dir(path)?;
        if manifest
            .plugin
            .id
            .as_ref()
            .is_some_and(|id| qol_conventions::is_reserved_plugin_id(id.as_str()))
        {
            return Ok(Self::SkippedReserved);
        }
        if !manifest.plugin.supports_current_platform() {
            return Ok(Self::SkippedHost);
        }
        if manifest.runtime.is_none() && !manifest.daemon.as_ref().is_some_and(|d| d.enabled) {
            return Ok(Self::SkippedNoRuntime);
        }
        Ok(Self::Buildable)
    }
}

pub fn scan_buildable_plugins(root: &Path) -> Result<PluginScan> {
    let mut scan = PluginScan::default();
    for dir in discover_plugin_dirs(root)? {
        match PluginEligibility::for_path(&dir)? {
            PluginEligibility::Buildable => {
                let package_name = cargo_package_name(&dir)
                    .with_context(|| format!("reading package name for {}", dir.display()))?;
                scan.buildable.push(BuildablePlugin { dir, package_name });
            }
            PluginEligibility::SkippedHost => scan.skipped_host += 1,
            PluginEligibility::SkippedNoRuntime => scan.skipped_no_runtime += 1,
            PluginEligibility::SkippedReserved => scan.skipped_reserved += 1,
        }
    }
    Ok(scan)
}

pub fn non_host_plugin_packages(root: &Path) -> Result<Vec<String>> {
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

pub fn plugin_build_features(plugin_dir: &Path) -> Vec<String> {
    PluginManifest::read_from_dir(plugin_dir)
        .map(|manifest| manifest.build.features)
        .unwrap_or_default()
}

pub fn qualified_plugin_build_features(plugin_dir: &Path) -> Result<Vec<String>> {
    let features = plugin_build_features(plugin_dir);
    if features.is_empty() {
        return Ok(Vec::new());
    }
    let package = cargo_package_name(plugin_dir)?;
    Ok(features
        .into_iter()
        .map(|feature| format!("{package}/{feature}"))
        .collect())
}

pub fn workspace_dev_features(root: &Path) -> Result<Vec<String>> {
    let mut features = Vec::new();
    for plugin in scan_buildable_plugins(root)?.buildable {
        features.extend(qualified_plugin_build_features(&plugin.dir)?);
    }
    features.sort();
    features.dedup();
    Ok(features)
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDeliveryFile {
    pub source: PathBuf,
    pub relative_path: PathBuf,
}

pub fn plugin_delivery_files(
    plugin_root: &Path,
    excluded_root_files: &[&str],
) -> Result<Vec<PluginDeliveryFile>> {
    let mut files = Vec::new();
    let entries = WalkDir::new(plugin_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(plugin_delivery_entry);
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to walk {}", plugin_root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative_path = entry
            .path()
            .strip_prefix(plugin_root)
            .with_context(|| format!("plugin file escaped {}", plugin_root.display()))?
            .to_path_buf();
        if relative_path
            .parent()
            .is_none_or(|parent| parent.as_os_str().is_empty())
            && relative_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| excluded_root_files.contains(&name))
        {
            continue;
        }
        files.push(PluginDeliveryFile {
            source: entry.path().to_path_buf(),
            relative_path,
        });
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn plugin_delivery_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return true;
    }
    entry
        .file_name()
        .to_str()
        .is_none_or(|name| !PLUGIN_DELIVERY_EXCLUDED_DIRECTORIES.contains(&name))
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

    fn write_plugin_dir(dir: &Path, pkg_name: &str, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{pkg_name}\"\n"),
        )
        .unwrap();
        fs::write(dir.join("plugin.toml"), body).unwrap();
    }

    fn plugin_toml(id: &str, platforms: &str, runtime: &str) -> String {
        format!(
            "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"0.0.0\"\nplatforms = [{platforms}]\n\n[menu]\nlabel = \"x\"\nitems = []\n{runtime}"
        )
    }

    #[test]
    fn workspace_root_from_finds_nearest_cargo_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("mono");
        let crate_dir = workspace.join("apps").join("qol-tray");
        fs::create_dir_all(&crate_dir).unwrap();
        write_workspace(&workspace);
        write_package(&crate_dir, "qol-tray");
        assert_eq!(workspace_root_from(&crate_dir).unwrap(), workspace);
    }

    #[test]
    fn monorepo_plugin_dirs_discovers_under_plugins_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("mono");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        let plugins = workspace.join("plugins");
        write_plugin_dir(
            &plugins.join("a"),
            "a-pkg",
            &plugin_toml(
                "plugin-a",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"a\"\n",
            ),
        );
        write_plugin_dir(
            &plugins.join("b"),
            "b-pkg",
            &plugin_toml(
                "plugin-b",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"b\"\n",
            ),
        );
        fs::create_dir_all(plugins.join("not-a-plugin")).unwrap();

        let names: Vec<_> = monorepo_plugin_dirs(&workspace)
            .unwrap()
            .into_iter()
            .map(|p| display_name(&p))
            .collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn discover_plugin_dirs_prefers_mono_layout_over_siblings() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("mono");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        write_plugin_dir(
            &workspace.join("plugins").join("x"),
            "x-pkg",
            &plugin_toml(
                "plugin-x",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"x\"\n",
            ),
        );
        write_package(&workspace.join("plugin-sibling"), "y-pkg");

        let names: Vec<_> = discover_plugin_dirs(&workspace)
            .unwrap()
            .into_iter()
            .map(|p| display_name(&p))
            .collect();
        assert_eq!(names, vec!["x".to_string()]);
    }

    #[test]
    fn plugin_delivery_files_exclude_sources_build_outputs_and_stale_binaries() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugin");
        for directory in ["src", "target/release", "assets"] {
            fs::create_dir_all(plugin.join(directory)).unwrap();
        }
        for (path, content) in [
            ("plugin.toml", "manifest"),
            ("qol-config.toml", "contract"),
            ("assets/icon.png", "icon"),
            ("src/main.rs", "source"),
            ("target/release/plugin-test", "build"),
            ("plugin-test", "stale"),
        ] {
            fs::write(plugin.join(path), content).unwrap();
        }

        let files = plugin_delivery_files(&plugin, &["plugin-test"]).unwrap();
        let relative = files
            .into_iter()
            .map(|file| file.relative_path)
            .collect::<Vec<_>>();

        assert_eq!(
            relative,
            [
                PathBuf::from("assets/icon.png"),
                PathBuf::from("plugin.toml"),
                PathBuf::from("qol-config.toml"),
            ]
        );
    }

    #[test]
    fn find_plugin_dirs_skips_generated_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target").join("debug").join("plugin-a");
        write_plugin_dir(
            &target,
            "plugin-a",
            &plugin_toml(
                "plugin-a",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"a\"\n",
            ),
        );
        assert!(find_plugin_dirs(&[tmp.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn find_plugin_dirs_skips_worktrees_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp
            .path()
            .join("worktrees")
            .join("diff-viewer")
            .join("qol-monorepo")
            .join("plugins")
            .join("plugin-a");
        write_plugin_dir(
            &worktree,
            "plugin-a",
            &plugin_toml(
                "plugin-a",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"a\"\n",
            ),
        );
        assert!(find_plugin_dirs(&[tmp.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn read_plugin_source_prefers_manifest_id() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin = tmp.path().join("folder-name");
        write_plugin_dir(
            &plugin,
            "pkg",
            &plugin_toml(
                "plugin-real",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"x\"\n",
            ),
        );
        let source = read_plugin_source(&plugin).unwrap();
        assert_eq!(source.id, "plugin-real");
        assert_eq!(source.name, "plugin-real");
    }

    #[test]
    fn cargo_bin_name_prefers_first_bin_table_and_falls_back_to_package_name() {
        let tmp = tempfile::tempdir().unwrap();
        let with_bin = tmp.path().join("with-bin");
        fs::create_dir_all(&with_bin).unwrap();
        fs::write(
            with_bin.join("Cargo.toml"),
            "[package]\nname = \"plugin-cli-sessions\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"cli-sessions\"\npath = \"src/main.rs\"\n",
        )
        .unwrap();
        assert_eq!(cargo_bin_name(&with_bin).unwrap(), "cli-sessions");

        let without_bin = tmp.path().join("without-bin");
        fs::create_dir_all(&without_bin).unwrap();
        fs::write(
            without_bin.join("Cargo.toml"),
            "[package]\nname = \"plugin-removeapp\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(cargo_bin_name(&without_bin).unwrap(), "plugin-removeapp");
    }

    #[test]
    fn workspace_dev_features_unions_and_sorts_plugin_build_features() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("mono");
        fs::create_dir_all(&workspace).unwrap();
        write_workspace(&workspace);
        write_plugin_dir(
            &workspace.join("plugins").join("b"),
            "b-pkg",
            &plugin_toml(
                "plugin-b",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"b\"\n\n[build]\nfeatures = [\"two\", \"one\"]\n",
            ),
        );
        write_plugin_dir(
            &workspace.join("plugins").join("a"),
            "a-pkg",
            &plugin_toml(
                "plugin-a",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"a\"\n\n[build]\nfeatures = [\"one\"]\n",
            ),
        );

        let features = workspace_dev_features(&workspace).unwrap();
        assert_eq!(features, vec!["a-pkg/one", "b-pkg/one", "b-pkg/two"]);
    }

    #[test]
    fn plugin_eligibility_classifies_manifests() {
        let tmp = tempfile::tempdir().unwrap();
        let buildable = tmp.path().join("buildable");
        write_plugin_dir(
            &buildable,
            "buildable",
            &plugin_toml(
                "plugin-a",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"x\"\n",
            ),
        );
        let unsupported = tmp.path().join("unsupported");
        write_plugin_dir(
            &unsupported,
            "unsupported",
            &plugin_toml("plugin-b", "\"plan9\"", "[runtime]\ncommand = \"x\"\n"),
        );
        let empty_platforms = tmp.path().join("empty-platforms");
        write_plugin_dir(
            &empty_platforms,
            "empty-platforms",
            &plugin_toml("plugin-c", "", "[runtime]\ncommand = \"x\"\n"),
        );
        let no_runtime = tmp.path().join("no-runtime");
        write_plugin_dir(
            &no_runtime,
            "no-runtime",
            &plugin_toml("plugin-d", "\"linux\", \"macos\", \"windows\"", ""),
        );
        let daemon_only = tmp.path().join("daemon-only");
        write_plugin_dir(
            &daemon_only,
            "daemon-only",
            &plugin_toml(
                "plugin-e",
                "\"linux\", \"macos\", \"windows\"",
                "[daemon]\nenabled = true\ncommand = \"x\"\n",
            ),
        );
        let reserved = tmp.path().join("reserved");
        write_plugin_dir(
            &reserved,
            "reserved",
            &plugin_toml(
                "plugin-template",
                "\"linux\", \"macos\", \"windows\"",
                "[runtime]\ncommand = \"x\"\n",
            ),
        );

        let cases = [
            (&buildable, PluginEligibility::Buildable),
            (&unsupported, PluginEligibility::SkippedHost),
            (&empty_platforms, PluginEligibility::SkippedHost),
            (&no_runtime, PluginEligibility::SkippedNoRuntime),
            (&daemon_only, PluginEligibility::Buildable),
            (&reserved, PluginEligibility::SkippedReserved),
        ];
        for (path, expected) in cases {
            assert_eq!(
                PluginEligibility::for_path(path).unwrap(),
                expected,
                "path: {}",
                path.display()
            );
        }
    }
}
