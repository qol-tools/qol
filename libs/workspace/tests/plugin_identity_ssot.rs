use qol_plugin_api::manifest::PluginManifest;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn bin_names(crate_manifest: &Path) -> Vec<String> {
    let content = fs::read_to_string(crate_manifest)
        .unwrap_or_else(|error| panic!("read {}: {error}", crate_manifest.display()));
    let parsed: toml::Value = toml::from_str(&content)
        .unwrap_or_else(|error| panic!("parse {}: {error}", crate_manifest.display()));
    if let Some(bins) = parsed.get("bin").and_then(toml::Value::as_array) {
        return bins
            .iter()
            .map(|bin| {
                bin.get("name")
                    .and_then(toml::Value::as_str)
                    .unwrap_or_else(|| {
                        panic!("{} has a [[bin]] without a name", crate_manifest.display())
                    })
                    .to_string()
            })
            .collect();
    }
    let package = parsed
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("{} has no [package].name", crate_manifest.display()));
    vec![package.to_string()]
}

fn binary_name_constant_files(plugin_dir: &Path) -> Vec<String> {
    let src = plugin_dir.join("src");
    if !src.is_dir() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for entry in WalkDir::new(&src).follow_links(false) {
        let entry = entry.expect("walk plugin src");
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        if content.contains("const BINARY_NAME") {
            found.push(entry.path().display().to_string());
        }
    }
    found
}

#[test]
fn every_plugin_declares_one_identity_everywhere() {
    let root = workspace_root();
    let plugin_dirs = qol_workspace::monorepo_plugin_dirs(&root).expect("read plugins directory");
    assert!(
        !plugin_dirs.is_empty(),
        "no plugins/*/plugin.toml found under {}",
        root.display()
    );

    let mut problems = Vec::new();
    let mut declared_ids: HashMap<String, PathBuf> = HashMap::new();

    for dir in plugin_dirs {
        let folder = dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let manifest_path = dir.join("plugin.toml");
        let crate_manifest = dir.join("Cargo.toml");
        let expected = format!("qol-{folder}");

        let manifest = match PluginManifest::read_from_dir(&dir) {
            Ok(manifest) => manifest,
            Err(error) => {
                problems.push(format!("{}: {error}", manifest_path.display()));
                continue;
            }
        };

        if let Some(id) = manifest.plugin.id.as_ref() {
            let id = id.as_str();
            if let Some(previous) = declared_ids.insert(id.to_string(), manifest_path.clone()) {
                problems.push(format!(
                    "{}: duplicate id {id}, also declared by {}",
                    manifest_path.display(),
                    previous.display()
                ));
            }
            if id != expected {
                problems.push(format!(
                    "{}: [plugin].id is {id}, expected {expected}",
                    manifest_path.display()
                ));
            }
        } else {
            problems.push(format!(
                "{}: [plugin].id is missing, expected {expected}",
                manifest_path.display()
            ));
        }

        match qol_workspace::cargo_package_name(&dir) {
            Ok(package) if package == expected => {}
            Ok(package) => problems.push(format!(
                "{}: Cargo package is {package}, expected {expected}",
                crate_manifest.display()
            )),
            Err(error) => problems.push(format!("{}: {error}", crate_manifest.display())),
        }

        for bin in bin_names(&crate_manifest) {
            if bin != expected {
                problems.push(format!(
                    "{}: [[bin]].name is {bin}, expected {expected}",
                    crate_manifest.display()
                ));
            }
        }

        if let Some(runtime) = manifest.runtime.as_ref() {
            if runtime.command != expected {
                problems.push(format!(
                    "{}: [runtime].command is {}, expected {expected}",
                    manifest_path.display(),
                    runtime.command
                ));
            }
        }

        if let Some(daemon) = manifest.daemon.as_ref().filter(|daemon| daemon.enabled) {
            if daemon.command != expected {
                problems.push(format!(
                    "{}: [daemon].command is {}, expected {expected}",
                    manifest_path.display(),
                    daemon.command
                ));
            }
        }

        match manifest.dependencies.as_ref() {
            Some(dependencies) if !dependencies.binaries.is_empty() => {
                for binary in &dependencies.binaries {
                    if binary.name != expected {
                        problems.push(format!(
                            "{}: [[dependencies.binaries]].name is {}, expected {expected}",
                            manifest_path.display(),
                            binary.name
                        ));
                    }
                    let expected_repo = format!("qol-tools/{expected}");
                    if binary.repo != expected_repo {
                        problems.push(format!(
                            "{}: [[dependencies.binaries]].repo is {}, expected {expected_repo}",
                            manifest_path.display(),
                            binary.repo
                        ));
                    }
                    if binary.pattern.is_empty() {
                        problems.push(format!(
                            "{}: [[dependencies.binaries]].pattern is empty, expected {expected}-{{os}}-{{arch}}",
                            manifest_path.display()
                        ));
                        continue;
                    }
                    let prefix = format!("{expected}-");
                    if !binary.pattern.starts_with(&prefix) {
                        problems.push(format!(
                            "{}: [[dependencies.binaries]].pattern is {}, expected to start with {prefix}",
                            manifest_path.display(),
                            binary.pattern
                        ));
                    }
                }
            }
            _ => problems.push(format!(
                "{}: [[dependencies.binaries]] is missing, expected an entry with name {expected} and pattern {expected}-{{os}}-{{arch}}",
                manifest_path.display()
            )),
        }

        for file in binary_name_constant_files(&dir) {
            problems.push(format!("{file}: const BINARY_NAME must not exist"));
        }
    }

    assert!(
        problems.is_empty(),
        "plugin identity violations:\n{}",
        problems.join("\n")
    );
}
