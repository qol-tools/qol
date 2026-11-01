use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::Path;

pub(super) const FILE: &str = "manifest.json";
const SCHEMA: u32 = 1;
const TARGET: &str = "x86_64-unknown-linux-gnu";

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
}

pub(super) fn read(path: &Path) -> Result<Manifest> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("the build has no {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("{} is not a build manifest", path.display()))
}

impl Manifest {
    pub(super) fn check(&self, number: u64, head: &str) -> Result<()> {
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
        if self.target != TARGET {
            bail!("the build targets {}, not {TARGET}", self.target);
        }
        for plugin in &self.plugins {
            for value in [&plugin.id, &plugin.tag, &plugin.file, &plugin.bin_name] {
                if !super::plain_name(value) {
                    bail!("the build manifest names `{value}`, which is not a plain file name");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

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
        build.check(412, HEAD).unwrap();
        assert_eq!(build.plugins[0].bin_name, "qol-cli-sessions");
    }

    #[test]
    fn a_build_of_another_pull_request_or_commit_is_refused() {
        let build = manifest(|_| {});
        assert!(build.check(413, HEAD).is_err());
        assert!(build.check(412, "feed").is_err());
        let other_target = manifest(|value| value["target"] = "aarch64-apple-darwin".into());
        assert!(other_target.check(412, HEAD).is_err());
        let other_schema = manifest(|value| value["schema"] = 2.into());
        assert!(other_schema.check(412, HEAD).is_err());
    }

    #[test]
    fn a_manifest_naming_a_path_is_refused() {
        for field in ["id", "tag", "file", "bin_name"] {
            let build = manifest(|value| value["plugins"][0][field] = "../escape".into());
            assert!(build.check(412, HEAD).is_err(), "{field}");
        }
    }

    #[test]
    fn read_reports_a_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let error = read(&dir.path().join(FILE)).unwrap_err();
        assert!(format!("{error:#}").contains("manifest.json"));
    }
}
