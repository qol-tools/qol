use anyhow::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime};

pub(crate) mod platform;
pub mod version;

static LATEST_VERSION: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static UPDATE_STATE: OnceLock<Mutex<UpdateState>> = OnceLock::new();
static HOST_UPDATE_STATE: OnceLock<Mutex<HostUpdateState>> = OnceLock::new();
static TRAY_START: OnceLock<SystemTime> = OnceLock::new();
static UPDATED_FROM: OnceLock<String> = OnceLock::new();

pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60 * 60);
pub(crate) const UPDATE_ALREADY_RUNNING: &str = "An update is already running";
pub(crate) const UPDATE_FAILED: &str = "The update could not be installed";

pub(super) const GITHUB_REPO: &str = "qol-tools/qol";
pub(super) const HOST_TAG_PREFIX: &str = "qol-tray";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const UPDATE_FROM_MARKER: &str = ".update-from-version";
const NO_CONNECTION: &str = "No connection to GitHub";

struct UpdateState {
    last_attempt: Option<SystemTime>,
    last_success: Option<SystemTime>,
    last_error: Option<String>,
    checking: bool,
    etag: Option<String>,
    update_found: bool,
}

struct HostUpdateState {
    queued: bool,
    running: bool,
    progress: Option<u8>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

struct FetchedRelease {
    update_found: bool,
    latest: Option<String>,
    etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckHealth {
    pub checking: bool,
    pub last_attempt: Option<SystemTime>,
    pub last_success: Option<SystemTime>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostUpdateStatus {
    pub queued: bool,
    pub running: bool,
    pub progress: Option<u8>,
    pub error: Option<String>,
}

pub fn current_version() -> &'static str {
    CURRENT_VERSION
}

pub fn latest_version() -> Option<String> {
    match lock_latest_version() {
        Some(latest) => (*latest).clone(),
        None => None,
    }
}

pub fn checks_enabled() -> bool {
    !cfg!(feature = "dev") && platform::detect_install_kind() != platform::InstallKind::Development
}

pub fn note_tray_start() {
    let _ = TRAY_START.set(SystemTime::now());
}

pub fn tray_start() -> SystemTime {
    *TRAY_START.get_or_init(SystemTime::now)
}

pub fn check_health() -> CheckHealth {
    match lock_update_state() {
        Some(state) => CheckHealth {
            checking: state.checking,
            last_attempt: state.last_attempt,
            last_success: state.last_success,
            last_error: state.last_error.clone(),
        },
        None => CheckHealth {
            checking: false,
            last_attempt: None,
            last_success: None,
            last_error: None,
        },
    }
}

pub fn host_update_status() -> HostUpdateStatus {
    match lock_host_update_state() {
        Some(state) => HostUpdateStatus {
            queued: state.queued,
            running: state.running,
            progress: state.progress,
            error: state.error.clone(),
        },
        None => HostUpdateStatus {
            queued: false,
            running: false,
            progress: None,
            error: None,
        },
    }
}

pub fn updated_from() -> Option<String> {
    UPDATED_FROM.get().cloned()
}

pub async fn check_for_updates() -> Result<bool> {
    check_for_updates_inner(false).await
}

pub async fn check_for_updates_force() -> Result<bool> {
    check_for_updates_inner(true).await
}

async fn check_for_updates_inner(force: bool) -> Result<bool> {
    if !checks_enabled() {
        return Ok(false);
    }
    {
        let Some(mut state) = lock_update_state() else {
            return Ok(false);
        };
        let last_success_age = state.last_success.and_then(|moment| moment.elapsed().ok());
        if check_skippable(state.checking, force, last_success_age) {
            return Ok(state.update_found);
        }
        state.checking = true;
        state.last_attempt = Some(SystemTime::now());
    }
    let _in_flight = CheckInFlight;

    let outcome = fetch_host_release().await;

    let Some(mut state) = lock_update_state() else {
        return Ok(false);
    };
    match outcome {
        Ok(release) => {
            state.last_success = Some(SystemTime::now());
            state.last_error = None;
            state.etag = release.etag;
            state.update_found = release.update_found;
            store_latest_version(release.latest);
            Ok(state.update_found)
        }
        Err(message) => {
            state.last_error = Some(message.clone());
            Err(anyhow::anyhow!(message))
        }
    }
}

struct CheckInFlight;

impl Drop for CheckInFlight {
    fn drop(&mut self) {
        if let Some(mut state) = lock_update_state() {
            state.checking = false;
        }
    }
}

fn check_skippable(checking: bool, force: bool, last_success_age: Option<Duration>) -> bool {
    if checking {
        return true;
    }
    !force && last_success_age.is_some_and(|age| age < CHECK_INTERVAL)
}

async fn fetch_host_release() -> Result<FetchedRelease, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page={}",
        GITHUB_REPO,
        crate::features::plugin_store::source::RELEASES_PER_PAGE
    );
    let token = crate::credentials::github_bearer_token();
    let etag = lock_update_state().and_then(|state| state.etag.clone());
    let mut request = crate::features::plugin_store::github::build_github_request(
        http_client(),
        &url,
        token.as_deref(),
    );
    if let Some(etag) = &etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .map_err(|_| NO_CONNECTION.to_string())?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        let Some(state) = lock_update_state() else {
            return Err(NO_CONNECTION.to_string());
        };
        return Ok(FetchedRelease {
            update_found: state.update_found,
            latest: latest_version(),
            etag: state.etag.clone(),
        });
    }
    if !status.is_success() {
        return Err(github_status_message(status.as_u16()));
    }
    let new_etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let releases: Vec<GitHubRelease> = response
        .json()
        .await
        .map_err(|_| NO_CONNECTION.to_string())?;
    if releases.len() == crate::features::plugin_store::source::RELEASES_PER_PAGE {
        log::warn!(
            "release list page is full ({} releases); the newest tag may be outside the fetched window",
            releases.len()
        );
    }
    let newest = pick_latest_host_version(&releases);
    let update_found = newest
        .as_deref()
        .map(|latest| is_newer_version(latest, CURRENT_VERSION))
        .unwrap_or(false);
    match newest.as_deref() {
        Some(latest) if update_found => {
            log::info!("Update available: {} -> {}", CURRENT_VERSION, latest)
        }
        Some(_) => log::info!("No updates available (current: {})", CURRENT_VERSION),
        None => log::info!("No qol-tray-v* releases published yet (current: {CURRENT_VERSION})"),
    }
    Ok(FetchedRelease {
        update_found,
        latest: newest.filter(|_| update_found),
        etag: new_etag,
    })
}

pub(crate) fn github_status_message(status: u16) -> String {
    if status == reqwest::StatusCode::FORBIDDEN.as_u16()
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS.as_u16()
    {
        "GitHub rate limit reached".to_string()
    } else {
        format!("GitHub returned {status}")
    }
}

pub(crate) fn plain_update_failure(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    if text.contains("already running") || text.contains("already in progress") {
        return UPDATE_ALREADY_RUNNING.to_string();
    }
    if text.contains("disabled in development") {
        return "Development builds update through Recompile".to_string();
    }
    if text.contains("No update version available") {
        return "No update is available".to_string();
    }
    if has_connection_failure(error) {
        return NO_CONNECTION.to_string();
    }
    if let Some(status) = github_status_in(&text) {
        return github_status_message(status);
    }
    UPDATE_FAILED.to_string()
}

fn has_connection_failure(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout() || error.is_connect())
    })
}

fn github_status_in(text: &str) -> Option<u16> {
    let rest = text.split("GitHub API returned ").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

pub(crate) struct HostUpdateLease(());

pub(crate) fn claim_host_update() -> Result<HostUpdateLease, String> {
    let Some(mut state) = lock_host_update_state() else {
        return Err("The update state is unavailable".to_string());
    };
    if state.running {
        return Err(UPDATE_ALREADY_RUNNING.to_string());
    }
    state.running = true;
    state.queued = false;
    state.progress = None;
    state.error = None;
    Ok(HostUpdateLease(()))
}

impl Drop for HostUpdateLease {
    fn drop(&mut self) {
        if let Some(mut state) = lock_host_update_state() {
            state.running = false;
            state.progress = None;
        }
    }
}

pub(crate) fn mark_host_update_queued() {
    if let Some(mut state) = lock_host_update_state() {
        state.queued = true;
    }
}

pub(crate) fn clear_host_update_queued() {
    if let Some(mut state) = lock_host_update_state() {
        state.queued = false;
    }
}

fn record_update_progress(percent: u8) {
    if let Some(mut state) = lock_host_update_state() {
        state.progress = Some(percent);
    }
}

fn record_update_error(message: String) {
    if let Some(mut state) = lock_host_update_state() {
        state.error = Some(message);
    }
}

fn lock_latest_version() -> Option<MutexGuard<'static, Option<String>>> {
    match LATEST_VERSION.get_or_init(|| Mutex::new(None)).lock() {
        Ok(guard) => Some(guard),
        Err(error) => {
            log::error!("Latest version lock is poisoned: {}", error);
            None
        }
    }
}

fn store_latest_version(latest: Option<String>) {
    if let Some(mut guard) = lock_latest_version() {
        *guard = latest;
    }
}

fn lock_update_state() -> Option<MutexGuard<'static, UpdateState>> {
    match update_state().lock() {
        Ok(guard) => Some(guard),
        Err(error) => {
            log::error!("Update state lock is poisoned: {}", error);
            None
        }
    }
}

fn lock_host_update_state() -> Option<MutexGuard<'static, HostUpdateState>> {
    match host_update_state().lock() {
        Ok(guard) => Some(guard),
        Err(error) => {
            log::error!("Host update state lock is poisoned: {}", error);
            None
        }
    }
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
            last_attempt: None,
            last_success: None,
            last_error: None,
            checking: false,
            etag: None,
            update_found: false,
        })
    })
}

fn host_update_state() -> &'static Mutex<HostUpdateState> {
    HOST_UPDATE_STATE.get_or_init(|| {
        Mutex::new(HostUpdateState {
            queued: false,
            running: false,
            progress: None,
            error: None,
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
    let lease = claim_host_update().map_err(|message| anyhow::anyhow!(message))?;
    run_host_update(lease, events).await
}

pub(crate) async fn run_host_update(
    lease: HostUpdateLease,
    events: std::sync::Arc<crate::daemon::EventBus>,
) -> Result<()> {
    record_update_progress(0);
    let result = platform::download_and_install(events).await;
    if let Err(error) = &result {
        log::error!("Self-update failed: {error:#}");
        record_update_error(plain_update_failure(error));
    }
    drop(lease);
    result
}

fn update_marker_path() -> Option<PathBuf> {
    crate::paths::shared_config_dir()
        .ok()
        .map(|dir| dir.join(UPDATE_FROM_MARKER))
}

pub(crate) fn write_pending_update_marker() -> Option<PathBuf> {
    let marker = update_marker_path()?;
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&marker, CURRENT_VERSION) {
        Ok(()) => Some(marker),
        Err(error) => {
            log::warn!("Failed to write the update confirmation marker: {}", error);
            None
        }
    }
}

pub fn consume_update_confirmation() -> Option<String> {
    let marker = update_marker_path()?;
    if !marker.exists() {
        return None;
    }
    let from = std::fs::read_to_string(&marker).ok();
    let _ = std::fs::remove_file(&marker);
    let from = from?.trim().to_string();
    if from.is_empty() || from == CURRENT_VERSION {
        return None;
    }
    let _ = UPDATED_FROM.set(from.clone());
    Some(from)
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

    #[test]
    fn github_status_messages_are_plain_words() {
        let cases = [
            (403, "GitHub rate limit reached"),
            (429, "GitHub rate limit reached"),
            (404, "GitHub returned 404"),
            (500, "GitHub returned 500"),
        ];
        for (status, expected) in cases {
            assert_eq!(github_status_message(status), expected, "status={status}");
        }
    }

    #[test]
    fn github_status_is_extracted_from_a_check_error_message() {
        let cases = [
            (
                "GitHub API returned 403 Forbidden: API rate limit exceeded",
                Some(403),
            ),
            ("GitHub API returned 404 Not Found: not found", Some(404)),
            ("Failed to replace /usr/bin/qol-tray", None),
        ];
        for (text, expected) in cases {
            assert_eq!(github_status_in(text), expected, "text={text}");
        }
    }

    #[test]
    fn check_reuse_rule_table() {
        let fresh = Some(Duration::from_secs(60));
        let expired = Some(CHECK_INTERVAL);
        let cases = [
            ("check already running, not forced", true, false, None, true),
            ("check already running, forced", true, true, None, true),
            ("fresh cache, not forced", false, false, fresh, true),
            (
                "cache exactly at the interval",
                false,
                false,
                expired,
                false,
            ),
            (
                "stale cache, not forced",
                false,
                false,
                Some(CHECK_INTERVAL + Duration::from_secs(1)),
                false,
            ),
            ("no success recorded, not forced", false, false, None, false),
            ("fresh cache, forced", false, true, fresh, false),
        ];
        for (name, checking, force, age, expected) in cases {
            assert_eq!(
                check_skippable(checking, force, age),
                expected,
                "case: {name}"
            );
        }
    }

    #[test]
    fn check_in_flight_guard_resets_the_checking_flag() {
        if let Some(mut state) = lock_update_state() {
            state.checking = true;
        }
        drop(CheckInFlight);
        assert!(lock_update_state().is_some_and(|state| !state.checking));
    }

    #[test]
    fn host_update_claim_is_single_flight() {
        let lease = claim_host_update().expect("first claim should succeed");
        assert!(claim_host_update().is_err());
        drop(lease);
        let release = claim_host_update();
        assert!(release.is_ok());
        drop(release);
    }

    #[test]
    fn update_failures_are_reported_in_plain_words() {
        let cases = [
            (
                "Self-update is disabled in development builds",
                "Development builds update through Recompile",
            ),
            ("No update version available", "No update is available"),
            (
                "An update is already running",
                "An update is already running",
            ),
            (
                "Plugin operation already in progress: plugin-launcher",
                "An update is already running",
            ),
            (
                "GitHub API returned 403 Forbidden: API rate limit exceeded",
                "GitHub rate limit reached",
            ),
            (
                "GitHub API returned 500 Internal Server Error",
                "GitHub returned 500",
            ),
            (
                "Failed to replace /usr/bin/qol-tray",
                "The update could not be installed",
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(
                plain_update_failure(&anyhow::anyhow!("{text}")),
                expected,
                "text={text}"
            );
        }
    }
}
