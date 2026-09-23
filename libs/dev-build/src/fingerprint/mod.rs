use std::path::Path;
use std::{collections::HashMap, path::PathBuf};

mod hash;
mod inputs;
mod path_deps;

pub fn fingerprint_plugin(path: &Path) -> Result<String, String> {
    let mut cache = FingerprintCache::default();
    fingerprint_plugin_with_cache(path, &mut cache)
}

#[derive(Default)]
pub(crate) struct FingerprintCache {
    dependency_contents: HashMap<PathBuf, Vec<hash::FingerprintContent>>,
    #[cfg(test)]
    dependency_reads: usize,
}

pub(crate) fn fingerprint_plugin_with_cache(
    path: &Path,
    cache: &mut FingerprintCache,
) -> Result<String, String> {
    let mut inputs = hash::read_inputs(inputs::fingerprint_inputs(path)?)?;
    if inputs.is_empty() {
        return Err("No Rust build inputs found".to_string());
    }
    for dep_contents in path_deps::collect_path_dep_contents(path, cache) {
        inputs.extend(dep_contents);
    }
    if let Some((workspace_root, _)) = path_deps::workspace_manifest(path) {
        let workspace_input = vec![(
            PathBuf::from("__workspace__/Cargo.toml"),
            workspace_root.join("Cargo.toml"),
        )];
        if let Ok(contents) = hash::read_inputs(workspace_input) {
            inputs.extend(contents);
        }
    }
    hash::hash_contents(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn dependency_contents_are_reused_across_fingerprints() {
        let tmp = TempDir::new().unwrap();
        let dependency = tmp.path().join("dependency");
        fs::create_dir_all(dependency.join("src")).unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"plugin-a\", \"plugin-b\", \"dependency\"]\n\n[workspace.dependencies]\ndependency = { path = \"dependency\" }\n",
        )
        .unwrap();
        fs::write(
            dependency.join("Cargo.toml"),
            "[package]\nname = \"dependency\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            dependency.join("src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .unwrap();
        let plugin_a = tmp.path().join("plugin-a");
        let plugin_b = tmp.path().join("plugin-b");
        write_plugin(&plugin_a, "plugin-a", "fn main() { println!(\"a\"); }\n");
        write_plugin(&plugin_b, "plugin-b", "fn main() { println!(\"b\"); }\n");

        let mut cache = FingerprintCache::default();
        let first = fingerprint_plugin_with_cache(&plugin_a, &mut cache).unwrap();
        let second = fingerprint_plugin_with_cache(&plugin_b, &mut cache).unwrap();

        assert_ne!(first, second);
        assert_eq!(cache.dependency_contents.len(), 1);
        assert_eq!(cache.dependency_reads, 1);
    }

    #[test]
    fn workspace_root_manifest_is_fingerprinted() {
        let tmp = TempDir::new().unwrap();
        let plugin = tmp.path().join("plugin-a");
        write_workspace_root(tmp.path(), 1);
        write_plugin(&plugin, "plugin-a", "fn main() { println!(\"a\"); }\n");

        let first = fingerprint_plugin(&plugin).unwrap();
        assert_eq!(first, fingerprint_plugin(&plugin).unwrap());

        write_workspace_root(tmp.path(), 2);
        assert_ne!(first, fingerprint_plugin(&plugin).unwrap());
    }

    #[test]
    fn plugin_outside_a_workspace_still_fingerprints() {
        let tmp = TempDir::new().unwrap();
        let plugin = tmp.path().join("plugin-a");
        write_plugin(&plugin, "plugin-a", "fn main() { println!(\"a\"); }\n");

        assert!(fingerprint_plugin(&plugin).is_ok());
    }

    fn write_workspace_root(path: &Path, opt_level: u8) {
        fs::write(
            path.join("Cargo.toml"),
            format!(
                "[workspace]\nresolver = \"2\"\nmembers = [\"plugin-a\"]\n\n[profile.dev.package.png]\nopt-level = {opt_level}\n"
            ),
        )
        .unwrap();
    }

    fn write_plugin(path: &Path, name: &str, source: &str) {
        fs::create_dir_all(path.join("src")).unwrap();
        fs::write(
            path.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ndependency = {{ workspace = true }}\n"
            ),
        )
        .unwrap();
        fs::write(path.join("src/main.rs"), source).unwrap();
    }
}
