use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use qol_sync::Mutex;
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::http_json;
use super::update_handlers::{action_error, action_ok, CoreActionRequest};
use crate::features::profile::feature_flag_enabled;
use qol_config::feature_flags::PULL_REQUEST_BUILDS;
use qol_conventions::pr_try::{
    CARRIES_TRAY_EXIT_CODE, DISMISS_ACTION as DISMISS, END_ACTION as END, KEEP_TRAY_FLAG,
    TRY_ACTION as TRY, TRY_WITH_TRAY_ACTION as TRY_WITH_TRAY,
};

const CLI: &str = "qol";
const BUSY: &str = "A pull request build is already loading";
const NO_NUMBER: &str = "Choose a pull request";
const NO_CLI: &str = "The qol command isn't installed";
const NO_START: &str = "Couldn't start qol";
const LIST_FAILED: &str = "Couldn't list the pull request builds";
const LIST_SLOW: &str = "Listing the pull request builds took too long";
const OFF: &str = "Trying pull request builds is turned off";
const LIST_DEADLINE: Duration = Duration::from_secs(25);
const LIST_POLL: Duration = Duration::from_millis(100);
const LOGIN_TOOL_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

static JOB: Mutex<Option<Job>> = Mutex::new(None);

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

pub(super) async fn get_list() -> Response {
    if !feature_flag_enabled(&PULL_REQUEST_BUILDS) {
        return (StatusCode::NOT_FOUND, OFF).into_response();
    }
    http_json::blocking("pull request builds query", || {
        let mut command = cli().ok_or_else(|| Box::new(unavailable(NO_CLI.to_string())))?;
        command.args(["pr", "list", "--json"]);
        let listed = output_within(command, LIST_DEADLINE)
            .map_err(|error| Box::new(unavailable(format!("{LIST_FAILED}: {error}"))))?
            .ok_or_else(|| Box::new(unavailable(LIST_SLOW.to_string())))?;
        if !listed.status.success() {
            return Err(Box::new(unavailable(
                last_line(&listed.stderr).unwrap_or_else(|| LIST_FAILED.to_string()),
            )));
        }
        if serde_json::from_slice::<serde_json::Value>(&listed.stdout).is_err() {
            return Err(Box::new(unavailable(LIST_FAILED.to_string())));
        }
        Ok(http_json::json_response(listed.stdout))
    })
    .await
}

struct Listed {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: String,
}

fn output_within(mut command: Command, deadline: Duration) -> std::io::Result<Option<Listed>> {
    let mut stdout = tempfile::tempfile()?;
    let stderr = tempfile::tempfile()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?)
        .spawn()?;
    let end = Instant::now() + deadline;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(LIST_POLL);
    };
    let mut bytes = Vec::new();
    stdout.seek(SeekFrom::Start(0))?;
    stdout.read_to_end(&mut bytes)?;
    Ok(Some(Listed {
        status,
        stdout: bytes,
        stderr: read_back(stderr),
    }))
}

pub(super) fn post_action(action: &str, request: &CoreActionRequest) -> Option<Response> {
    if !feature_flag_enabled(&PULL_REQUEST_BUILDS) {
        return None;
    }
    let response = match action {
        TRY => start(Run::Try {
            number: number(request),
            keep_tray: true,
        }),
        TRY_WITH_TRAY => start(Run::Try {
            number: number(request),
            keep_tray: false,
        }),
        END => start(Run::Off),
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

fn number(request: &CoreActionRequest) -> u64 {
    request
        .id
        .as_deref()
        .and_then(|id| id.trim_start_matches('#').parse().ok())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    Try { number: u64, keep_tray: bool },
    Off,
}

impl Run {
    fn args(self) -> Vec<String> {
        match self {
            Self::Try { number, keep_tray } => {
                let mut args = vec!["pr".to_string(), "try".to_string(), number.to_string()];
                if keep_tray {
                    args.push(KEEP_TRAY_FLAG.to_string());
                }
                args
            }
            Self::Off => vec!["pr".to_string(), "off".to_string()],
        }
    }

    fn job(self) -> Job {
        match self {
            Self::Try { number, .. } => Job::Getting { number },
            Self::Off => Job::Ending,
        }
    }

    fn finished(self, status: ExitStatus, stderr: &str) -> Option<Job> {
        if status.success() {
            return None;
        }
        if let Self::Try {
            number,
            keep_tray: true,
        } = self
        {
            if status.code() == Some(CARRIES_TRAY_EXIT_CODE) {
                return Some(Job::NeedsTray { number });
            }
        }
        let command = format!("qol {}", self.args().join(" "));
        let message = last_line(stderr).unwrap_or_else(|| format!("{command} failed"));
        Some(Job::Failed { message })
    }
}

fn start(run: Run) -> Response {
    if matches!(run, Run::Try { number: 0, .. }) {
        return action_error(StatusCode::BAD_REQUEST, NO_NUMBER);
    }
    let mut job = JOB.lock();
    if job.as_ref().is_some_and(Job::busy) {
        return action_error(StatusCode::CONFLICT, BUSY);
    }
    let Some(mut command) = cli() else {
        return action_error(StatusCode::SERVICE_UNAVAILABLE, NO_CLI);
    };
    let spawned = tempfile::tempfile().and_then(|stderr| {
        qol_process::isolate_owned_session(&mut command)?;
        let child = command
            .args(run.args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr.try_clone()?)
            .spawn()?;
        Ok((child, stderr))
    });
    let (mut child, stderr) = match spawned {
        Ok(spawned) => spawned,
        Err(error) => {
            log::warn!(
                "[pull-request-builds] could not start qol {:?}: {error}",
                run.args()
            );
            return action_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{NO_START}: {error}"),
            );
        }
    };
    *job = Some(run.job());
    drop(job);
    std::thread::spawn(move || {
        let finished = match child.wait() {
            Ok(status) => run.finished(status, &read_back(stderr)),
            Err(error) => Some(Job::Failed {
                message: format!("qol stopped answering: {error}"),
            }),
        };
        *JOB.lock() = finished;
    });
    action_ok("Started")
}

fn read_back(mut file: File) -> String {
    let mut text = String::new();
    if file.seek(SeekFrom::Start(0)).is_ok() {
        let _ = file.read_to_string(&mut text);
    }
    text
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(ToString::to_string)
}

fn cli() -> Option<Command> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let cargo_bin = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            crate::paths::user_home_dir()
                .ok()
                .map(|home| home.join(".cargo"))
        })
        .map(|cargo| cargo.join("bin"));
    let cli = find_cli(&path, cargo_bin.as_deref())?;
    let mut command = Command::new(&cli);
    if let Some(search) = child_path(&path, cli.parent(), cargo_bin.as_deref()) {
        command.env("PATH", search);
    }
    qol_process::hide_console_window(&mut command);
    Some(command)
}

fn child_path(path: &OsStr, cli_dir: Option<&Path>, cargo_bin: Option<&Path>) -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    let extra = cli_dir
        .into_iter()
        .chain(cargo_bin)
        .map(Path::to_path_buf)
        .chain(LOGIN_TOOL_DIRS.iter().map(PathBuf::from));
    for dir in extra {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).ok()
}

fn find_cli(path: &OsStr, cargo_bin: Option<&Path>) -> Option<PathBuf> {
    let name = format!("{CLI}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path)
        .chain(cargo_bin.map(Path::to_path_buf))
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
}

fn unavailable(message: String) -> Response {
    (StatusCode::BAD_GATEWAY, message).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn exited(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[cfg(unix)]
    #[test]
    fn a_kept_tray_refusal_asks_and_other_failures_say_why() {
        let keep = Run::Try {
            number: 621,
            keep_tray: true,
        };
        assert_eq!(keep.finished(exited(0), ""), None);
        assert_eq!(
            keep.finished(exited(CARRIES_TRAY_EXIT_CODE), "replaces qol-tray"),
            Some(Job::NeedsTray { number: 621 })
        );
        assert_eq!(
            keep.finished(exited(1), "downloading\ngh run failed: offline\n\n"),
            Some(Job::Failed {
                message: "gh run failed: offline".into()
            })
        );
        let replace = Run::Try {
            number: 621,
            keep_tray: false,
        };
        assert_eq!(
            replace.finished(exited(CARRIES_TRAY_EXIT_CODE), ""),
            Some(Job::Failed {
                message: "qol pr try 621 failed".into()
            })
        );
        assert_eq!(
            Run::Off.finished(exited(1), ""),
            Some(Job::Failed {
                message: "qol pr off failed".into()
            })
        );
    }

    #[test]
    fn runs_name_the_cli_arguments_and_the_job_they_show() {
        let keep = Run::Try {
            number: 7,
            keep_tray: true,
        };
        assert_eq!(keep.args(), ["pr", "try", "7", "--keep-tray"]);
        assert_eq!(keep.job(), Job::Getting { number: 7 });
        assert_eq!(Run::Off.args(), ["pr", "off"]);
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
        assert_eq!(
            start(Run::Try {
                number: 0,
                keep_tray: true
            })
            .status(),
            StatusCode::BAD_REQUEST
        );
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
    fn the_routes_answer_only_while_the_profile_flag_is_on() {
        let root = tempfile::tempdir().unwrap();
        let _root = crate::paths::push_test_path_root(root.path());
        let dismiss = CoreActionRequest::default();
        assert!(!feature_flag_enabled(&PULL_REQUEST_BUILDS));
        assert!(post_action(DISMISS, &dismiss).is_none());
        let core = crate::features::profile::ProfileScopeStore::from_active()
            .unwrap()
            .core_dir();
        qol_config::feature_flags::save(&core, &[PULL_REQUEST_BUILDS.id.to_string()]).unwrap();
        assert!(feature_flag_enabled(&PULL_REQUEST_BUILDS));
        assert_eq!(
            post_action(DISMISS, &dismiss).map(|response| response.status()),
            Some(StatusCode::OK)
        );
        assert!(post_action("unknown", &dismiss).is_none());
    }

    #[test]
    fn the_child_path_adds_where_qol_and_its_tools_live_once() {
        let path = std::env::join_paths(["/usr/bin", "/home/me/.cargo/bin"]).unwrap();
        let joined = child_path(
            &path,
            Some(Path::new("/opt/qol/bin")),
            Some(Path::new("/home/me/.cargo/bin")),
        )
        .unwrap();
        let dirs: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        let mut expected: Vec<PathBuf> = ["/usr/bin", "/home/me/.cargo/bin", "/opt/qol/bin"]
            .iter()
            .map(PathBuf::from)
            .collect();
        expected.extend(LOGIN_TOOL_DIRS.iter().map(PathBuf::from));
        assert_eq!(dirs, expected);
    }

    #[cfg(unix)]
    #[test]
    fn a_listing_past_its_deadline_is_killed() {
        let mut slow = Command::new("sleep");
        slow.arg("5");
        assert!(output_within(slow, Duration::from_millis(200))
            .unwrap()
            .is_none());
        let mut quick = Command::new("echo");
        quick.arg("[]");
        let listed = output_within(quick, Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(listed.status.success());
        assert_eq!(listed.stdout, b"[]\n");
    }

    #[test]
    fn the_cli_is_found_on_path_then_in_cargo_bin() {
        let root = tempfile::tempdir().unwrap();
        let on_path = root.path().join("path");
        let cargo_bin = root.path().join("cargo-bin");
        std::fs::create_dir_all(&on_path).unwrap();
        std::fs::create_dir_all(&cargo_bin).unwrap();
        let name = format!("{CLI}{}", std::env::consts::EXE_SUFFIX);
        let path = std::env::join_paths([&on_path]).unwrap();
        assert_eq!(find_cli(&path, Some(&cargo_bin)), None);
        std::fs::write(cargo_bin.join(&name), "").unwrap();
        assert_eq!(
            find_cli(&path, Some(&cargo_bin)),
            Some(cargo_bin.join(&name))
        );
        std::fs::write(on_path.join(&name), "").unwrap();
        assert_eq!(find_cli(&path, Some(&cargo_bin)), Some(on_path.join(&name)));
    }
}
