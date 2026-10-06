use super::document::{release, IndexedRelease};
use super::fetch::{self, blob_client};
use super::registry::fetch_blob;
use super::IndexLocation;
use anyhow::{Context, Result};
use std::path::Path;

pub(crate) async fn stage_release(
    location: &IndexLocation,
    plugin_id: &str,
    version: Option<&str>,
    plugin_dir: &Path,
) -> Result<IndexedRelease> {
    let release = release(&fetch::load(location).await?, plugin_id, version)?;
    unpack_release_tree(&release, plugin_dir).await?;
    log::info!(
        "Staged {} {} from the plugin index",
        release.plugin_id,
        release.version
    );
    Ok(release)
}

pub(crate) async fn load_config_contract(
    location: &IndexLocation,
    plugin_id: &str,
    version: Option<&str>,
    plugin_dir: &Path,
) -> Result<Option<qol_config::contract::ConfigSpec>> {
    let release = release(&fetch::load(location).await?, plugin_id, version)?;
    unpack_release_tree(&release, plugin_dir).await?;
    crate::plugins::config::load_config_contract_from_root(plugin_dir)
}

async fn unpack_release_tree(release: &IndexedRelease, plugin_dir: &Path) -> Result<()> {
    let label = format!("{} {} plugin tree", release.plugin_id, release.version);
    let tree = fetch_blob(
        &blob_client(),
        &release.registry,
        &release.files.tree,
        &label,
    )
    .await?;
    let staging_dir = plugin_dir.to_path_buf();
    tokio::task::spawn_blocking(move || unpack_tree(&tree, &staging_dir))
        .await
        .context("plugin tree unpacking stopped")??;
    require_release_identity(&release.plugin_id, &release.version, plugin_dir)
}

fn require_release_identity(plugin_id: &str, version: &str, plugin_dir: &Path) -> Result<()> {
    let manifest = crate::plugins::PluginManifest::read_from_dir(plugin_dir)?;
    let declared_id = manifest.plugin.require_declared_id()?;
    if declared_id.as_str() != plugin_id || manifest.plugin.version != version {
        anyhow::bail!(
            "the plugin tree for {plugin_id} {version} declares {} {}",
            declared_id.as_str(),
            manifest.plugin.version
        );
    }
    Ok(())
}

fn unpack_tree(bytes: &[u8], plugin_dir: &Path) -> Result<()> {
    if plugin_dir.exists() {
        std::fs::remove_dir_all(plugin_dir)
            .with_context(|| format!("failed to clear {}", plugin_dir.display()))?;
    }
    std::fs::create_dir_all(plugin_dir)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            anyhow::bail!("the plugin tree contains a link or special file");
        }
        if !entry.unpack_in(plugin_dir)? {
            anyhow::bail!("a plugin tree entry escapes {}", plugin_dir.display());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod fixtures {
    pub(crate) fn tree(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, content, mode) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder.append_data(&mut header, path, *content).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::tree;
    use super::*;

    #[test]
    fn unpack_tree_replaces_the_staging_dir_with_the_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("stale.txt"), "old").unwrap();
        let bytes = tree(&[
            ("plugin.toml", b"[plugin]\n", 0o644),
            ("shell/run.sh", b"#!/bin/sh\n", 0o755),
        ]);

        unpack_tree(&bytes, &plugin_dir).unwrap();

        assert!(!plugin_dir.join("stale.txt").exists());
        assert_eq!(
            std::fs::read(plugin_dir.join("plugin.toml")).unwrap(),
            b"[plugin]\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(plugin_dir.join("shell/run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn unpack_tree_refuses_links() {
        let tmp = tempfile::tempdir().unwrap();
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_cksum();
        builder
            .append_link(&mut header, "escape", "/etc/passwd")
            .unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();

        assert!(unpack_tree(&bytes, &tmp.path().join("staging")).is_err());
    }

    #[test]
    fn staged_tree_must_declare_the_indexed_id_and_version() {
        let tmp = tempfile::tempdir().unwrap();
        let cases: &[(&str, &str, bool)] = &[
            ("qol-shot", "1.0.0", true),
            ("qol-launcher", "1.0.0", false),
            ("qol-shot", "2.0.0", false),
        ];
        for (declared_id, declared_version, accepted) in cases {
            let manifest = format!(
                "[plugin]\nid = \"{declared_id}\"\nname = \"x\"\ndescription = \"\"\nversion = \"{declared_version}\"\n\n[menu]\nlabel = \"x\"\nitems = []\n"
            );
            std::fs::write(tmp.path().join("plugin.toml"), manifest).unwrap();
            let result = require_release_identity("qol-shot", "1.0.0", tmp.path());
            assert_eq!(
                result.is_ok(),
                *accepted,
                "{declared_id} {declared_version}: {result:?}"
            );
        }
    }
}
