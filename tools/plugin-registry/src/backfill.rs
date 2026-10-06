use crate::artifact::{push, release_from_git, Pushed};
use crate::oci::sha256_digest;
use crate::parallel::parallel_map;
use crate::registry::{read_body, send, Registry};
use crate::release::{newest_per_plugin, ReleaseTag};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const RELEASE_WORKERS: usize = 4;

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

pub struct Outcome {
    pub tag: String,
    pub pushed: Option<Pushed>,
}

pub fn backfill(
    registry: &Registry,
    agent: &ureq::Agent,
    github_repo: &str,
    github_token: Option<&str>,
    source: &str,
) -> Result<Vec<Outcome>> {
    let releases = github_releases(agent, github_repo, github_token)?;
    let current = current_plugin_ids(Path::new("plugins"))?;
    let latest = newest_per_plugin(releases.keys().map(String::as_str), 1);
    let selected: Vec<ReleaseTag> = latest
        .into_iter()
        .filter(|release| current.contains(&release.plugin_id))
        .collect();
    parallel_map(&selected, RELEASE_WORKERS, |release| {
        if registry.manifest(&release.tag)?.is_some() {
            return Ok(Outcome {
                tag: release.tag.clone(),
                pushed: None,
            });
        }
        let assets = download_assets(agent, &releases[&release.tag])?;
        let artifact = release_from_git(&release.tag, &release.tag, assets)?;
        Ok(Outcome {
            tag: release.tag.clone(),
            pushed: Some(push(registry, &artifact, source)?),
        })
    })
}

fn github_releases(
    agent: &ureq::Agent,
    github_repo: &str,
    github_token: Option<&str>,
) -> Result<BTreeMap<String, Vec<GithubAsset>>> {
    let mut releases = BTreeMap::new();
    for page in 1.. {
        let url =
            format!("https://api.github.com/repos/{github_repo}/releases?per_page=100&page={page}");
        let mut request = agent.get(&url).set("Accept", "application/vnd.github+json");
        if let Some(token) = github_token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        let response = send(request, None, &url)?;
        if response.status() != 200 {
            anyhow::bail!("{url} answered {}", response.status());
        }
        let batch: Vec<GithubRelease> = serde_json::from_slice(&read_body(response, &url)?)
            .with_context(|| format!("{url} answered an unreadable release list"))?;
        if batch.is_empty() {
            break;
        }
        releases.extend(
            batch
                .into_iter()
                .map(|release| (release.tag_name, release.assets)),
        );
    }
    Ok(releases)
}

fn download_assets(
    agent: &ureq::Agent,
    assets: &[GithubAsset],
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut downloaded = BTreeMap::new();
    for asset in assets {
        let url = &asset.browser_download_url;
        let response = send(agent.get(url), None, url)?;
        if response.status() != 200 {
            anyhow::bail!("{url} answered {}", response.status());
        }
        let bytes = read_body(response, url)?;
        let digest_matches = asset
            .digest
            .as_deref()
            .is_none_or(|digest| digest == sha256_digest(&bytes));
        if bytes.len() as u64 != asset.size || !digest_matches {
            anyhow::bail!("{url} does not match the size and digest GitHub lists");
        }
        downloaded.insert(asset.name.clone(), bytes);
    }
    Ok(downloaded)
}

fn current_plugin_ids(plugins: &Path) -> Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    for entry in
        std::fs::read_dir(plugins).with_context(|| format!("cannot read {}", plugins.display()))?
    {
        let manifest = entry?.path().join("plugin.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            ids.extend(crate::release::declared_id(&text));
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_plugin_ids_reads_each_plugin_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        for (dir, manifest) in [
            ("alt-tab", Some("[plugin]\nid = \"qol-alt-tab\"\n")),
            ("shot", Some("[plugin]\nid = \"qol-shot\"\n")),
            ("notes", None),
        ] {
            let path = tmp.path().join(dir);
            std::fs::create_dir_all(&path).unwrap();
            if let Some(manifest) = manifest {
                std::fs::write(path.join("plugin.toml"), manifest).unwrap();
            }
        }
        let ids = current_plugin_ids(tmp.path()).unwrap();
        assert_eq!(
            ids.into_iter().collect::<Vec<_>>(),
            ["qol-alt-tab", "qol-shot"]
        );
    }
}
