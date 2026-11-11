use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::Path;

pub(super) const FILE: &str = "manifest.json";
pub(super) const CONTRACT_FILES: [&str; 2] = ["qol-config.toml", "qol-runtime.toml"];
const SCHEMA: u32 = 1;

#[derive(Debug, Deserialize)]
pub(super) struct Manifest {
    schema: u32,
    pull_request: u64,
    head_sha: String,
    target: String,
    pub(super) plugins: Vec<PluginBuild>,
    pub(super) tray: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PluginBuild {
    pub(super) id: String,
    pub(super) version: String,
    pub(super) tag: String,
    pub(super) file: String,
    pub(super) bin_name: String,
    #[serde(default)]
    pub(super) contract_files: Option<Vec<String>>,
}

pub(super) fn read(path: &Path) -> Result<Manifest> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("the build has no {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("{} is not a build manifest", path.display()))
}

impl Manifest {
    pub(super) fn check(&self, number: u64, head: &str, target: &str) -> Result<()> {
        if self.schema != SCHEMA {
            bail!(
                "build manifest schema {} is not {SCHEMA}; update qol",
                self.schema
            );
        }
        if self.pull_request != number {
            bail!(
                "the build is for pull request #{}, not #{number}",
                self.pull_request
            );
        }
        if self.head_sha != head {
            bail!(
                "the build is of {}, but the pull request is at {head}; wait for its new build",
                self.head_sha
            );
        }
        if self.target != target {
            bail!("the build targets {}, not {target}", self.target);
        }
        for plugin in &self.plugins {
            for value in [&plugin.id, &plugin.tag, &plugin.file, &plugin.bin_name] {
                if !super::plain_name(value) {
                    bail!("the build manifest names `{value}`, which is not a plain file name");
                }
            }
            for name in plugin.contract_files.iter().flatten() {
                if !CONTRACT_FILES.contains(&name.as_str()) {
                    bail!("the build manifest names `{name}`, which is not a plugin settings file");
                }
            }
        }
        Ok(())
    }

    pub(super) fn replaces(&self, id: &str) -> bool {
        self.plugins.iter().any(|plugin| plugin.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    const LINUX: &str = "x86_64-unknown-linux-gnu";
    const MACOS: &str = "aarch64-apple-darwin";

    fn manifest(edit: impl FnOnce(&mut serde_json::Value)) -> Manifest {
        let mut value = serde_json::json!({
            "schema": 1,
            "pull_request": 412,
            "head_sha": HEAD,
            "run_id": 77,
            "target": "x86_64-unknown-linux-gnu",
            "plugins": [{
                "id": "qol-cli-sessions",
                "version": "0.127.1",
                "tag": "qol-cli-sessions-v0.127.1",
                "file": "qol-cli-sessions-linux-x86_64",
                "bin_name": "qol-cli-sessions"
            }],
            "tray": ""
        });
        edit(&mut value);
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_build_of_the_pull_request_head_passes() {
        let build = manifest(|_| {});
        build.check(412, HEAD, LINUX).unwrap();
        assert_eq!(build.plugins[0].bin_name, "qol-cli-sessions");
        let mac = manifest(|value| value["target"] = MACOS.into());
        mac.check(412, HEAD, MACOS).unwrap();
    }

    #[test]
    fn a_build_of_another_pull_request_or_commit_is_refused() {
        let build = manifest(|_| {});
        assert!(build.check(413, HEAD, LINUX).is_err());
        assert!(build.check(412, "feed", LINUX).is_err());
        let error = build.check(412, HEAD, MACOS).unwrap_err().to_string();
        assert!(error.contains("not aarch64-apple-darwin"), "{error}");
        let other_target = manifest(|value| value["target"] = MACOS.into());
        assert!(other_target.check(412, HEAD, LINUX).is_err());
        let other_schema = manifest(|value| value["schema"] = 2.into());
        assert!(other_schema.check(412, HEAD, LINUX).is_err());
    }

    #[test]
    fn a_build_replaces_only_the_plugins_it_carries() {
        let build = manifest(|_| {});
        assert!(build.replaces("qol-cli-sessions"));
        assert!(!build.replaces("qol-alt-tab"));
    }

    #[test]
    fn a_manifest_naming_a_path_is_refused() {
        for field in ["id", "tag", "file", "bin_name"] {
            let build = manifest(|value| value["plugins"][0][field] = "../escape".into());
            assert!(build.check(412, HEAD, LINUX).is_err(), "{field}");
        }
    }

    #[test]
    fn only_plugin_settings_files_are_accepted() {
        let carried = |names: serde_json::Value| {
            manifest(|value| value["plugins"][0]["contract_files"] = names)
        };
        assert_eq!(manifest(|_| {}).plugins[0].contract_files, None);
        let build = carried(serde_json::json!(CONTRACT_FILES));
        build.check(412, HEAD, LINUX).unwrap();
        assert_eq!(build.plugins[0].contract_files.as_ref().unwrap().len(), 2);
        carried(serde_json::json!([]))
            .check(412, HEAD, LINUX)
            .unwrap();
        for name in ["plugin.toml", "../qol-config.toml", "qol-cli-sessions"] {
            let build = carried(serde_json::json!([name]));
            assert!(build.check(412, HEAD, LINUX).is_err(), "{name}");
        }
    }

    #[test]
    fn read_reports_a_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let error = read(&dir.path().join(FILE)).unwrap_err();
        assert!(format!("{error:#}").contains("manifest.json"));
    }
}
