use anyhow::Result;
pub(super) use qol_plugin_index::{
    verify_signed, Blob, IndexDocument, IndexedPlugin, IndexedVersion, RegistryLocation,
};

#[derive(Debug, Clone)]
pub(crate) struct IndexedRelease {
    pub(super) plugin_id: String,
    pub(super) version: String,
    pub(super) registry: RegistryLocation,
    pub(super) files: IndexedVersion,
}

pub(super) fn release(
    document: &IndexDocument,
    plugin_id: &str,
    version: Option<&str>,
) -> Result<IndexedRelease> {
    let plugin = document
        .plugins
        .get(plugin_id)
        .ok_or_else(|| anyhow::anyhow!("the plugin index does not list {plugin_id}"))?;
    let version = version.unwrap_or(&plugin.latest);
    let files = plugin
        .versions
        .get(version)
        .ok_or_else(|| anyhow::anyhow!("the plugin index does not list {plugin_id} {version}"))?;
    Ok(IndexedRelease {
        plugin_id: plugin_id.to_string(),
        version: version.to_string(),
        registry: document.registry.clone(),
        files: files.clone(),
    })
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::{json, Value};

    pub(crate) use qol_plugin_index::test_signer::TestSigner as Signer;

    pub(crate) fn blob(digest: &str, size: u64) -> Value {
        json!({ "digest": digest, "size": size })
    }

    pub(crate) fn version(plugin_id: &str, version: &str, tree: Value, assets: Value) -> Value {
        let plugin_toml = format!(
            "[plugin]\nid = \"{plugin_id}\"\nname = \"{plugin_id}\"\ndescription = \"test\"\nversion = \"{version}\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n\n[menu]\nlabel = \"{plugin_id}\"\nitems = []\n\n[runtime]\ncommand = \"{plugin_id}\"\n\n[[dependencies.binaries]]\nname = \"{plugin_id}\"\nrepo = \"qol-tools/{plugin_id}\"\npattern = \"{plugin_id}-{{os}}-{{arch}}\"\n"
        );
        json!({
            "manifest_digest": "sha256:manifest",
            "dir": plugin_id.trim_start_matches("qol-"),
            "plugin_toml": plugin_toml,
            "tree": tree,
            "assets": assets,
        })
    }

    pub(crate) fn document(serial: u64, registry_url: &str, plugins: Value) -> Value {
        json!({
            "schema": 1,
            "serial": serial,
            "registry": { "url": registry_url, "repository": "qol-tools/plugins" },
            "plugins": plugins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{blob, document, version, Signer};
    use super::*;
    use serde_json::json;

    fn two_version_index(serial: u64) -> Vec<u8> {
        let plugins = json!({
            "qol-shot": {
                "latest": "1.2.0",
                "versions": {
                    "1.1.0": version("qol-shot", "1.1.0", blob("sha256:t1", 1), json!({})),
                    "1.2.0": version("qol-shot", "1.2.0", blob("sha256:t2", 2), json!({})),
                }
            }
        });
        serde_json::to_vec(&document(serial, "https://ghcr.io", plugins)).unwrap()
    }

    #[test]
    fn release_picks_latest_or_the_requested_version() {
        let signer = Signer::generate();
        let body = two_version_index(1);
        let document = verify_signed(&signer.sign_index(&body), &signer.public_key()).unwrap();
        let cases: &[(Option<&str>, &str, &str)] = &[
            (None, "1.2.0", "sha256:t2"),
            (Some("1.1.0"), "1.1.0", "sha256:t1"),
        ];
        for (requested, expected, tree) in cases {
            let release = release(&document, "qol-shot", *requested).unwrap();
            assert_eq!(release.version, *expected);
            assert_eq!(release.files.tree.digest, *tree);
            assert_eq!(release.files.dir, "shot");
        }
        assert!(release(&document, "qol-shot", Some("9.9.9")).is_err());
        assert!(release(&document, "qol-missing", None).is_err());
    }
}
