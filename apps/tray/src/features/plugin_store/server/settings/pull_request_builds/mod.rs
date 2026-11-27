use anyhow::{Context, Result};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use qol_pr_try::{Host, Places, RestartsTray, TrayBack, Try, REPO};
use qol_sync::Mutex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::types::AppState;
use super::http_json;
use super::update_handlers::{self, action_error, action_ok, CoreActionRequest};
use crate::features::plugin_store::github::Actions;
use crate::features::profile::pull_request_builds;
use crate::installer::trial::Trial;
use qol_conventions::pr_try::{
    DISMISS_ACTION as DISMISS, END_ACTION as END, TRY_ACTION as TRY,
    TRY_WITH_TRAY_ACTION as TRY_WITH_TRAY,
};

const BUSY: &str = "A pull request build is already loading";
const NO_NUMBER: &str = "Choose a pull request";
const NO_PLACES: &str = "Couldn't find where qol keeps its files";
const NO_START: &str = "Couldn't start getting the build";
const LIST_FAILED: &str = "Couldn't list the pull request builds";
const LIST_SLOW: &str = "Listing the pull request builds took too long";
const LIST_DEADLINE: Duration = Duration::from_secs(25);
const CHECK_POLL: Duration = Duration::from_millis(250);
const OFF: &str = "Trying pull request builds is turned off";
const SIGN_IN: &str = "Sign in to GitHub on the Profiles page to try pull request builds";
const LOG: &str = "[pull-request-builds]";
pub(in crate::features::plugin_store::server) const NEWER_BUILD_EVERY: Duration =
    Duration::from_secs(10 * 60);

static JOB: Mutex<Option<Job>> = Mutex::new(None);
static NEWER: Mutex<Option<Newer>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Newer {
    number: u64,
    tried: String,
    head: String,
}

impl Newer {
    fn of(&self, trial: &Trial) -> Option<String> {
        (self.number == trial.pull_request && self.tried == trial.head)
            .then(|| qol_conventions::pr_try::short_sha(&self.head).to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum Job {
    Getting { number: u64 },
    Ending,
    NeedsTray { number: u64 },
    Failed { message: String },
}

impl Job {
    fn busy(&self) -> bool {
        matches!(self, Self::Getting { .. } | Self::Ending)
    }
}

pub(super) fn current() -> Option<Job> {
    JOB.lock().clone()
}

pub(super) fn shown() -> bool {
    pull_request_builds::enabled()
}

pub(super) fn newer_build(trial: &Trial) -> Option<String> {
    NEWER.lock().as_ref().and_then(|newer| newer.of(trial))
}

pub(in crate::features::plugin_store::server) async fn check_newer_build(trial: Trial) {
    if !shown() {
        return;
    }
    let runtime = tokio::runtime::Handle::current();
    let number = trial.pull_request;
    let checked = tokio::task::spawn_blocking(move || {
        let source = signed_in(runtime).context(SIGN_IN)?;
        let head =
            qol_pr_try::newer_build(&source, qol_pr_try::host_build()?, number, &trial.head)?;
        Ok::<_, anyhow::Error>(head.map(|head| Newer {
            number,
            tried: trial.head,
            head,
        }))
    })
    .await;
    match checked {
        Ok(Ok(newer)) => {
            if let Some(newer) = &newer {
                log::info!("{LOG} #{number} has a newer build at {}", newer.head);
            }
            *NEWER.lock() = newer;
        }
        Ok(Err(error)) => {
            log::warn!("{LOG} checking #{number} for a newer build failed: {error:#}")
        }
        Err(error) => log::warn!("{LOG} checking #{number} for a newer build stopped: {error}"),
    }
}

pub(super) async fn get_list() -> Response {
    if !shown() {
        return (StatusCode::NOT_FOUND, OFF).into_response();
    }
    let runtime = tokio::runtime::Handle::current();
    let listing = http_json::blocking("pull request builds query", move || {
        let source = signed_in(runtime).ok_or_else(|| Box::new(unavailable(SIGN_IN)))?;
        let ready = qol_pr_try::host_build()
            .and_then(|build| qol_pr_try::ready(&source, build))
            .map_err(|error| {
                log::warn!("{LOG} listing failed: {error:#}");
                Box::new(unavailable(&format!("{LIST_FAILED}: {error:#}")))
            })?;
        let json = http_json::encode_json(&ready, LIST_FAILED)?;
        Ok(http_json::json_response(json))
    });
    tokio::time::timeout(LIST_DEADLINE, listing)
        .await
        .unwrap_or_else(|_| unavailable(LIST_SLOW))
}

fn signed_in(runtime: tokio::runtime::Handle) -> Option<Actions> {
    crate::credentials::github_bearer_token().map(|token| Actions::new(REPO, token, runtime))
}

pub(super) fn post_action(
    state: &AppState,
    action: &str,
    request: &CoreActionRequest,
) -> Option<Response> {
    if refused(action, pull_request_builds::enabled()) {
        return Some(action_error(StatusCode::NOT_FOUND, OFF));
    }
    let response = match action {
        TRY => start(
            state,
            Run::Try(Try {
                number: number(request),
                keep_tray: true,
            }),
        ),
        TRY_WITH_TRAY => start(
            state,
            Run::Try(Try {
                number: number(request),
                keep_tray: false,
            }),
        ),
        END => start(state, Run::Off),
        DISMISS => {
            let mut job = JOB.lock();
            if job.as_ref().is_some_and(|job| !job.busy()) {
                *job = None;
            }
            action_ok("Dismissed")
        }
        _ => return None,
    };
    Some(response)
}

fn refused(action: &str, enabled: bool) -> bool {
    !enabled && matches!(action, TRY | TRY_WITH_TRAY)
}

fn number(request: &CoreActionRequest) -> u64 {
    request
        .id
        .as_deref()
        .and_then(|id| id.trim_start_matches('#').parse().ok())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    Try(Try),
    Off,
}

impl Run {
    fn job(self) -> Job {
        match self {
            Self::Try(tried) => Job::Getting {
                number: tried.number,
            },
            Self::Off => Job::Ending,
        }
    }

    fn finished(self, result: &Result<()>) -> Option<Job> {
        let error = result.as_ref().err()?;
        if let Self::Try(tried) = self {
            if tried.keep_tray && error.downcast_ref::<RestartsTray>().is_some() {
                return Some(Job::NeedsTray {
                    number: tried.number,
                });
            }
        }
        Some(Job::Failed {
            message: format!("{error:#}"),
        })
    }
}

fn start(state: &AppState, run: Run) -> Response {
    if matches!(run, Run::Try(Try { number: 0, .. })) {
        return action_error(StatusCode::BAD_REQUEST, NO_NUMBER);
    }
    let mut job = JOB.lock();
    if job.as_ref().is_some_and(Job::busy) {
        return action_error(StatusCode::CONFLICT, BUSY);
    }
    let runtime = tokio::runtime::Handle::current();
    let source = match run {
        Run::Try(_) => match signed_in(runtime.clone()) {
            Some(source) => Some(source),
            None => return action_error(StatusCode::UNAUTHORIZED, SIGN_IN),
        },
        Run::Off => None,
    };
    let Some(places) = places() else {
        return action_error(StatusCode::INTERNAL_SERVER_ERROR, NO_PLACES);
    };
    let mut host = TrayHost {
        state: state.clone(),
        runtime,
    };
    let spawned = std::thread::Builder::new()
        .name("qol-pr-try".to_string())
        .spawn(move || {
            let result = match (run, source) {
                (Run::Try(tried), Some(source)) => qol_pr_try::host_build().and_then(|build| {
                    let number = tried.number;
                    let replaced =
                        crate::installer::trial::current().map(|trial| trial.pull_request);
                    match qol_pr_try::try_build(&source, &mut host, tried, build, &places)? {
                        qol_pr_try::Tried::Installed => Ok(()),
                        qol_pr_try::Tried::Nothing => Err(nothing_tried(number, replaced)),
                    }
                }),
                (Run::Try(_), None) => Err(anyhow::anyhow!(SIGN_IN)),
                (Run::Off, _) => qol_pr_try::turn_off(&mut host, &places)
                    .map(|put_back| host.catch_up(&put_back)),
            };
            if let Err(error) = &result {
                log::error!("{LOG} {run:?} failed: {error:#}");
            }
            *JOB.lock() = run.finished(&result);
        });
    if let Err(error) = spawned {
        log::warn!("{LOG} could not start {run:?}: {error}");
        return action_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{NO_START}: {error}"),
        );
    }
    *job = Some(run.job());
    action_ok("Started")
}

fn nothing_tried(number: u64, replaced: Option<u64>) -> anyhow::Error {
    match replaced {
        Some(other) if other == number => anyhow::anyhow!(
            "the new head of pull request #{number} changes no plugin installed here; the builds its earlier head swapped in were put back"
        ),
        Some(other) => anyhow::anyhow!(
            "pull request #{number} changes no plugin installed here; the builds of #{other} were put back and nothing else changed"
        ),
        None => anyhow::anyhow!(
            "pull request #{number} changes no plugin installed here, so nothing was swapped"
        ),
    }
}

fn places() -> Option<Places> {
    let config = crate::paths::shared_config_dir().ok()?;
    let data = crate::paths::base_data_dir().ok()?;
    let cache = qol_config::cache_dir()?;
    Some(Places::under(&config, &data, &cache))
}

struct TrayHost {
    state: AppState,
    runtime: tokio::runtime::Handle,
}

impl TrayHost {
    fn hand_over(&self, binary: &Path) -> anyhow::Error {
        log::info!("{LOG} restarting into {}", binary.display());
        let plugin_manager = self.state.plugin_manager.clone();
        let error = self
            .runtime
            .block_on(crate::relaunch::restart_into(binary, async move {
                plugin_manager.lock().stop_for_restart();
                Ok(())
            }));
        if let Err(restarted) = self
            .state
            .plugin_manager
            .lock()
            .start_after_failed_restart()
        {
            log::warn!("{LOG} plugins did not start again: {restarted:#}");
        }
        anyhow::anyhow!("starting {}: {error}", binary.display())
    }
}

impl Host for TrayHost {
    fn say(&mut self, line: &str) {
        log::info!("{LOG} {}", line.trim());
    }

    fn run_tray(&mut self, binary: &Path) -> Result<String> {
        Err(self.hand_over(binary))
    }

    fn put_back_tray(
        &mut self,
        try_binary: &Path,
        installed: &Path,
        settle: &mut dyn FnMut() -> Result<()>,
    ) -> Result<TrayBack> {
        settle()?;
        if !qol_pr_try::runs_binary(std::process::id(), try_binary) {
            return Ok(TrayBack {
                said: "the installed tray already runs".to_string(),
                started: false,
            });
        }
        Err(self.hand_over(installed))
    }

    fn running_install(&self, downloads: &Path) -> Result<Option<PathBuf>> {
        let current = qol_conventions::artifact::normalized_executable(std::env::current_exe()?);
        Ok(installed_unless_tried(current, downloads))
    }

    fn catch_up(&mut self, put_back: &[String]) {
        if put_back.is_empty() {
            return;
        }
        let check = super::super::update_check::claim(&self.state);
        let state = self.state.clone();
        let put_back = put_back.to_vec();
        self.runtime.spawn(async move {
            match check {
                Some(check) => check.await,
                None => {
                    while super::super::update_check::running(&state) {
                        tokio::time::sleep(CHECK_POLL).await;
                    }
                }
            }
            if crate::installer::trial::current().is_none() {
                update_handlers::update_put_back(&state, &put_back);
            }
        });
    }
}

fn installed_unless_tried(current: PathBuf, downloads: &Path) -> Option<PathBuf> {
    (!current.starts_with(downloads)).then_some(current)
}

fn unavailable(message: &str) -> Response {
    (StatusCode::BAD_GATEWAY, message.to_string()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_try_that_swapped_nothing_says_what_was_put_back() {
        let put_back = nothing_tried(641, Some(640)).to_string();
        assert!(
            put_back.contains("builds of #640 were put back"),
            "{put_back}"
        );
        let same = nothing_tried(641, Some(641)).to_string();
        assert!(
            same.contains("its earlier head swapped in were put back"),
            "{same}"
        );
        let alone = nothing_tried(641, None).to_string();
        assert!(alone.ends_with("so nothing was swapped"), "{alone}");
    }

    fn tried(number: u64, keep_tray: bool) -> Run {
        Run::Try(Try { number, keep_tray })
    }

    #[test]
    fn a_kept_tray_refusal_asks_and_other_failures_say_why() {
        let keep = tried(621, true);
        assert_eq!(keep.finished(&Ok(())), None);
        assert_eq!(
            keep.finished(&Err(RestartsTray::Carries(621).into())),
            Some(Job::NeedsTray { number: 621 })
        );
        assert_eq!(
            keep.finished(&Err(anyhow::anyhow!("offline").context("listing runs"))),
            Some(Job::Failed {
                message: "listing runs: offline".into()
            })
        );
        assert_eq!(
            tried(621, false).finished(&Err(RestartsTray::Carries(621).into())),
            Some(Job::Failed {
                message: RestartsTray::Carries(621).to_string()
            })
        );
        assert_eq!(
            Run::Off.finished(&Err(anyhow::anyhow!("no record"))),
            Some(Job::Failed {
                message: "no record".into()
            })
        );
    }

    #[test]
    fn runs_name_the_job_they_show() {
        assert_eq!(tried(7, true).job(), Job::Getting { number: 7 });
        assert_eq!(Run::Off.job(), Job::Ending);
    }

    #[test]
    fn a_request_names_a_positive_pull_request_number() {
        let request = |id: Option<&str>| CoreActionRequest {
            id: id.map(ToString::to_string),
            repo: None,
        };
        assert_eq!(number(&request(Some("621"))), 621);
        assert_eq!(number(&request(Some("#621"))), 621);
        assert_eq!(number(&request(Some("x"))), 0);
        assert_eq!(number(&request(None)), 0);
    }

    #[test]
    fn the_job_serializes_with_its_state_tag() {
        assert_eq!(
            serde_json::to_string(&Job::NeedsTray { number: 621 }).unwrap(),
            r#"{"state":"needs_tray","number":621}"#
        );
        assert_eq!(
            serde_json::to_string(&Job::Ending).unwrap(),
            r#"{"state":"ending"}"#
        );
    }

    #[test]
    fn the_builds_show_only_while_this_device_turned_them_on() {
        let root = tempfile::tempdir().unwrap();
        let _root = crate::paths::push_test_path_root(root.path());
        assert!(!shown());
        let core = crate::features::profile::ProfileScopeStore::from_active()
            .unwrap()
            .core_dir();
        qol_config::feature_flags::save(&core, &["updates.pull_request_builds".to_string()])
            .unwrap();
        assert!(!shown(), "the old profile flag no longer turns them on");
        pull_request_builds::set(true).unwrap();
        assert!(shown());
    }

    #[test]
    fn only_starting_a_try_needs_the_switch() {
        for action in [TRY, TRY_WITH_TRAY] {
            assert!(refused(action, false), "{action}");
            assert!(!refused(action, true), "{action}");
        }
        for action in [END, DISMISS, "check_updates"] {
            assert!(!refused(action, false), "{action}");
        }
    }

    #[tokio::test]
    async fn an_active_try_leaves_the_builds_hidden_while_the_switch_is_off() {
        let root = tempfile::tempdir().unwrap();
        let _root = crate::paths::push_test_path_root(root.path());
        let record = qol_conventions::pr_try::record_path(&crate::paths::base_data_dir().unwrap());
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        let tried = qol_conventions::pr_try::TryRecord {
            pull_request: Some(621),
            head_sha: Some("0123456789abcdef".into()),
            plugins: vec![qol_conventions::pr_try::SwappedPlugin {
                id: "qol-demo".into(),
                target: "/p".into(),
                backup: "/b".into(),
            }],
            ..Default::default()
        };
        std::fs::write(&record, serde_json::to_vec(&tried).unwrap()).unwrap();
        assert!(crate::installer::trial::current().is_some());
        assert!(!shown());
        assert_eq!(get_list().await.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_running_try_build_is_never_taken_for_the_install() {
        let downloads = Path::new("/home/u/.cache/qol-tray/pr-try");
        let installed = PathBuf::from("/home/u/.local/bin/qol-tray");
        assert_eq!(
            installed_unless_tried(installed.clone(), downloads),
            Some(installed)
        );
        assert_eq!(
            installed_unless_tried(downloads.join("412-abc/tray/qol-tray"), downloads),
            None
        );
    }
}
