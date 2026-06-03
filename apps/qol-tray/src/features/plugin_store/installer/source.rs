use super::super::github::{build_github_request, get_stored_token, send_checked};
use super::super::source::PluginSource;
use super::command::{run_git, run_git_checked};
use super::InstallSource;
use anyhow::Result;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

pub(super) async fn resolve_latest_plugin_version(
    source: &PluginSource,
    plugin_id: &str,
) -> Result<String> {
    let url = source.releases_api_url();
    let client = reqwest::Client::new();
    let token = get_stored_token();
    let request = build_github_request(&client, &url, token.as_deref());
    let response = send_checked(request).await?;
    let releases: Vec<ReleaseListEntry> = response.json().await?;
    let tags: Vec<&str> = releases.iter().map(|r| r.tag_name.as_str()).collect();
    let tag = super::super::source::select_release_tag(tags, plugin_id).ok_or_else(|| {
        anyhow::anyhow!(
            "no release tag prefixed with '{}-v' found in {}",
            plugin_id,
            source.repo
        )
    })?;
    super::super::source::version_from_plugin_tag(tag, plugin_id).ok_or_else(|| {
        anyhow::anyhow!(
            "selected release tag '{}' for {} is not valid semver",
            tag,
            plugin_id
        )
    })
}

#[derive(Deserialize)]
struct ReleaseListEntry {
    tag_name: String,
}

pub(super) async fn clone_source_repo(
    source: &PluginSource,
    staging_dir: &Path,
    plugin_id: &str,
    install_source: &InstallSource,
) -> Result<()> {
    let staging_str = path_utf8(staging_dir)?;
    let url = source.repo_clone_url();
    log::info!(
        "Cloning plugin source {} (repo={}) to {:?}",
        source.name,
        source.repo,
        staging_dir
    );
    run_git_checked(["clone", url.as_str(), staging_str], None, "clone").await?;
    checkout_install_source(source, staging_dir, plugin_id, install_source).await?;
    Ok(())
}

pub(super) async fn prepare_update_repo(
    source: &PluginSource,
    staging_dir: &Path,
    plugin_id: &str,
    install_source: &InstallSource,
) -> Result<()> {
    clone_source_repo(source, staging_dir, plugin_id, install_source).await?;
    if matches!(install_source, InstallSource::TaggedVersion(_)) {
        return Ok(());
    }
    let branch = get_default_branch(staging_dir).await?;
    reset_to_origin_head(staging_dir, &branch).await
}

fn path_utf8(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow::anyhow!("Plugin path contains invalid UTF-8"))
}

async fn get_default_branch(plugin_dir: &Path) -> Result<String> {
    let output = run_git(
        ["symbolic-ref", "refs/remotes/origin/HEAD", "--short"],
        Some(plugin_dir),
        Duration::from_secs(10),
        "symbolic-ref",
    )
    .await?;

    let branch = output_branch_name(&output);
    if let Some(branch) = branch {
        return Ok(branch);
    }

    Ok("main".to_string())
}

fn output_branch_name(output: &std::process::Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout);
    let branch = branch.trim().trim_start_matches("origin/");
    if !is_safe_branch_name(branch) {
        log::warn!("Invalid branch name from git: {:?}", branch);
        return None;
    }

    Some(branch.to_string())
}

async fn reset_to_origin_head(plugin_dir: &Path, branch: &str) -> Result<()> {
    let candidates = candidate_branches(branch);
    let mut last_error = String::new();

    for candidate in candidates {
        let output = reset_to_branch(plugin_dir, &candidate).await?;
        if output.status.success() {
            return Ok(());
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    }

    if last_error.is_empty() {
        anyhow::bail!("Git reset failed for origin/main and origin/master");
    }

    anyhow::bail!(
        "Git reset failed for origin/main and origin/master: {}",
        last_error
    )
}

fn candidate_branches(branch: &str) -> Vec<String> {
    let mut candidates = vec![branch.to_string()];
    if branch != "main" {
        candidates.push("main".to_string());
    }
    if branch != "master" {
        candidates.push("master".to_string());
    }
    candidates
}

async fn reset_to_branch(plugin_dir: &Path, branch: &str) -> Result<std::process::Output> {
    let reset_target = format!("origin/{}", branch);
    run_git(
        ["reset", "--hard", reset_target.as_str()],
        Some(plugin_dir),
        super::GIT_TIMEOUT,
        "reset",
    )
    .await
}

async fn checkout_install_source(
    source: &PluginSource,
    plugin_dir: &Path,
    plugin_id: &str,
    install_source: &InstallSource,
) -> Result<()> {
    let InstallSource::TaggedVersion(version) = install_source else {
        return Ok(());
    };
    let tag = plugin_release_tag(source, plugin_id, version)?;
    run_git_checked(
        ["checkout", "--detach", tag.as_str()],
        Some(plugin_dir),
        "checkout-version",
    )
    .await
    .map(|_| ())
}

fn plugin_release_tag(source: &PluginSource, plugin_id: &str, version: &str) -> Result<String> {
    if !is_safe_ref_component(version) {
        anyhow::bail!("invalid plugin version ref: {}", version);
    }
    if !is_safe_ref_component(plugin_id) {
        anyhow::bail!("invalid plugin id for ref: {}", plugin_id);
    }
    Ok(source.plugin_release_tag(plugin_id, version))
}

fn is_safe_ref_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' || ch == '+'
        })
        && !value.starts_with('-')
        && !value.starts_with('.')
        && !value.contains("..")
}

fn is_safe_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 256
        && branch.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' || ch == '.'
        })
        && !branch.starts_with('-')
        && !branch.starts_with('.')
        && !branch.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_source() -> PluginSource {
        PluginSource::new("core", "qol-tools/qol", "main")
    }

    #[test]
    fn is_safe_branch_name_cases() {
        let valid = [
            "main",
            "master",
            "develop",
            "feature/foo",
            "release-1.0",
            "v1.0.0",
            "fix_bug",
            "a",
        ];
        for branch in valid {
            assert!(is_safe_branch_name(branch), "should be valid: {:?}", branch);
        }

        let invalid = [
            "",
            "-leading-dash",
            ".hidden",
            "has..double-dots",
            "has\nline",
            "has\ttab",
            "has space",
            &"x".repeat(300),
        ];
        for branch in invalid {
            assert!(
                !is_safe_branch_name(branch),
                "should be invalid: {:?}",
                branch
            );
        }
    }

    #[test]
    fn plugin_release_tag_builds_monorepo_tag_format() {
        let s = core_source();
        let cases = [
            ("plugin-alt-tab", "1.2.3", Some("plugin-alt-tab-v1.2.3")),
            (
                "plugin-launcher",
                "0.1.0-beta.1",
                Some("plugin-launcher-v0.1.0-beta.1"),
            ),
        ];
        for (plugin_id, version, expected) in cases {
            assert_eq!(
                plugin_release_tag(&s, plugin_id, version).ok().as_deref(),
                expected,
                "plugin_id={:?}, version={:?}",
                plugin_id,
                version
            );
        }
    }

    #[test]
    fn plugin_release_tag_rejects_unsafe_input() {
        let s = core_source();
        let bad_versions = ["", "../x", "has space", ".hidden", "-leading"];
        for version in bad_versions {
            assert!(
                plugin_release_tag(&s, "plugin-alt-tab", version).is_err(),
                "version: {:?}",
                version
            );
        }
        let bad_ids = ["", "../id", "has space"];
        for id in bad_ids {
            assert!(plugin_release_tag(&s, id, "1.0.0").is_err(), "id: {:?}", id);
        }
    }
}
