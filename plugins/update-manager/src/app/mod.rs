use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use chrono::{Local, NaiveDateTime};
use qol_audio::devices::{self, Direction, Kind, State as DeviceState};
use qol_config::pages::{Go, Press, PressReply, Stack};
use qol_host_fixes::residency::HostResidency;
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use serde::Serialize;

use crate::cleanup;
use crate::pages;
use crate::platform::Refusal;
use crate::session::{Helper as HelperState, InUse, Journal, Rules, Session, Task};
use crate::updates::{self, Failure, Helper, Reason};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};
const HOST_EVERY: Duration = Duration::from_secs(60);
const UPDATES_EVERY: Duration = Duration::from_secs(15 * 60);
const RULES_EVERY: Duration = Duration::from_secs(5);
const SURVEY_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const SURVEY_RETRY: Duration = Duration::from_secs(10 * 60);
const SURVEY_CHECK: Duration = Duration::from_secs(60);
const RELEASE_WAIT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Ping,
    Kill,
    Pages,
    Press,
    Settings,
}

struct State {
    session: Session,
    want_updates: bool,
    want_host: bool,
    want_rules: bool,
    want_survey: bool,
    reading: bool,
    surveying: bool,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    journal: Option<PathBuf>,
    log: Option<PathBuf>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>, timeout: Duration) -> MutexGuard<'a, State> {
        self.wake
            .wait_timeout(state, timeout)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    fn save(&self, session: &Session) {
        let Some(path) = self.journal.as_ref() else {
            return;
        };
        if let Err(error) = session.journal.save(path) {
            log::warn!(
                "update-manager: {} could not be saved: {error}",
                path.display()
            );
        }
    }

    fn rules(&self, change: impl FnOnce(&mut Rules)) {
        change(&mut self.lock().session.rules);
    }
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

pub fn run() -> ExitCode {
    let dir = qol_config::data_subdir("plugins").map(|dir| dir.join(crate::PLUGIN_ID));
    let journal = dir.as_ref().map(|dir| dir.join("journal.json"));
    let log = dir.as_ref().map(|dir| dir.join("install.log"));
    let mut session = Session::new(
        journal.as_deref().map(Journal::load).unwrap_or_default(),
        !HostResidency::current().is_resident(),
    );
    session.install_log = log.as_ref().map(|log| log.display().to_string());
    let shared = Arc::new(Shared {
        state: Mutex::new(starting(session)),
        wake: Condvar::new(),
        journal,
        log,
    });
    if let Err(error) = spawn("refresh", &shared, refresh) {
        log::error!("update-manager: cannot start reading updates: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = spawn("survey", &shared, survey) {
        log::warn!("update-manager: cannot start measuring what updates no longer need: {error}");
        shared.lock().surveying = false;
        shared.wake.notify_all();
    }
    let (released, done) = mpsc::channel();
    let watcher = Arc::clone(&shared);
    let rules_thread = thread::Builder::new()
        .name("qol-update-manager-rules".into())
        .spawn(move || {
            rules(&watcher);
            let _ = released.send(());
        });
    if let Err(error) = rules_thread {
        log::error!("update-manager: cannot start its rules: {error}");
        return ExitCode::FAILURE;
    }
    let listened = core_daemon::run_stateful_request_listener(
        &DAEMON_CONFIG,
        Arc::clone(&shared),
        |shared, request| handle(shared, &request.action, &request.input),
    );
    shared.lock().stop = true;
    shared.wake.notify_all();
    if done.recv_timeout(RELEASE_WAIT).is_err() {
        log::warn!("update-manager: the rules did not end in time; exiting releases them");
    }
    core_daemon::cleanup(&DAEMON_CONFIG);
    match listened {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("update-manager: the daemon listener failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn starting(session: Session) -> State {
    State {
        session,
        want_updates: true,
        want_host: true,
        want_rules: true,
        want_survey: false,
        reading: false,
        surveying: true,
        stop: false,
    }
}

fn spawn(
    name: &str,
    shared: &Arc<Shared>,
    work: fn(&Shared),
) -> std::io::Result<thread::JoinHandle<()>> {
    let shared = Arc::clone(shared);
    thread::Builder::new()
        .name(format!("qol-update-manager-{name}"))
        .spawn(move || work(&shared))
}

fn classify(action: &str) -> Option<Action> {
    Some(match action {
        "ping" => Action::Ping,
        "kill" => Action::Kill,
        "pages" => Action::Pages,
        "press" => Action::Press,
        "settings" => Action::Settings,
        _ => return None,
    })
}

fn handle(shared: &Arc<Shared>, action: &str, input: &serde_json::Value) -> ReadResult<()> {
    let Some(action) = classify(action) else {
        return ReadResult::Error(format!("unknown action `{action}`"));
    };
    match action {
        Action::Ping | Action::Kill => ReadResult::Handled,
        Action::Pages => match open(input) {
            Ok(open) => payload(
                &pages::build(&shared.lock().session, now())
                    .pages
                    .only(&open),
            ),
            Err(problem) => ReadResult::Error(problem),
        },
        Action::Press => press(shared, input),
        Action::Settings => settings(),
    }
}

fn open(input: &serde_json::Value) -> Result<Vec<String>, String> {
    if input.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value::<Stack>(input.clone())
        .map(|stack| stack.open)
        .map_err(|error| format!("pages needs the open pages: {error}"))
}

fn payload<T: Serialize>(value: &T) -> ReadResult<()> {
    match serde_json::to_value(value) {
        Ok(data) => ReadResult::HandledWithData(data),
        Err(error) => ReadResult::Error(format!("failed to encode the answer: {error}")),
    }
}

fn press(shared: &Arc<Shared>, input: &serde_json::Value) -> ReadResult<()> {
    let mut press: Press = match serde_json::from_value(input.clone()) {
        Ok(press) => press,
        Err(error) => return ReadResult::Error(format!("press needs a page and a row: {error}")),
    };
    let at = now();
    let mut state = shared.lock();
    let outcome = pages::press(&mut state.session, &press.page, &press.row, at);
    match outcome.task {
        Some(Task::Rules) => {
            state.want_rules = true;
            shared.wake.notify_all();
        }
        Some(task) => start(shared, &mut state.session, task),
        None => {}
    }
    if let Some(Go::Open(page) | Go::Replace(page)) = &outcome.go {
        press.open.push(page.clone());
    }
    payload(&PressReply {
        pages: Some(pages::build(&state.session, at).pages.only(&press.open)),
        go: outcome.go,
        notice: outcome.notice,
    })
}

fn start(shared: &Arc<Shared>, session: &mut Session, task: Task) {
    let worker = Arc::clone(shared);
    let queued = task.clone();
    let spawned = thread::Builder::new()
        .name("qol-update-manager-task".into())
        .spawn(move || work(&worker, queued));
    if let Err(error) = spawned {
        let reason = format!("qol could not start the change: {error}");
        match task {
            Task::Install(_) => session.installed(Err(Failure {
                done: 0,
                reason: Reason::Stopped(reason),
            })),
            other => session.done(&other, Err(Refusal::Failed(reason))),
        }
    }
}

fn work(shared: &Shared, task: Task) {
    let result = match &task {
        Task::Install(list) => {
            let result = match &shared.log {
                Some(log) => panic::catch_unwind(AssertUnwindSafe(|| {
                    updates::install(list, log, &mut |done| {
                        shared.lock().session.progress(done);
                    })
                }))
                .unwrap_or_else(|_| {
                    Err(Failure {
                        done: 0,
                        reason: Reason::Stopped(
                            "qol stopped unexpectedly during the install".into(),
                        ),
                    })
                }),
                None => Err(Failure {
                    done: 0,
                    reason: Reason::Stopped(
                        "the install log could not be written: qol's data folder was not found"
                            .into(),
                    ),
                }),
            };
            let mut state = shared.lock();
            state.session.installed(result);
            shared.save(&state.session);
            state.want_updates = true;
            state.want_host = true;
            state.want_survey = true;
            shared.wake.notify_all();
            return;
        }
        Task::Hours {
            hours,
            startup_delay,
            restart,
            ..
        } => updates::set_hours(*hours, *startup_delay, *restart),
        Task::Auto { key, on, .. } => updates::set_auto(*key, *on),
        Task::Restart(at) => updates::schedule_restart(*at),
        Task::RestartNow => updates::restart_now(),
        Task::Skips {
            add,
            remove,
            system,
        } => updates::set_skips(add, remove, *system),
        Task::Remove { kinds, consent } => cleanup::sweep(kinds, *consent),
        Task::Open(path) => {
            if let Err(error) = qol_apps::desktop_integration::open_with_default_app_checked(path) {
                log::warn!("update-manager: {path} could not be opened: {error}");
            }
            return;
        }
        Task::Rules => return,
    };
    finish(shared, &task, result);
}

fn finish(shared: &Shared, task: &Task, result: Result<(), Refusal>) {
    let mut state = shared.lock();
    state.session.done(task, result);
    shared.save(&state.session);
    state.want_host = true;
    state.want_survey = true;
    shared.wake.notify_all();
}

fn refresh(shared: &Shared) {
    let mut next_updates = Instant::now();
    let mut next_host = Instant::now();
    loop {
        let (read_updates, read_host) = {
            let mut state = shared.lock();
            loop {
                if state.stop {
                    return;
                }
                let at = Instant::now();
                let idle = may_read_updates(&state);
                let updates_due = idle && (state.want_updates || at >= next_updates);
                let host_due = state.want_host || at >= next_host;
                if updates_due || host_due {
                    if updates_due {
                        state.want_updates = false;
                        state.reading = true;
                    }
                    state.want_host = false;
                    break (updates_due, host_due);
                }
                let until = if idle {
                    next_host.min(next_updates)
                } else {
                    next_host
                };
                state = shared.wait(state, until.saturating_duration_since(at));
            }
        };
        if read_host {
            let host = updates::read_host();
            let boot = updates::boot_id();
            next_host = Instant::now() + HOST_EVERY;
            let mut state = shared.lock();
            state.session.apply_host(host);
            if state.session.journal.start_boot(boot) {
                shared.save(&state.session);
            }
        }
        if read_updates {
            let list = updates::read_updates();
            next_updates = Instant::now() + UPDATES_EVERY;
            let mut state = shared.lock();
            state.reading = false;
            shared.wake.notify_all();
            if state.session.job.is_none() {
                state.session.apply_updates(list, now());
            } else {
                state.want_updates = true;
            }
        }
    }
}

fn survey(shared: &Shared) {
    let mut next = Instant::now();
    let mut seen: Option<Vec<Option<SystemTime>>> = None;
    loop {
        {
            let mut state = shared.lock();
            if state.stop {
                state.surveying = false;
                return;
            }
            while !state.surveying {
                if state.stop {
                    return;
                }
                let busy = state.reading || state.session.job.is_some();
                let changed = seen.as_ref().is_some_and(|seen| *seen != cleanup::stamp());
                let left = next.saturating_duration_since(Instant::now());
                match survey_wait(state.want_survey, busy, changed, left) {
                    None => {
                        state.want_survey = false;
                        state.surveying = true;
                    }
                    Some(wait) => state = shared.wait(state, wait),
                }
            }
        }
        let found = cleanup::survey(now());
        seen = Some(found.stamp.clone());
        next = Instant::now()
            + if found.failed() {
                SURVEY_RETRY
            } else {
                SURVEY_EVERY
            };
        let mut state = shared.lock();
        store(&mut state, found);
        shared.wake.notify_all();
    }
}

fn store(state: &mut State, found: cleanup::Survey) {
    if state.session.job.is_none() {
        state.session.survey = Some(found);
    }
    state.surveying = false;
}

fn may_read_updates(state: &State) -> bool {
    state.session.job.is_none() && !state.surveying
}

fn survey_wait(want: bool, busy: bool, changed: bool, left: Duration) -> Option<Duration> {
    if busy {
        return Some(SURVEY_RETRY);
    }
    if want || changed || left.is_zero() {
        return None;
    }
    Some(left.min(SURVEY_CHECK))
}

fn rules(shared: &Shared) {
    let mut helper: Option<Helper> = None;
    let mut next = Instant::now();
    loop {
        {
            let mut state = shared.lock();
            while !state.stop && !state.want_rules && Instant::now() < next {
                let wait = next.saturating_duration_since(Instant::now());
                state = shared.wait(state, wait);
            }
            if state.stop {
                break;
            }
            state.want_rules = false;
        }
        next = Instant::now() + RULES_EVERY;
        let playing = in_use();
        let (want_helper, want_hold, holding) = {
            let mut state = shared.lock();
            state.session.in_use = playing;
            (
                state.session.want_helper(),
                state.session.want_hold(),
                state.session.rules.holding,
            )
        };
        if !want_helper {
            if helper.take().is_some() {
                shared.rules(|rules| {
                    rules.helper = HelperState::Off;
                    rules.holding = None;
                });
            }
            continue;
        }
        if helper.is_none() {
            shared.rules(|rules| rules.helper = HelperState::Starting);
            match Helper::start() {
                Ok(started) => {
                    helper = Some(started);
                    shared.rules(|rules| {
                        rules.helper = HelperState::Ready;
                        rules.holding = None;
                    });
                }
                Err(refusal) => {
                    shared.rules(|rules| rules.helper = HelperState::Failed(refusal));
                    continue;
                }
            }
        }
        let Some(active) = helper.as_mut() else {
            continue;
        };
        let result = match (want_hold, holding) {
            (true, None) => active.hold().map(Some),
            (false, Some(_)) => active.release().map(|()| None),
            _ => continue,
        };
        match result {
            Ok(holding) => shared.rules(|rules| rules.holding = holding),
            Err(refusal) => {
                helper = None;
                shared.rules(|rules| {
                    rules.helper = HelperState::Failed(refusal);
                    rules.holding = None;
                });
            }
        }
    }
    drop(helper);
}

fn in_use() -> Option<InUse> {
    let playing = devices::list(Direction::Output)
        .ok()?
        .into_iter()
        .find(|device| device.state == DeviceState::Running && device.monitor_of_sink.is_none())?;
    let prefix = format!("{}:", playing.name);
    let known = devices::list_outputs().ok().and_then(|outputs| {
        outputs
            .into_iter()
            .find(|output| output.identity.as_str().starts_with(&prefix))
    });
    Some(match known {
        Some(output) => InUse {
            label: output.label,
            bluetooth: output.kind == Kind::Bluetooth,
        },
        None => InUse {
            label: playing.description,
            bluetooth: false,
        },
    })
}

fn settings() -> ReadResult<()> {
    match qol_apps::desktop_integration::open_plugin_settings_via_tray(crate::PLUGIN_ID) {
        Ok(()) => ReadResult::Handled,
        Err(error) => ReadResult::Error(format!(
            "failed to open the Update Manager settings: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_plugin_api::manifest::PluginManifest;

    fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State {
                session: Session::new(Journal::default(), false),
                want_updates: false,
                want_host: false,
                want_rules: false,
                want_survey: false,
                reading: false,
                surveying: false,
                stop: false,
            }),
            wake: Condvar::new(),
            journal: None,
            log: None,
        })
    }

    #[test]
    fn an_install_without_a_data_folder_stops_and_says_why() {
        let shared = shared();
        shared.lock().session.job = Some(crate::session::Job {
            updates: Vec::new(),
            done: 0,
            started: false,
        });
        work(&shared, Task::Install(Vec::new()));
        let state = shared.lock();
        let failed = state.session.failed.as_ref().expect("failed");
        assert_eq!(
            failed.failure.reason,
            Reason::Stopped(
                "the install log could not be written: qol's data folder was not found".into()
            )
        );
        assert!(state.want_survey, "an install measures leftovers again");
    }

    #[test]
    fn the_first_survey_runs_before_the_first_read_of_updates() {
        let state = starting(Session::new(Journal::default(), false));
        assert!(
            state.surveying,
            "the survey thread starts holding the first survey"
        );
        assert!(state.want_updates && !state.want_survey && !state.reading);
        assert!(
            !may_read_updates(&state),
            "updates are read only once the first survey is stored"
        );
        let shared = shared();
        assert!(may_read_updates(&shared.lock()));
        shared.lock().surveying = true;
        assert!(!may_read_updates(&shared.lock()), "never while measuring");
    }

    #[test]
    fn a_survey_an_install_overlaps_is_dropped() {
        let shared = shared();
        let survey = || crate::cleanup::fake::survey(Vec::new());
        {
            let mut state = shared.lock();
            state.surveying = true;
            state.session.job = Some(crate::session::Job {
                updates: Vec::new(),
                done: 0,
                started: false,
            });
            store(&mut state, survey());
            assert!(
                state.session.survey.is_none(),
                "the install changed what is left"
            );
            assert!(!state.surveying);
            state.session.job = None;
            store(&mut state, survey());
            assert!(state.session.survey.is_some());
        }
    }

    #[test]
    fn the_survey_waits_for_reads_and_installs_and_wakes_for_package_changes() {
        let hour = Duration::from_secs(3600);
        assert_eq!(survey_wait(false, false, false, hour), Some(SURVEY_CHECK));
        assert_eq!(survey_wait(false, false, true, hour), None, "dpkg changed");
        assert_eq!(survey_wait(true, false, false, hour), None, "asked for");
        assert_eq!(
            survey_wait(false, false, false, Duration::ZERO),
            None,
            "due"
        );
        assert_eq!(
            survey_wait(false, false, false, Duration::from_secs(5)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            survey_wait(true, true, true, Duration::ZERO),
            Some(SURVEY_RETRY),
            "never while updates are read or installed"
        );
    }

    #[test]
    fn a_finished_switch_change_measures_leftovers_again() {
        let shared = shared();
        let task = Task::Auto {
            key: crate::updates::AutoKey::Switch(&crate::cleanup::fake::MINT_KERNELS),
            on: true,
        };
        finish(&shared, &task, Ok(()));
        let state = shared.lock();
        assert!(state.want_survey);
        assert!(state.want_host);
    }

    #[test]
    fn ping_kill_and_pages_answer_without_a_backend() {
        let shared = shared();
        assert!(matches!(
            handle(&shared, "ping", &serde_json::Value::Null),
            ReadResult::Handled
        ));
        assert!(matches!(
            handle(&shared, "kill", &serde_json::Value::Null),
            ReadResult::Handled
        ));
        let ReadResult::HandledWithData(data) = handle(&shared, "pages", &serde_json::Value::Null)
        else {
            panic!("pages must answer with data");
        };
        assert_eq!(data["root"], "main");
    }

    #[test]
    fn pages_answer_only_the_first_page_and_the_open_ones() {
        let shared = shared();
        let ReadResult::HandledWithData(data) = handle(
            &shared,
            "pages",
            &serde_json::json!({ "open": ["main", "last", "nowhere"] }),
        ) else {
            panic!("pages must answer with data");
        };
        let pages: qol_config::pages::Pages =
            serde_json::from_value(data).expect("pages on the wire");
        assert_eq!(pages.pages.keys().collect::<Vec<_>>(), ["main", "last"]);
        assert!(matches!(
            handle(&shared, "pages", &serde_json::json!({ "open": "main" })),
            ReadResult::Error(_)
        ));
    }

    #[test]
    fn a_press_answers_with_the_pages_and_a_notice_for_unknown_rows() {
        let shared = shared();
        let ReadResult::HandledWithData(data) = handle(
            &shared,
            "press",
            &serde_json::json!({ "page": "main", "row": "nowhere" }),
        ) else {
            panic!("press must answer with data");
        };
        let reply: PressReply = serde_json::from_value(data).expect("a press reply");
        assert!(reply.pages.is_some());
        assert!(reply.notice.is_some());
        assert!(matches!(
            handle(&shared, "press", &serde_json::json!({ "row": "x" })),
            ReadResult::Error(_)
        ));
    }

    #[test]
    fn unknown_actions_are_errors() {
        assert!(matches!(
            handle(&shared(), "upgrade", &serde_json::Value::Null),
            ReadResult::Error(_)
        ));
    }

    #[test]
    fn every_declared_action_and_query_maps_to_a_branch() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml must load");
        for id in manifest.executable_action_ids() {
            assert!(
                classify(&id).is_some(),
                "plugin.toml action `{id}` has no daemon branch"
            );
        }
        let runtime = qol_config::contract::parse_runtime_spec("qol-runtime.toml")
            .expect("qol-runtime.toml must load");
        for name in runtime.queries.keys() {
            assert!(
                classify(name).is_some(),
                "qol-runtime.toml query `{name}` has no daemon branch"
            );
        }
        let mapped = manifest.executable_action_ids();
        for name in runtime.actions.keys() {
            assert!(
                classify(name).is_some(),
                "qol-runtime.toml action `{name}` has no daemon branch"
            );
            assert!(
                mapped.iter().any(|id| id == name),
                "qol-runtime.toml action `{name}` has no plugin.toml action, so the tray refuses it"
            );
        }
    }
}
