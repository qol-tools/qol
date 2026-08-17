use anyhow::Result;
use serde::Deserialize;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

pub(crate) mod platform;
pub mod version;

static LATEST_VERSION: OnceLock<String> = OnceLock::new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static UPDATE_STATE: OnceLock<Mutex<UpdateState>> = OnceLock::new();

const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60 * 60);

pub(super) const GITHUB_REPO: &str = "qol-tools/qol";
pub(super) const HOST_TAG_PREFIX: &str = "qol-tray";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

struct UpdateState {
    last_check: Option<SystemTime>,
    etag: Option<String>,
    update_found: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

pub fn latest_version() -> Option<&'static str> {
    LATEST_VERSION.get().map(|s| s.as_str())
}

pub async fn check_for_updates() -> Result<bool> {
    check_for_updates_inner(false).await
}

pub async fn check_for_updates_force() -> Result<bool> {
    check_for_updates_inner(true).await
}

async fn check_for_updates_inner(force: bool) -> Result<bool> {
    let state = update_state();
    if !force {
        let guard = state.lock().expect("update state lock");
        if let Some(last) = guard.last_check {
            if last
                .elapsed()
                .map(|age| age < CHECK_INTERVAL)
                .unwrap_or(false)
            {
                return Ok(guard.update_found);
            }
        }
    }

    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page={}",
        GITHUB_REPO,
        crate::features::plugin_store::source::RELEASES_PER_PAGE
    );
    let token = crate::credentials::github_bearer_token();
    let etag = state.lock().expect("update state lock").etag.clone();
    let mut request = crate::features::plugin_store::github::build_github_request(
        http_client(),
        &url,
        token.as_deref(),
    );
    if let Some(etag) = &etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().await?;
    let (update_found, new_etag) = if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        let guard = state.lock().expect("update state lock");
        (guard.update_found, guard.etag.clone())
    } else if response.status().is_success() {
        let new_etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let releases: Vec<GitHubRelease> = response.json().await?;
        if releases.len() == crate::features::plugin_store::source::RELEASES_PER_PAGE {
            log::warn!(
                    "release list page is full ({} releases); the newest tag may be outside the fetched window",
                    releases.len()
                );
        }
        let update_found = match pick_latest_host_version(&releases) {
            Some(latest) => {
                let newer = is_newer_version(&latest, CURRENT_VERSION);
                if newer {
                    log::info!("Update available: {} -> {}", CURRENT_VERSION, latest);
                    let _ = LATEST_VERSION.set(latest);
                } else {
                    log::info!("No updates available (current: {})", CURRENT_VERSION);
                }
                newer
            }
            None => {
                log::info!("No qol-tray-v* releases published yet (current: {CURRENT_VERSION})");
                false
            }
        };
        (update_found, new_etag)
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API returned {}: {}", status, body);
    };

    let mut guard = state.lock().expect("update state lock");
    guard.last_check = Some(SystemTime::now());
    guard.etag = new_etag;
    guard.update_found = update_found;
    Ok(update_found)
}

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

fn update_state() -> &'static Mutex<UpdateState> {
    UPDATE_STATE.get_or_init(|| {
        Mutex::new(UpdateState {
            last_check: None,
            etag: None,
            update_found: false,
        })
    })
}

fn pick_latest_host_version(releases: &[GitHubRelease]) -> Option<String> {
    use crate::features::plugin_store::source::{select_release_tag, version_from_plugin_tag};
    let tag = select_release_tag(
        releases.iter().map(|r| r.tag_name.as_str()),
        HOST_TAG_PREFIX,
    )?;
    version_from_plugin_tag(tag, HOST_TAG_PREFIX)
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    use crate::version::Version;
    Version::parse(latest).is_newer_than(&Version::parse(current))
}

pub async fn download_and_install(events: std::sync::Arc<crate::daemon::EventBus>) -> Result<()> {
    platform::download_and_install(events).await
}

pub(super) fn verify_host_update(
    path: &Path,
    expected_version: Option<&str>,
    target_expectation: fn(
        qol_artifact::ArtifactExpectation,
        &str,
    ) -> qol_artifact::ArtifactExpectation,
) -> Result<()> {
    let running = qol_conventions::artifact::current()
        .ok_or_else(|| anyhow::anyhow!("running build identity is unavailable"))?;
    let expectation =
        host_update_expectation(&running.target, expected_version, target_expectation);
    let inspected = qol_artifact::verify_path(path, &expectation)?;
    log::info!(
        "[artifact-identity] verified self-update binary path={} version={} slices={}",
        path.display(),
        expected_version.unwrap_or("<dev-override>"),
        inspected.slices.len()
    );
    Ok(())
}

pub(super) fn verify_courier_update(path: &Path) -> Result<()> {
    let expectation = qol_artifact::ArtifactExpectation::production(
        qol_conventions::artifact::COURIER_BINARY_NAME,
        qol_conventions::artifact::COURIER_PACKAGE_NAME,
        qol_conventions::artifact::BuildRole::Courier,
    );
    let inspected = qol_artifact::verify_path(path, &expectation)?;
    log::info!(
        "[artifact-identity] verified self-update courier path={} slices={}",
        path.display(),
        inspected.slices.len()
    );
    Ok(())
}

fn host_update_expectation(
    running_target: &str,
    expected_version: Option<&str>,
    target_expectation: fn(
        qol_artifact::ArtifactExpectation,
        &str,
    ) -> qol_artifact::ArtifactExpectation,
) -> qol_artifact::ArtifactExpectation {
    let mut expectation = qol_artifact::ArtifactExpectation::production(
        qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
        qol_conventions::artifact::TRAY_PACKAGE_NAME,
        qol_conventions::artifact::BuildRole::Host,
    );
    expectation = target_expectation(expectation, running_target);
    if let Some(version) = expected_version {
        expectation = expectation.with_version(version);
    }
    expectation
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_conventions::artifact::{
        BuildFlavor, BuildIdentity, BuildIntent, BuildProfile, BuildRole, CompilerFacts,
        SourceIdentity, SCHEMA_VERSION,
    };

    fn rel(tag: &str) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
        }
    }

    fn update_identity(version: &str, target: &str) -> BuildIdentity {
        BuildIdentity {
            schema: SCHEMA_VERSION,
            binary: qol_conventions::artifact::TRAY_HOST_BINARY_NAME.to_string(),
            role: BuildRole::Host,
            package: qol_conventions::artifact::TRAY_PACKAGE_NAME.to_string(),
            version: version.to_string(),
            target: target.to_string(),
            intent: BuildIntent::Production,
            flavor: BuildFlavor {
                profile: BuildProfile::Release,
                dev_features: false,
            },
            compiler: CompilerFacts {
                cargo_profile: "release".to_string(),
                opt_level: "3".to_string(),
                debuginfo: false,
                debug_assertions: false,
                overflow_checks: None,
                test: false,
            },
            features: vec!["default".to_string()],
            source: SourceIdentity::Git {
                commit: "a".repeat(40),
                head_tree: "b".repeat(40),
                working_tree: "b".repeat(40),
            },
        }
    }

    #[test]
    fn self_update_policy_rejects_wrong_version_target_and_intent() {
        let target = "x86_64-unknown-linux-gnu";
        let expectation = host_update_expectation(
            target,
            Some("3.41.0"),
            qol_artifact::ArtifactExpectation::with_exact_target,
        );
        qol_artifact::verify_identity(&update_identity("3.41.0", target), &expectation).unwrap();

        let wrong_version = update_identity("3.40.6", target);
        assert!(qol_artifact::verify_identity(&wrong_version, &expectation).is_err());

        let wrong_target = update_identity("3.41.0", "x86_64-pc-windows-msvc");
        assert!(qol_artifact::verify_identity(&wrong_target, &expectation).is_err());

        let mut development = update_identity("3.41.0", target);
        development.intent = BuildIntent::Development;
        assert!(qol_artifact::verify_identity(&development, &expectation).is_err());
    }

    #[test]
    fn pick_latest_host_version_strips_qol_tray_prefix() {
        let cases = [
            (vec!["qol-tray-v1.2.3"], Some("1.2.3")),
            (vec!["qol-tray-v0.6.0"], Some("0.6.0")),
            (vec!["qol-tray-v9.9.9-beta.1"], Some("9.9.9-beta.1")),
        ];
        for (tags, expected) in cases {
            let releases: Vec<_> = tags.iter().map(|t| rel(t)).collect();
            assert_eq!(
                pick_latest_host_version(&releases).as_deref(),
                expected,
                "tags: {tags:?}"
            );
        }
    }

    #[test]
    fn pick_latest_host_version_ignores_plugin_tags() {
        let releases = vec![
            rel("plugin-launcher-v1.8.0"),
            rel("plugin-alt-tab-v2.0.1"),
            rel("plugin-keyremap-v0.3.0"),
        ];
        assert_eq!(pick_latest_host_version(&releases), None);
    }

    #[test]
    fn pick_latest_host_version_picks_max_host_version_when_mixed() {
        let releases = vec![
            rel("plugin-launcher-v1.8.0"),
            rel("qol-tray-v3.1.0"),
            rel("qol-tray-v3.2.1"),
            rel("plugin-alt-tab-v2.0.1"),
        ];
        assert_eq!(
            pick_latest_host_version(&releases).as_deref(),
            Some("3.2.1")
        );
    }

    #[test]
    fn pick_latest_host_version_returns_none_when_empty() {
        let releases: Vec<GitHubRelease> = vec![];
        assert_eq!(pick_latest_host_version(&releases), None);
    }

    #[test]
    fn pick_latest_host_version_rejects_collision_with_other_prefix() {
        let releases = vec![rel("qol-tray-doctor-v1.0.0")];
        assert_eq!(pick_latest_host_version(&releases), None);
    }

    #[test]
    fn is_newer_version_strict_ordering() {
        let cases = [
            ("1.2.4", "1.2.3", true),
            ("2.0.0", "1.99.99", true),
            ("1.2.3", "1.2.3", false),
            ("1.2.2", "1.2.3", false),
            ("0.6.0", "0.5.99", true),
        ];
        for (latest, current, expected) in cases {
            assert_eq!(
                is_newer_version(latest, current),
                expected,
                "latest={latest} current={current}"
            );
        }
    }

    #[test]
    fn host_tag_prefix_matches_release_workflow() {
        assert_eq!(HOST_TAG_PREFIX, "qol-tray");
    }

    #[test]
    fn github_repo_targets_monorepo() {
        assert_eq!(GITHUB_REPO, "qol-tools/qol");
    }
}
