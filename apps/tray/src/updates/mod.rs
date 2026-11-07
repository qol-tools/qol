use crate::features::plugin_store::release_integrity::{self, IndexedHostRelease};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime};

pub(crate) mod jobs;
pub(crate) mod platform;
pub mod version;
mod workspace_build;

pub use platform::recover_interrupted_update;

static LATEST_UPDATE: OnceLock<Mutex<Option<FoundUpdate>>> = OnceLock::new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static UPDATE_STATE: OnceLock<Mutex<UpdateState>> = OnceLock::new();
static TRAY_START: OnceLock<SystemTime> = OnceLock::new();
static UPDATED_FROM: OnceLock<String> = OnceLock::new();

pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60 * 60);
const OFFLINE_RETRY: Duration = Duration::from_secs(60);
pub(crate) const UPDATE_ALREADY_RUNNING: &str = "An update is already running";
pub(crate) const UPDATE_FAILED: &str = "The update could not be installed";

#[derive(Debug)]
struct IncompatibleUpdate;

impl std::fmt::Display for IncompatibleUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("This update cannot run on your system; your current version was kept")
    }
}

impl std::error::Error for IncompatibleUpdate {}

pub(super) const GITHUB_REPO: &str = "qol-tools/qol";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const NO_CONNECTION: &str = "No connection to GitHub";

struct UpdateState {
    last_attempt: Option<SystemTime>,
    last_success: Option<SystemTime>,
    last_error: Option<String>,
    checking: bool,
    update_found: bool,
}

struct FetchedRelease {
    update_found: bool,
    latest: Option<FoundUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FoundUpdate {
    PluginIndex(IndexedHostRelease),
}

impl FoundUpdate {
    pub(super) fn version(&self) -> &str {
        match self {
            Self::PluginIndex(release) => &release.version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckHealth {
    pub checking: bool,
    pub last_attempt: Option<SystemTime>,
    pub last_success: Option<SystemTime>,
    pub last_error: Option<String>,
}

impl CheckHealth {
    pub fn latest_succeeded(&self) -> bool {
        self.last_success.is_some() && self.last_error.is_none()
    }
}

pub fn current_version() -> &'static str {
    CURRENT_VERSION
}

pub fn latest_version() -> Option<String> {
    latest_update().map(|update| update.version().to_string())
}

fn latest_update() -> Option<FoundUpdate> {
    match lock_latest_update() {
        Some(latest) => (*latest).clone(),
        None => None,
    }
}

pub fn checks_enabled() -> bool {
    !cfg!(feature = "dev")
        && platform::detect_install_kind() != platform::InstallKind::Development
        && !crate::installer::stand_in::is_stand_in()
}

pub fn host_update_available() -> bool {
    checks_enabled() && lock_update_state().is_some_and(|state| state.update_found)
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

pub fn updated_from() -> Option<String> {
    UPDATED_FROM.get().cloned()
}

pub async fn check_for_updates() -> Result<bool> {
    if !checks_enabled() {
        return Ok(false);
    }
    {
        let Some(mut state) = lock_update_state() else {
            return Ok(false);
        };
        if let Some(running) = begin_check(&mut state, SystemTime::now()) {
            return Ok(running);
        }
    }
    let _in_flight = CheckInFlight;

    let outcome = fetch_host_update().await;

    let Some(mut state) = lock_update_state() else {
        return Ok(false);
    };
    match outcome {
        Ok(release) => {
            state.last_success = Some(SystemTime::now());
            state.last_error = None;
            state.update_found = release.update_found;
            store_latest_update(release.latest);
            Ok(state.update_found)
        }
        Err(message) => {
            state.last_error = Some(message.clone());
            Err(anyhow::anyhow!(message))
        }
    }
}

fn begin_check(state: &mut UpdateState, now: SystemTime) -> Option<bool> {
    if state.checking {
        return Some(state.update_found);
    }
    state.checking = true;
    state.last_attempt = Some(now);
    None
}

struct CheckInFlight;

impl Drop for CheckInFlight {
    fn drop(&mut self) {
        if let Some(mut state) = lock_update_state() {
            state.checking = false;
        }
    }
}

pub(crate) fn next_check_delay() -> Duration {
    check_delay(check_health().last_error.as_deref())
}

fn check_delay(last_error: Option<&str>) -> Duration {
    if last_error == Some(NO_CONNECTION) {
        OFFLINE_RETRY
    } else {
        CHECK_INTERVAL
    }
}

async fn fetch_host_update() -> Result<FetchedRelease, String> {
    let indexed = release_integrity::latest_host_release(platform::index_os()).await;
    let Some(release) = newer_index_release(indexed, CURRENT_VERSION)? else {
        log::info!("No updates available (current: {})", CURRENT_VERSION);
        return Ok(no_update());
    };
    if workspace_build::is_running() && !workspace_build::precedes_release(&release.revision).await
    {
        log::info!("No updates available (current: {})", CURRENT_VERSION);
        return Ok(no_update());
    }
    log::info!(
        "Update available: {} -> {} (plugin index)",
        CURRENT_VERSION,
        release.version
    );
    Ok(FetchedRelease {
        update_found: true,
        latest: Some(FoundUpdate::PluginIndex(release)),
    })
}

fn no_update() -> FetchedRelease {
    FetchedRelease {
        update_found: false,
        latest: None,
    }
}

fn newer_index_release(
    indexed: Result<Option<IndexedHostRelease>>,
    current: &str,
) -> Result<Option<IndexedHostRelease>, String> {
    match indexed {
        Ok(Some(release)) if version::is_newer_version(&release.version, current) => {
            Ok(Some(release))
        }
        Ok(Some(_) | None) => Ok(None),
        Err(error) => {
            log::warn!("Could not read qol-tray updates from the plugin index: {error:#}");
            Err(plain_update_failure(&error))
        }
    }
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
    if error.is::<IncompatibleUpdate>() {
        return IncompatibleUpdate.to_string();
    }
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
            .is_some_and(crate::features::connection_health::is_unreachable)
    })
}

fn github_status_in(text: &str) -> Option<u16> {
    let rest = text.split("GitHub API returned ").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

fn record_update_progress(percent: u8) {
    jobs::QUEUE.set_progress(jobs::HOST_ID, percent);
}

fn lock_latest_update() -> Option<MutexGuard<'static, Option<FoundUpdate>>> {
    match LATEST_UPDATE.get_or_init(|| Mutex::new(None)).lock() {
        Ok(guard) => Some(guard),
        Err(error) => {
            log::error!("Latest version lock is poisoned: {}", error);
            None
        }
    }
}

fn store_latest_update(latest: Option<FoundUpdate>) {
    if let Some(mut guard) = lock_latest_update() {
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

pub(crate) const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(CHECK_TIMEOUT)
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
            update_found: false,
        })
    })
}

const STAND_IN_REFUSAL: &str =
    "This qol-tray build runs in place of the installed one and does not update itself. \
     Update once the installed tray runs again.";

pub(crate) async fn install_host_update(
    events: std::sync::Arc<crate::daemon::EventBus>,
    plugin_manager: std::sync::Arc<Mutex<crate::plugins::PluginManager>>,
    confirm_after_restart: bool,
    update_plugins: Vec<String>,
) -> Result<(), String> {
    if crate::installer::stand_in::is_stand_in() {
        log::warn!("Self-update refused: {STAND_IN_REFUSAL}");
        events.send(crate::daemon::DaemonEvent::UpdateFailed {
            message: STAND_IN_REFUSAL.to_string(),
        });
        return Err(STAND_IN_REFUSAL.to_string());
    }
    let record = confirm_after_restart
        .then(|| write_restart_record(update_plugins))
        .flatten();
    record_update_progress(0);
    let update = newest_update_before_install().await;
    store_latest_update(update.clone());
    let Err(error) = platform::download_and_install(events.clone(), plugin_manager, update).await
    else {
        return Ok(());
    };
    log::error!("Self-update failed: {error:#}");
    platform::recover_interrupted_update();
    if let Some(record) = record {
        let _ = std::fs::remove_file(record);
    }
    let message = plain_update_failure(&error);
    events.send(crate::daemon::DaemonEvent::UpdateFailed {
        message: message.clone(),
    });
    Err(message)
}

async fn newest_update_before_install() -> Option<FoundUpdate> {
    let cached = latest_update();
    let refreshed = check_for_updates().await.map(|_| latest_update());
    version_to_install(refreshed, cached)
}

fn version_to_install<T: std::fmt::Debug>(
    refreshed: Result<Option<T>>,
    cached: Option<T>,
) -> Option<T> {
    match refreshed {
        Ok(latest) => latest,
        Err(error) => {
            log::warn!("Could not re-check the newest qol-tray release, installing the cached {cached:?}: {error:#}");
            cached
        }
    }
}

fn write_restart_record(update_plugins: Vec<String>) -> Option<PathBuf> {
    let record = crate::restart_record::RestartRecord {
        from_version: CURRENT_VERSION.to_string(),
        attempts: 0,
        update_plugins,
    };
    let written = crate::paths::shared_config_dir()
        .map_err(std::io::Error::other)
        .and_then(|dir| crate::restart_record::write(&dir, &record));
    match written {
        Ok(path) => Some(path),
        Err(error) => {
            log::warn!("Failed to write the restart record: {}", error);
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumedUpdate {
    pub from_version: String,
    pub queued_plugins: usize,
    pub dropped_plugins: usize,
}

pub fn resume_after_restart() -> Option<ResumedUpdate> {
    let dir = crate::paths::shared_config_dir().ok()?;
    let record = crate::restart_record::begin(&dir)?;
    let resume = crate::restart_record::resume_plan(&record, CURRENT_VERSION);
    let Some(from_version) = resume.updated_from else {
        crate::restart_record::finish(&dir);
        return None;
    };
    let _ = UPDATED_FROM.set(from_version.clone());
    let mut queued_plugins = 0;
    for id in &resume.update_plugins {
        match jobs::push(id, jobs::Operation::Update) {
            Ok(()) => queued_plugins += 1,
            Err(message) => log::warn!("Failed to resume the update of {id}: {message}"),
        }
    }
    if !resume.dropped_plugins.is_empty() {
        log::warn!(
            "Not resuming plugin updates after repeated restarts: {}",
            resume.dropped_plugins.join(", ")
        );
    }
    Some(ResumedUpdate {
        from_version,
        queued_plugins,
        dropped_plugins: resume.dropped_plugins.len(),
    })
}

pub fn finish_restart_resume() {
    if let Ok(dir) = crate::paths::shared_config_dir() {
        crate::restart_record::finish(&dir);
    }
}

pub(super) fn verify_host_update(
    path: &Path,
    expected_version: Option<&str>,
    target_expectation: fn(
        qol_artifact::ArtifactExpectation,
        &str,
    ) -> qol_artifact::ArtifactExpectation,
) -> Result<()> {
    verify_release_binary(
        path,
        qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
        expected_version,
        target_expectation,
    )
}

pub(super) fn verify_release_binary(
    path: &Path,
    binary: &str,
    expected_version: Option<&str>,
    target_expectation: fn(
        qol_artifact::ArtifactExpectation,
        &str,
    ) -> qol_artifact::ArtifactExpectation,
) -> Result<()> {
    let running = qol_conventions::artifact::current()
        .ok_or_else(|| anyhow::anyhow!("running build identity is unavailable"))?;
    let expectation = release_expectation(
        binary,
        &running.target,
        expected_version,
        target_expectation,
    )?;
    let inspected = qol_artifact::verify_path(path, &expectation)?;
    log::info!(
        "[artifact-identity] verified self-update binary path={} version={} slices={}",
        path.display(),
        expected_version.unwrap_or("<dev-override>"),
        inspected.slices.len()
    );
    Ok(())
}

fn release_expectation(
    binary: &str,
    running_target: &str,
    expected_version: Option<&str>,
    target_expectation: fn(
        qol_artifact::ArtifactExpectation,
        &str,
    ) -> qol_artifact::ArtifactExpectation,
) -> Result<qol_artifact::ArtifactExpectation> {
    let role = qol_conventions::artifact::tray_binary_role(binary)
        .ok_or_else(|| anyhow::anyhow!("{binary} is not a qol-tray release binary"))?;
    let mut expectation = qol_artifact::ArtifactExpectation::production(
        binary,
        qol_conventions::artifact::TRAY_PACKAGE_NAME,
        role,
    );
    expectation = target_expectation(expectation, running_target);
    if let Some(version) = expected_version {
        expectation = expectation.with_version(version);
    }
    Ok(expectation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_conventions::artifact::{
        BuildFlavor, BuildIdentity, BuildIntent, BuildProfile, BuildRole, CompilerFacts,
        SourceIdentity, SCHEMA_VERSION,
    };

    #[test]
    fn the_plugin_index_answers_only_when_it_names_a_newer_host() {
        let listed = |version: &str| Ok(Some(IndexedHostRelease::test_fixture(version, &[])));
        type Case = (
            &'static str,
            Result<Option<IndexedHostRelease>>,
            Result<Option<&'static str>, &'static str>,
        );
        let cases: Vec<Case> = vec![
            ("index newer", listed("3.2.0"), Ok(Some("3.2.0"))),
            ("index the running version", listed("3.1.0"), Ok(None)),
            ("index older", listed("3.0.9"), Ok(None)),
            (
                "index prerelease of the running version",
                listed("3.1.0-rc.1"),
                Ok(None),
            ),
            ("index has no entry for this os", Ok(None), Ok(None)),
            (
                "index unavailable",
                Err(anyhow::anyhow!("offline")),
                Err("The update could not be installed"),
            ),
            (
                "index signature refused",
                Err(anyhow::anyhow!("the index signature does not verify")),
                Err("The update could not be installed"),
            ),
        ];
        for (case, indexed, expected) in cases {
            let picked = newer_index_release(indexed, "3.1.0");
            let picked = picked
                .as_ref()
                .map(|release| release.as_ref().map(|release| release.version.as_str()))
                .map_err(String::as_str);
            assert_eq!(picked, expected, "case: {case}");
        }
    }

    #[test]
    fn a_found_update_names_its_version() {
        let update = FoundUpdate::PluginIndex(IndexedHostRelease::test_fixture("3.2.0", &[]));
        assert_eq!(update.version(), "3.2.0");
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
        let expectation = release_expectation(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            target,
            Some("3.41.0"),
            qol_artifact::ArtifactExpectation::with_exact_target,
        )
        .unwrap();
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
    fn installer_updates_are_checked_against_the_installer_identity() {
        let target = "x86_64-pc-windows-msvc";
        let expectation = release_expectation(
            qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
            target,
            Some("3.41.0"),
            qol_artifact::ArtifactExpectation::with_exact_target,
        )
        .unwrap();
        let mut installer = update_identity("3.41.0", target);
        installer.binary = qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME.to_string();
        installer.role = BuildRole::Installer;
        qol_artifact::verify_identity(&installer, &expectation).unwrap();

        let host = update_identity("3.41.0", target);
        assert!(qol_artifact::verify_identity(&host, &expectation).is_err());

        assert!(release_expectation(
            "qol-tray-unknown",
            target,
            None,
            qol_artifact::ArtifactExpectation::with_exact_target,
        )
        .is_err());
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
                version::is_newer_version(latest, current),
                expected,
                "latest={latest} current={current}"
            );
        }
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
    fn a_check_that_found_no_connection_retries_within_a_minute() {
        let rate_limited = github_status_message(403);
        let cases = [
            ("last check succeeded", None, CHECK_INTERVAL),
            ("no connection at boot", Some(NO_CONNECTION), OFFLINE_RETRY),
            ("rate limited", Some(rate_limited.as_str()), CHECK_INTERVAL),
            ("server error", Some("GitHub answered 500"), CHECK_INTERVAL),
        ];
        for (name, last_error, expected) in cases {
            assert_eq!(check_delay(last_error), expected, "case: {name}");
        }
    }

    #[test]
    fn only_a_running_check_holds_back_the_next_one() {
        let now = SystemTime::now();
        let mut state = UpdateState {
            last_attempt: Some(now),
            last_success: Some(now),
            last_error: None,
            checking: false,
            update_found: false,
        };
        assert_eq!(begin_check(&mut state, now), None);
        assert!(state.checking);

        state.update_found = true;
        assert_eq!(begin_check(&mut state, now), Some(true));
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
    fn install_version_prefers_the_refreshed_check() {
        let cases = [
            (
                "refresh found a newer release",
                Ok(Some("3.5.0")),
                Some("3.4.0"),
                Some("3.5.0"),
            ),
            (
                "refresh agrees with the cache",
                Ok(Some("3.4.0")),
                Some("3.4.0"),
                Some("3.4.0"),
            ),
            ("refresh found no update", Ok(None), Some("3.4.0"), None),
            (
                "refresh failed, cache kept",
                Err("offline"),
                Some("3.4.0"),
                Some("3.4.0"),
            ),
            ("refresh failed, nothing cached", Err("offline"), None, None),
            (
                "refresh found a release, nothing cached",
                Ok(Some("3.5.0")),
                None,
                Some("3.5.0"),
            ),
        ];
        for (name, refreshed, cached, expected) in cases {
            let refreshed = refreshed
                .map(|latest| latest.map(str::to_string))
                .map_err(|message| anyhow::anyhow!(message));
            assert_eq!(
                version_to_install(refreshed, cached.map(str::to_string)).as_deref(),
                expected,
                "case: {name}"
            );
        }
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
                "Plugin operation already in progress: qol-launcher",
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
