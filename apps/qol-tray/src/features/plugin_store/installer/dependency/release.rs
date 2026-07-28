use super::{DependencyPlan, ReleaseTagPick};
use crate::features::plugin_store::release_integrity::{self, GitHubRelease};
use anyhow::Result;

pub(super) async fn download_dependency_binary(plan: &DependencyPlan<'_>) -> Result<bool> {
    log::info!(
        "Fetching {} from {} ({:?})",
        plan.asset_name,
        plan.asset_repo(),
        plan.release_tag
    );

    let release = fetch_release(plan.asset_repo(), &plan.release_tag).await?;
    release_integrity::require_immutable_release(&release)?;

    if !release
        .assets
        .iter()
        .any(|asset| asset.name == plan.asset_name)
    {
        return missing_asset(plan);
    }

    let asset = release_integrity::verified_asset(&release, &plan.asset_name)?;
    release_integrity::download_verified(&asset, &plan.binary_path).await?;
    Ok(true)
}

fn missing_asset(plan: &DependencyPlan<'_>) -> Result<bool> {
    log::warn!(
        "Release asset '{}' missing for {} ({:?})",
        plan.asset_name,
        plan.asset_repo(),
        plan.release_tag
    );
    Ok(false)
}

async fn fetch_release(repo: &str, release_tag: &ReleaseTagPick) -> Result<GitHubRelease> {
    match release_tag {
        ReleaseTagPick::Latest => release_integrity::fetch_latest_release(repo).await,
        ReleaseTagPick::PluginTag(tag) => release_integrity::fetch_release(repo, tag).await,
    }
}

#[cfg(test)]
fn release_url(repo: &str, release_tag: &ReleaseTagPick) -> String {
    match release_tag {
        ReleaseTagPick::Latest => {
            format!("https://api.github.com/repos/{}/releases/latest", repo)
        }
        ReleaseTagPick::PluginTag(tag) => {
            format!(
                "https://api.github.com/repos/{}/releases/tags/{}",
                repo, tag
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_url_uses_per_plugin_tag_against_source_repo_not_dep_repo() {
        let cases: &[(&str, ReleaseTagPick, &str)] = &[
            (
                "qol-tools/qol",
                ReleaseTagPick::Latest,
                "https://api.github.com/repos/qol-tools/qol/releases/latest",
            ),
            (
                "qol-tools/qol",
                ReleaseTagPick::PluginTag("plugin-alt-tab-v1.2.3".to_string()),
                "https://api.github.com/repos/qol-tools/qol/releases/tags/plugin-alt-tab-v1.2.3",
            ),
            (
                "qol-tools/qol",
                ReleaseTagPick::PluginTag("plugin-launcher-v0.4.0".to_string()),
                "https://api.github.com/repos/qol-tools/qol/releases/tags/plugin-launcher-v0.4.0",
            ),
        ];
        for (repo, tag, expected) in cases {
            assert_eq!(
                release_url(repo, tag),
                *expected,
                "repo={} tag={:?}",
                repo,
                tag
            );
        }
    }
}
