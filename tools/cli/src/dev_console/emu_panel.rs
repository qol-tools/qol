use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::commands::emu::{emu_dir, newest_run_detail, ImageCandidate, RunDetail};
use qol_dev_env::{
    CleanupState, EnvironmentSnapshot, Inventory, ReportKind, ReportStatus, ResolutionState,
    RunConcern, RunReport, RunSummary,
};
use qol_dev_orchestrator::{FlowStart, ImageImportStart, RunHandle, WaitState};

use super::render_util::{
    accent, caret, cursor_window_start, list_capacity, list_window, now_unix_ms, relative_age,
    spaced_height, view_content, NavigationOverflow,
};
use super::{
    copy_highlight, draw_run_log, frame_accent, spawn_forwarders, Dash, LogPane, LogRing, View,
    ITEM_GAP,
};

const SANDBOX_STOP_GRACE: Duration = Duration::from_secs(15);
const WORKER_TERMINATION_GRACE: Duration = Duration::from_secs(3);

pub(super) enum EmuState {
    Probing,
    Done(Inventory),
    Failed(String),
}

pub(super) struct EmuDetail {
    pub(super) id: String,
    pub(super) info: Vec<Line<'static>>,
    pub(super) warnings: Vec<Line<'static>>,
    pub(super) replay: Option<LogPane>,
}

#[derive(Clone)]
pub(super) enum SandboxLaunch {
    Environment { batch_id: String },
    Flow { batch_id: String },
    ImageImport { run_id: String },
    Candidate,
}

pub(super) struct ActiveSandboxRun {
    pub(super) pane: LogPane,
    launch: SandboxLaunch,
    report_path: Option<PathBuf>,
    worker_log_path: Option<PathBuf>,
    typed_run: Option<TypedSandboxRun>,
}

struct TypedSandboxRun {
    handle: RunHandle,
    last_observation: String,
}

impl ActiveSandboxRun {
    pub(super) fn environment(pane: LogPane, batch_id: impl Into<String>) -> Self {
        Self {
            pane,
            launch: SandboxLaunch::Environment {
                batch_id: batch_id.into(),
            },
            report_path: None,
            worker_log_path: None,
            typed_run: None,
        }
    }

    pub(super) fn candidate(pane: LogPane) -> Self {
        Self {
            pane,
            launch: SandboxLaunch::Candidate,
            report_path: None,
            worker_log_path: None,
            typed_run: None,
        }
    }

    fn flow(handle: RunHandle) -> Self {
        let batch_id = handle.ticket().run_id.clone();
        Self::typed(
            handle,
            SandboxLaunch::Flow { batch_id },
            "typed flow worker",
        )
    }

    fn image_import(handle: RunHandle) -> Self {
        let run_id = handle.ticket().run_id.clone();
        Self::typed(
            handle,
            SandboxLaunch::ImageImport { run_id },
            "typed image import worker",
        )
    }

    fn typed(handle: RunHandle, launch: SandboxLaunch, worker: &str) -> Self {
        let report_path = handle.ticket().report_path.clone();
        let worker_log_path = handle.ticket().worker_log_path();
        let mut pane = LogPane::new();
        match &worker_log_path {
            Ok(path) => pane.push(emu_run_line("worker", &path.display().to_string())),
            Err(error) => pane.push(emu_run_line(
                "error",
                &format!("worker log path unavailable: {error:#}"),
            )),
        }
        pane.push(emu_run_line("report", &report_path.display().to_string()));
        pane.push(emu_run_line("start", worker));
        Self {
            pane,
            launch,
            report_path: Some(report_path),
            worker_log_path: worker_log_path.ok(),
            typed_run: Some(TypedSandboxRun {
                handle,
                last_observation: "starting".to_string(),
            }),
        }
    }

    fn failed(launch: SandboxLaunch, error: &anyhow::Error) -> Self {
        let mut pane = LogPane::new();
        pane.push(emu_run_line("error", &format!("{error:#}")));
        Self {
            pane,
            launch,
            report_path: None,
            worker_log_path: None,
            typed_run: None,
        }
    }

    pub(super) fn report_path(&self) -> Option<&Path> {
        self.report_path.as_deref()
    }

    fn worker_log_path(&self) -> Option<&Path> {
        self.worker_log_path.as_deref()
    }

    fn with_report_path(mut self, report_path: PathBuf) -> Self {
        self.report_path = Some(report_path);
        self
    }

    fn is_live(&self) -> bool {
        self.typed_run.is_some() || self.pane.is_live()
    }

    fn request_cancellation(&self, run_id: &str) -> anyhow::Result<PathBuf> {
        let Some(run) = self.typed_run.as_ref() else {
            return qol_dev_env::request_cancellation(run_id);
        };
        if run.handle.ticket().run_id != run_id {
            anyhow::bail!("typed worker handle does not match its sandbox run identity");
        }
        run.handle.cancel()
    }

    fn poll_finished(&mut self) -> bool {
        let state = match self.typed_run.as_mut() {
            Some(run) => run.handle.poll(),
            None => return self.pane.poll_finished(keep_emu_line),
        };
        self.apply_typed_run_state(state)
    }

    fn wait_for_exit_until(&mut self, deadline: Instant) -> bool {
        let state = match self.typed_run.as_mut() {
            Some(run) => run
                .handle
                .wait_timeout(deadline.saturating_duration_since(Instant::now())),
            None => return self.pane.wait_for_exit_until(deadline),
        };
        match state {
            Ok(Some(state)) => self.apply_typed_run_state(Ok(state)),
            Ok(None) => false,
            Err(error) => self.apply_typed_run_state(Err(error)),
        }
    }

    fn apply_typed_run_state(&mut self, state: anyhow::Result<WaitState>) -> bool {
        match state {
            Ok(WaitState::Starting) => {
                self.push_typed_run_observation("starting", "status", "starting");
                false
            }
            Ok(WaitState::Running(report)) => {
                let status = report.status.as_str();
                self.push_typed_run_observation(&format!("status:{status}"), "status", status);
                false
            }
            Ok(WaitState::Terminal {
                report,
                worker_success,
            }) => {
                let worker = if worker_success { "ok" } else { "failed" };
                let detail = format!("{} · worker {worker}", report.status.as_str());
                let verb = if worker_success { "done" } else { "error" };
                self.push_typed_run_observation(&format!("terminal:{detail}"), verb, &detail);
                self.typed_run.take();
                true
            }
            Ok(WaitState::Failed {
                report,
                worker_exit,
            }) => {
                let status = report
                    .as_ref()
                    .map(|report| report.status.as_str())
                    .unwrap_or("missing");
                let detail =
                    format!("worker exited {worker_exit} without cleanup proof · report {status}");
                self.push_typed_run_observation(&format!("failed:{detail}"), "error", &detail);
                self.typed_run.take();
                true
            }
            Err(error) => {
                let detail = format!("typed worker observation failed: {error:#}");
                self.push_typed_run_observation(
                    &format!("observation-error:{detail}"),
                    "error",
                    &detail,
                );
                false
            }
        }
    }

    fn push_typed_run_observation(&mut self, key: &str, verb: &str, detail: &str) {
        let Some(run) = self.typed_run.as_mut() else {
            return;
        };
        if run.last_observation == key {
            return;
        }
        run.last_observation = key.to_string();
        self.pane.push(emu_run_line(verb, detail));
    }

    fn typed_run_verb(&self) -> Option<String> {
        let observation = &self.typed_run.as_ref()?.last_observation;
        if let Some(status) = observation.strip_prefix("status:") {
            return Some(status.to_string());
        }
        if observation.starts_with("observation-error:") {
            return Some("error".to_string());
        }
        Some(observation.to_string())
    }

    fn terminate_typed_coordinator_if_safe(&mut self) -> anyhow::Result<&'static str> {
        let run = self
            .typed_run
            .as_mut()
            .context("sandbox has no typed coordinator")?;
        let report = run.handle.ticket().read()?.map(|report| report.summary());
        let reason = match report.as_ref() {
            None => "no report or mutable guest state was published",
            Some(report)
                if report.status.is_terminal()
                    && matches!(report.cleanup, CleanupState::Complete) =>
            {
                "terminal cleanup was already proven"
            }
            Some(report) => anyhow::bail!(
                "coordinator report is {} with unresolved cleanup",
                report.status.as_str()
            ),
        };
        run.handle
            .terminate_worker(WORKER_TERMINATION_GRACE)
            .context("failed to terminate the owned typed coordinator")?;
        self.poll_finished();
        Ok(reason)
    }
}

#[derive(Clone)]
struct SandboxStopTarget {
    key: String,
    launch: SandboxLaunch,
    report_path: Option<PathBuf>,
}

impl SandboxStopTarget {
    fn report_identity(&self) -> Option<(&str, ReportKind)> {
        match &self.launch {
            SandboxLaunch::Environment { batch_id } => {
                Some((batch_id, ReportKind::EnvironmentBatch))
            }
            SandboxLaunch::Flow { batch_id } => Some((batch_id, ReportKind::FlowFanout)),
            SandboxLaunch::ImageImport { run_id } => Some((run_id, ReportKind::ImageImport)),
            SandboxLaunch::Candidate => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExactLaneShutdown {
    target_key: String,
    run_root: PathBuf,
    run_id: String,
}

pub(super) fn open_emu(dash: &mut Dash) {
    dash.view = View::Emu;
    dash.scroll_offset = 0;
    dash.pokes.emu = true;
}

pub(super) fn emu_env_count(dash: &Dash) -> usize {
    match &dash.emu {
        EmuState::Done(inventory) => inventory.environments.len(),
        EmuState::Probing | EmuState::Failed(_) => 0,
    }
}

fn selected_environment(dash: &Dash) -> Option<&EnvironmentSnapshot> {
    match &dash.emu {
        EmuState::Done(inventory) => inventory.environments.get(dash.emu_cursor),
        EmuState::Probing | EmuState::Failed(_) => None,
    }
}

fn selected_candidate(dash: &Dash) -> Option<&ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get(index))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImageImportTarget {
    environment_id: String,
    environment_index: usize,
    candidate_id: Option<String>,
    source: PathBuf,
}

fn selected_image_import_target(
    inventory: &Inventory,
    candidates: &[ImageCandidate],
    cursor: usize,
) -> Result<ImageImportTarget, String> {
    if let Some(environment) = inventory.environments.get(cursor) {
        return image_import_target_for_environment(environment, cursor, candidates);
    }
    let candidate_index = cursor
        .checked_sub(inventory.environments.len())
        .ok_or_else(|| "select a missing environment or qcow2 candidate".to_string())?;
    let candidate = candidates
        .get(candidate_index)
        .ok_or_else(|| "select a missing environment or qcow2 candidate".to_string())?;
    image_import_target_for_candidate(candidate, &inventory.environments)
}

fn image_import_target_for_environment(
    environment: &EnvironmentSnapshot,
    environment_index: usize,
    candidates: &[ImageCandidate],
) -> Result<ImageImportTarget, String> {
    let resolved = &environment.resolved;
    if resolved.state != ResolutionState::Missing {
        return Err(format!(
            "{} is {} · image verification requires a missing environment",
            resolved.definition.id,
            resolved.state.as_str()
        ));
    }
    require_qcow2_environment(environment)?;
    if let Some(source) = resolved.image_path.as_deref() {
        if is_regular_image(source)? {
            return Ok(ImageImportTarget {
                environment_id: resolved.definition.id.clone(),
                environment_index,
                candidate_id: candidates
                    .iter()
                    .find(|candidate| candidate.path == source)
                    .map(|candidate| candidate.id.clone()),
                source: source.to_path_buf(),
            });
        }
    }
    let expected = expected_qcow2_name(environment)?;
    let mut matches = candidates
        .iter()
        .filter(|candidate| qcow2_name(&candidate.path) == Some(expected))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.path.cmp(&right.path));
    if matches.is_empty() {
        return Err(format!(
            "no qcow2 candidate exactly matches {}",
            resolved.definition.image.base.display()
        ));
    }
    if matches.len() > 1 {
        let paths = matches
            .iter()
            .map(|candidate| candidate.path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "multiple qcow2 candidates match {}: {paths}",
            resolved.definition.image.base.display()
        ));
    }
    Ok(image_import_target(
        environment,
        environment_index,
        matches[0],
    ))
}

fn image_import_target_for_candidate(
    candidate: &ImageCandidate,
    environments: &[EnvironmentSnapshot],
) -> Result<ImageImportTarget, String> {
    let candidate_name = qcow2_name(&candidate.path).ok_or_else(|| {
        format!(
            "{} is not an exact qcow2 image candidate",
            candidate.path.display()
        )
    })?;
    let mut matches = environments
        .iter()
        .enumerate()
        .filter(|(_, environment)| {
            environment.resolved.state == ResolutionState::Missing
                && expected_qcow2_name(environment).ok() == Some(candidate_name)
        })
        .collect::<Vec<_>>();
    matches.sort_by(|(_, left), (_, right)| {
        left.resolved
            .definition
            .id
            .cmp(&right.resolved.definition.id)
    });
    if matches.is_empty() {
        return Err(format!(
            "no missing environment exactly expects {}",
            candidate.path.display()
        ));
    }
    if matches.len() > 1 {
        let ids = matches
            .iter()
            .map(|(_, environment)| environment.resolved.definition.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "{} matches multiple missing environments: {ids}",
            candidate.path.display()
        ));
    }
    let (environment_index, environment) = matches[0];
    Ok(image_import_target(
        environment,
        environment_index,
        candidate,
    ))
}

fn image_import_target(
    environment: &EnvironmentSnapshot,
    environment_index: usize,
    candidate: &ImageCandidate,
) -> ImageImportTarget {
    ImageImportTarget {
        environment_id: environment.resolved.definition.id.clone(),
        environment_index,
        candidate_id: Some(candidate.id.clone()),
        source: candidate.path.clone(),
    }
}

fn expected_qcow2_name(environment: &EnvironmentSnapshot) -> Result<&OsStr, String> {
    require_qcow2_environment(environment)?;
    qcow2_name(&environment.resolved.definition.image.base).ok_or_else(|| {
        format!(
            "{} does not declare an exact qcow2 filename",
            environment.resolved.definition.id
        )
    })
}

fn require_qcow2_environment(environment: &EnvironmentSnapshot) -> Result<(), String> {
    if environment.resolved.definition.image.kind != "qcow2" {
        return Err(format!(
            "{} expects {} media, not qcow2",
            environment.resolved.definition.id, environment.resolved.definition.image.kind
        ));
    }
    Ok(())
}

fn is_regular_image(path: &Path) -> Result<bool, String> {
    match path.symlink_metadata() {
        Ok(metadata) => Ok(metadata.file_type().is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "failed to inspect associated image {}: {error}",
            path.display()
        )),
    }
}

fn qcow2_name(path: &Path) -> Option<&OsStr> {
    let extension = path.extension()?.to_str()?;
    extension
        .eq_ignore_ascii_case("qcow2")
        .then(|| path.file_name())?
}

pub(super) fn is_running(dash: &Dash, id: &str) -> bool {
    dash.active_runs
        .get(id)
        .is_some_and(ActiveSandboxRun::is_live)
}

pub(super) fn act_emu(dash: &mut Dash, _modified: bool) {
    if let Some((id, state, run_root, live_runs)) = selected_environment(dash).map(|snapshot| {
        (
            snapshot.resolved.definition.id.clone(),
            snapshot.resolved.state,
            snapshot.resolved.run_root.clone(),
            snapshot
                .live_runs()
                .map(|run| run.run_id.clone())
                .collect::<Vec<_>>(),
        )
    }) {
        if is_running(dash, &id) {
            return;
        }
        if live_runs.len() == 1 {
            fire_env_down(dash, &id, &live_runs[0]);
            return;
        }
        if live_runs.len() > 1 {
            dash.notice = Some((
                Instant::now(),
                format!(
                    "{} has {} live lanes · choose an exact run",
                    id,
                    live_runs.len()
                ),
            ));
            return;
        }
        if state == ResolutionState::Ready {
            launch_environment(dash, id, run_root);
        }
        return;
    }
    let Some(id) = selected_candidate(dash).map(|candidate| candidate.id.clone()) else {
        return;
    };
    if is_running(dash, &id) {
        fire_emu_down(dash, &id);
    } else {
        launch_candidate(dash, id);
    }
}

fn launch_environment(dash: &mut Dash, id: String, run_root: Option<PathBuf>) {
    let mut pane = LogPane::new();
    let batch_id = match crate::commands::emu::new_run_id(&format!("{id}-batch")) {
        Ok(batch_id) => batch_id,
        Err(error) => {
            pane.push(emu_run_line(
                "error",
                &format!("could not create run id: {error:#}"),
            ));
            dash.active_runs
                .insert(id.clone(), ActiveSandboxRun::environment(pane, id));
            return;
        }
    };
    match spawn_env_up(&id, &batch_id, &dash.running_worktree) {
        Some((child, rx)) => pane.attach(child, rx),
        None => pane.push(emu_run_line(
            "error",
            &format!("could not launch qol env up {id}"),
        )),
    }
    let run_root = run_root.unwrap_or_else(|| dash.running_worktree.join("target/qol-env"));
    let report_path = run_root.join(&batch_id).join("report.json");
    dash.active_runs.insert(
        id,
        ActiveSandboxRun::environment(pane, batch_id).with_report_path(report_path),
    );
}

fn launch_candidate(dash: &mut Dash, id: String) {
    let mut pane = LogPane::new();
    match spawn_qol(&["emu", "up", &id, "--windowed"]) {
        Some((child, rx)) => pane.attach(child, rx),
        None => pane.push(emu_run_line(
            "error",
            &format!("could not launch qol emu up {id}"),
        )),
    }
    dash.active_runs
        .insert(id, ActiveSandboxRun::candidate(pane));
}

pub(super) fn run_selected_flow(dash: &mut Dash) {
    let Some((id, state, workflow, live_lanes)) = selected_environment(dash).map(|snapshot| {
        (
            snapshot.resolved.definition.id.clone(),
            snapshot.resolved.state,
            snapshot
                .resolved
                .definition
                .capabilities
                .get("default_workflow")
                .cloned(),
            snapshot.live_lane_count(),
        )
    }) else {
        dash.notice = Some((
            Instant::now(),
            "select a registered sandbox environment first".to_string(),
        ));
        return;
    };
    if state != ResolutionState::Ready {
        dash.notice = Some((Instant::now(), format!("{id} is not ready")));
        return;
    }
    if is_running(dash, &id) || live_lanes > 0 {
        dash.notice = Some((
            Instant::now(),
            format!("{id} already has an active sandbox run"),
        ));
        return;
    }
    let Some(workflow) = workflow else {
        dash.notice = Some((
            Instant::now(),
            format!("{id} has no manifest-selected default workflow"),
        ));
        return;
    };
    let batch_id = match crate::commands::emu::new_run_id(&format!("flow-{workflow}")) {
        Ok(batch_id) => batch_id,
        Err(error) => {
            dash.notice = Some((
                Instant::now(),
                format!("could not create flow run id: {error:#}"),
            ));
            return;
        }
    };
    let start = sandbox_flow_start(
        &workflow,
        &id,
        &batch_id,
        dash.sandbox_flow_lanes,
        &dash.running_worktree,
    );
    let run = std::env::current_exe()
        .map_err(anyhow::Error::new)
        .and_then(|executable| crate::commands::flow::start_typed_flow(&executable, start, false))
        .map(ActiveSandboxRun::flow)
        .unwrap_or_else(|error| ActiveSandboxRun::failed(SandboxLaunch::Flow { batch_id }, &error));
    dash.active_runs.insert(id, run);
}

#[cfg(test)]
fn flow_report_path(run_root: &Path, batch_id: &str) -> PathBuf {
    run_root.join("flows").join(batch_id).join("report.json")
}

fn sandbox_flow_start(
    workflow: &str,
    environment_id: &str,
    run_id: &str,
    lanes: u32,
    worktree: &Path,
) -> FlowStart {
    FlowStart {
        workflow: workflow.to_string(),
        environment_id: environment_id.to_string(),
        worktree: worktree.to_path_buf(),
        run_id: run_id.to_string(),
        repeat: lanes,
        jobs: lanes,
        memory_mb: None,
        cpus: None,
        force: false,
    }
}

pub(super) fn emu_run_line(verb: &str, detail: &str) -> String {
    format!("  {verb:<9}{detail}")
}

pub(super) fn keep_emu_line(line: &str) -> bool {
    let trimmed = line.trim();
    !(trimmed.is_empty()
        || trimmed.starts_with("qol emu")
        || trimmed.starts_with("qol env")
        || trimmed.starts_with("qol flow")
        || trimmed.starts_with("hint:"))
}

fn spawn_emu_verb(verb: &str, id: &str) -> Option<(Child, Receiver<String>)> {
    let exe = std::env::current_exe().ok()?;
    let root = crate::workspace::repo_root().ok()?;
    let mut child = Command::new(exe)
        .args(["emu", verb, id])
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn spawn_env_up(id: &str, batch_id: &str, worktree: &Path) -> Option<(Child, Receiver<String>)> {
    let exe = std::env::current_exe().ok()?;
    let root = crate::workspace::repo_root().ok()?;
    let mut command = Command::new(exe);
    command
        .args(["env", "up", id, "--run-id", batch_id, "--dev-worktree"])
        .arg(worktree)
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn spawn_env_down(run_id: &str) -> Option<(Child, Receiver<String>)> {
    spawn_qol(&["env", "down", run_id])
}

fn spawn_qol(args: &[&str]) -> Option<(Child, Receiver<String>)> {
    let exe = std::env::current_exe().ok()?;
    let root = crate::workspace::repo_root().ok()?;
    let mut command = Command::new(exe);
    command
        .args(args)
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn fire_env_down(dash: &mut Dash, id: &str, run_id: &str) {
    let line = match spawn_env_down(run_id) {
        Some((mut child, _)) => {
            let _ = child.wait();
            emu_run_line("down", &format!("sent to {run_id}"))
        }
        None => emu_run_line("error", &format!("could not stop {run_id}")),
    };
    dash.active_runs
        .entry(id.to_string())
        .or_insert_with(|| ActiveSandboxRun::environment(LogPane::new(), run_id))
        .pane
        .push(line);
    dash.pokes.emu = true;
}

fn fire_emu_down(dash: &mut Dash, id: &str) {
    let line = match spawn_emu_verb("down", id) {
        Some((mut child, _)) => {
            let _ = child.wait();
            emu_run_line("down", &format!("sent to {id}"))
        }
        None => emu_run_line("error", &format!("could not send down to {id}")),
    };
    if let Some(run) = dash.active_runs.get_mut(id) {
        run.pane.push(line);
    }
}

pub(super) fn open_emu_dir(dash: &mut Dash) {
    let active_dir = selected_environment(dash)
        .and_then(|snapshot| dash.active_runs.get(&snapshot.resolved.definition.id))
        .and_then(ActiveSandboxRun::report_path)
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let dir = active_dir
        .or_else(|| {
            selected_environment(dash).and_then(|snapshot| {
                snapshot
                    .latest_session()
                    .or_else(|| snapshot.latest_run())
                    .and_then(|run| run.report_path.parent().map(std::path::Path::to_path_buf))
                    .or_else(|| snapshot.resolved.run_root.clone())
            })
        })
        .or_else(emu_dir);
    let Some(dir) = dir else {
        return;
    };
    let message = match std::fs::create_dir_all(&dir) {
        Ok(()) => match crate::host_facade::open_path(&dir) {
            Ok(outcome) if outcome.desktop_opened() => {
                format!("opened emu folder {}", dir.display())
            }
            Ok(_) => {
                format!(
                    "could not open emu folder {} · no desktop session",
                    dir.display()
                )
            }
            Err(error) => format!("could not open emu folder {} · {error:#}", dir.display()),
        },
        Err(error) => format!("could not prepare emu folder {} · {error}", dir.display()),
    };
    dash.notice = Some((Instant::now(), message));
}

pub(super) fn repair_sandbox_cleanup(dash: &mut Dash) {
    let message = match crate::commands::env::repair_cleanup() {
        Ok(summary) if summary.remaining == 0 => format!(
            "swept {} stale launch(es); every cleanup report is verified",
            summary.swept
        ),
        Ok(summary) => format!(
            "swept {} stale launch(es); {} warning(s) still lack cleanup proof",
            summary.swept, summary.remaining
        ),
        Err(error) => format!("cleanup repair failed: {error:#}"),
    };
    dash.notice = Some((Instant::now(), message));
    dash.pokes.emu = true;
}

pub(super) fn verify_selected_image(dash: &mut Dash) {
    let target = match &dash.emu {
        EmuState::Done(inventory) => {
            selected_image_import_target(inventory, &dash.emu_candidates, dash.emu_cursor)
        }
        EmuState::Probing => Err("sandbox inventory is still scanning".to_string()),
        EmuState::Failed(error) => Err(format!("sandbox inventory unavailable: {error}")),
    };
    let target = match target {
        Ok(target) => target,
        Err(error) => {
            dash.notice = Some((Instant::now(), error));
            return;
        }
    };
    let candidate_running = target
        .candidate_id
        .as_deref()
        .is_some_and(|candidate_id| is_running(dash, candidate_id));
    if is_running(dash, &target.environment_id) || candidate_running {
        dash.notice = Some((
            Instant::now(),
            format!(
                "{} already has an active sandbox run",
                target.environment_id
            ),
        ));
        return;
    }
    let has_live_lanes = selected_environment_has_live_lanes(dash, &target.environment_id);
    if has_live_lanes {
        dash.notice = Some((
            Instant::now(),
            format!("{} already has active sandbox lanes", target.environment_id),
        ));
        return;
    }
    let run_id = match crate::commands::emu::new_run_id("image-import") {
        Ok(run_id) => run_id,
        Err(error) => {
            dash.notice = Some((
                Instant::now(),
                format!("could not create image import run id: {error:#}"),
            ));
            return;
        }
    };
    let start = sandbox_image_import_start(
        &target.environment_id,
        &target.source,
        &run_id,
        &dash.running_worktree,
    );
    let started = std::env::current_exe()
        .map_err(anyhow::Error::new)
        .and_then(|executable| {
            crate::commands::env::start_typed_image_import(&executable, start, false)
        });
    let (run, notice) = match started {
        Ok(handle) => (
            ActiveSandboxRun::image_import(handle),
            format!(
                "verifying {} for {}",
                target.source.display(),
                target.environment_id
            ),
        ),
        Err(error) => (
            ActiveSandboxRun::failed(
                SandboxLaunch::ImageImport {
                    run_id: run_id.clone(),
                },
                &error,
            ),
            format!("image verification failed to start: {error:#} · → opens error log"),
        ),
    };
    dash.notice = Some((Instant::now(), notice));
    dash.emu_cursor = target.environment_index;
    dash.active_runs.insert(target.environment_id, run);
}

fn selected_environment_has_live_lanes(dash: &Dash, environment_id: &str) -> bool {
    let EmuState::Done(inventory) = &dash.emu else {
        return false;
    };
    inventory.environments.iter().any(|environment| {
        environment.resolved.definition.id == environment_id && environment.live_lane_count() > 0
    })
}

fn sandbox_image_import_start(
    environment_id: &str,
    source: &Path,
    run_id: &str,
    worktree: &Path,
) -> ImageImportStart {
    ImageImportStart {
        environment_id: environment_id.to_string(),
        source: source.to_path_buf(),
        worktree: worktree.to_path_buf(),
        run_id: run_id.to_string(),
    }
}

pub(super) fn drain_emu_runs(dash: &mut Dash) {
    let mut finished = false;
    for run in dash.active_runs.values_mut() {
        if run.poll_finished() {
            finished = true;
        }
    }
    if finished {
        dash.pokes.emu = true;
        dash.pokes.doctor = true;
    }
}

pub(super) fn stop_emu_runs(dash: &mut Dash) {
    let targets = dash
        .active_runs
        .iter()
        .filter(|(_, run)| run.is_live())
        .map(|(id, run)| SandboxStopTarget {
            key: id.clone(),
            launch: run.launch.clone(),
            report_path: run.report_path.clone(),
        })
        .collect::<Vec<_>>();
    let cancellation = targets
        .iter()
        .filter_map(|target| {
            target.report_identity().map(|(run_id, _)| {
                let result = dash
                    .active_runs
                    .get(&target.key)
                    .ok_or_else(|| anyhow::anyhow!("sandbox run disappeared before cancellation"))
                    .and_then(|run| run.request_cancellation(run_id));
                (target.key.clone(), run_id.to_string(), result)
            })
        })
        .collect::<Vec<_>>();
    for (key, run_id, result) in cancellation {
        let detail = match result {
            Ok(_) => format!("requested for {run_id}"),
            Err(error) => format!("request failed for {run_id}: {error:#}"),
        };
        push_shutdown_line(dash, &key, "cancel", &detail);
    }
    let candidates = targets
        .iter()
        .filter(|target| matches!(&target.launch, SandboxLaunch::Candidate))
        .map(|target| target.key.clone())
        .collect::<Vec<_>>();
    if !candidates.is_empty() {
        let context = std::env::current_exe()
            .map_err(anyhow::Error::new)
            .and_then(|executable| crate::workspace::repo_root().map(|root| (executable, root)));
        let results = match context {
            Ok((executable, root)) => execute_shutdowns(&candidates, &|id| {
                request_candidate_shutdown(&executable, &root, id)
            }),
            Err(error) => {
                let detail = error.to_string();
                candidates
                    .iter()
                    .map(|_| Err(anyhow::anyhow!(detail.clone())))
                    .collect()
            }
        };
        for (id, result) in candidates.iter().zip(results) {
            let detail = match result {
                Ok(()) => format!("sent to {id}"),
                Err(error) => format!("could not stop {id}: {error:#}"),
            };
            push_shutdown_line(dash, id, "down", &detail);
        }
    }

    let first_deadline = Instant::now() + SANDBOX_STOP_GRACE;
    let timed_out = wait_for_sandbox_targets(dash, &targets, first_deadline);
    let mut exact_shutdowns = Vec::new();
    for target in targets.iter().filter(|target| {
        timed_out.contains(&target.key)
            && target
                .report_identity()
                .is_some_and(|(_, kind)| kind.is_session())
    }) {
        match exact_owned_lane_shutdowns(target) {
            Ok(mut shutdowns) => exact_shutdowns.append(&mut shutdowns),
            Err(error) => push_shutdown_line(
                dash,
                &target.key,
                "cleanup",
                &format!("lane escalation refused: {error:#}"),
            ),
        }
    }
    if !exact_shutdowns.is_empty() {
        let executable = std::env::current_exe();
        let results = match executable {
            Ok(executable) => execute_shutdowns(&exact_shutdowns, &|shutdown| {
                request_exact_lane_shutdown(&executable, shutdown)
            }),
            Err(error) => {
                let detail = error.to_string();
                exact_shutdowns
                    .iter()
                    .map(|_| Err(anyhow::anyhow!(detail.clone())))
                    .collect()
            }
        };
        for (shutdown, result) in exact_shutdowns.iter().zip(results) {
            let detail = match result {
                Ok(()) => format!("verified stop sent to {}", shutdown.run_id),
                Err(error) => format!("{} stop failed: {error:#}", shutdown.run_id),
            };
            push_shutdown_line(dash, &shutdown.target_key, "cleanup", &detail);
        }
    }

    let second_targets = bounded_followup_targets(&targets, &timed_out);
    if !second_targets.is_empty() {
        let second_deadline = Instant::now() + SANDBOX_STOP_GRACE;
        wait_for_sandbox_targets(dash, &second_targets, second_deadline);
    }
    let escalation_keys = targets
        .iter()
        .filter(|target| {
            dash.active_runs
                .get(&target.key)
                .is_some_and(|run| run.is_live() && run.typed_run.is_some())
        })
        .map(|target| target.key.clone())
        .collect::<Vec<_>>();
    let escalations = escalation_keys
        .into_iter()
        .map(|key| {
            let result = dash
                .active_runs
                .get_mut(&key)
                .context("sandbox run disappeared before coordinator escalation")
                .and_then(ActiveSandboxRun::terminate_typed_coordinator_if_safe);
            (key, result)
        })
        .collect::<Vec<_>>();
    for (key, result) in escalations {
        let detail = match result {
            Ok(reason) => format!("owned coordinator stopped because {reason}"),
            Err(error) => format!("coordinator retained: {error:#}"),
        };
        push_shutdown_line(dash, &key, "coordinator", &detail);
    }
    for target in &targets {
        if target.report_identity().is_none() {
            if timed_out.contains(&target.key) {
                push_shutdown_line(
                    dash,
                    &target.key,
                    "cleanup",
                    "coordinator remains live and was not killed without guest-exit proof",
                );
            }
            continue;
        }
        match verify_owned_cleanup(target) {
            Ok(report) => push_shutdown_line(
                dash,
                &target.key,
                "cleanup",
                &format!("verified by {}", report.report_path.display()),
            ),
            Err(error) => push_shutdown_line(
                dash,
                &target.key,
                "cleanup",
                &format!("unverified: {error:#}; durable owner and report remain for recovery"),
            ),
        }
    }
}

fn bounded_followup_targets(
    targets: &[SandboxStopTarget],
    timed_out: &BTreeSet<String>,
) -> Vec<SandboxStopTarget> {
    targets
        .iter()
        .filter(|target| timed_out.contains(&target.key) && target.report_identity().is_some())
        .cloned()
        .collect()
}

fn push_shutdown_line(dash: &mut Dash, key: &str, verb: &str, detail: &str) {
    if let Some(run) = dash.active_runs.get_mut(key) {
        run.pane.push(emu_run_line(verb, detail));
    }
}

fn wait_for_sandbox_targets(
    dash: &mut Dash,
    targets: &[SandboxStopTarget],
    deadline: Instant,
) -> BTreeSet<String> {
    targets
        .iter()
        .filter_map(|target| {
            let exited = dash
                .active_runs
                .get_mut(&target.key)
                .is_none_or(|run| run.wait_for_exit_until(deadline));
            (!exited).then(|| target.key.clone())
        })
        .collect()
}

fn verify_owned_cleanup(target: &SandboxStopTarget) -> anyhow::Result<RunReport> {
    let (run_id, kind) = target
        .report_identity()
        .ok_or_else(|| anyhow::anyhow!("sandbox launch has no owned report"))?;
    let report_path = target
        .report_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("sandbox launch did not record its report path"))?;
    let report = qol_dev_env::read_report_checked(report_path, run_id, &kind)?
        .ok_or_else(|| anyhow::anyhow!("owned report {} is missing", report_path.display()))?;
    if !report.status.is_terminal() {
        anyhow::bail!(
            "owned report status `{}` is not terminal",
            report.status.as_str()
        );
    }
    if !report.cleanup.is_complete() {
        let detail = match &report.cleanup {
            CleanupState::Incomplete(error) => error.as_str(),
            CleanupState::Pending => "cleanup is pending",
            CleanupState::Complete => unreachable!(),
        };
        anyhow::bail!("owned cleanup is incomplete: {detail}");
    }
    Ok(report)
}

fn exact_owned_lane_shutdowns(
    target: &SandboxStopTarget,
) -> anyhow::Result<Vec<ExactLaneShutdown>> {
    let (run_id, kind) = target
        .report_identity()
        .filter(|(_, kind)| kind.is_session())
        .ok_or_else(|| anyhow::anyhow!("sandbox launch does not own child lanes"))?;
    let report_path = target
        .report_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("sandbox launch did not record its report path"))?;
    let report = qol_dev_env::read_report_checked(report_path, run_id, &kind)?
        .ok_or_else(|| anyhow::anyhow!("owned report {} is missing", report_path.display()))?;
    let run_root = canonical_owned_case_root(report_path, run_id, &kind)?;
    Ok(report
        .owned_lane_run_ids()?
        .into_iter()
        .map(|run_id| ExactLaneShutdown {
            target_key: target.key.clone(),
            run_root: run_root.clone(),
            run_id,
        })
        .collect())
}

fn canonical_owned_case_root(
    report_path: &Path,
    run_id: &str,
    kind: &ReportKind,
) -> anyhow::Result<PathBuf> {
    let batch_dir = report_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("owned report has no run directory"))?;
    if report_path.file_name() != Some(OsStr::new("report.json"))
        || batch_dir.file_name() != Some(OsStr::new(run_id))
    {
        anyhow::bail!("owned report path does not match its run identity");
    }
    let run_root = match kind {
        ReportKind::EnvironmentBatch => batch_dir
            .parent()
            .ok_or_else(|| anyhow::anyhow!("environment report has no run root"))?,
        ReportKind::FlowFanout => {
            let flows_dir = batch_dir
                .parent()
                .ok_or_else(|| anyhow::anyhow!("flow report has no flows directory"))?;
            if flows_dir.file_name() != Some(OsStr::new("flows")) {
                anyhow::bail!("flow report path has no owned flows directory");
            }
            flows_dir
                .parent()
                .ok_or_else(|| anyhow::anyhow!("flow report has no run root"))?
        }
        ReportKind::Environment
        | ReportKind::Flow
        | ReportKind::ImageImport
        | ReportKind::Unknown(_) => {
            anyhow::bail!("report kind `{}` is not an aggregate batch", kind.as_str())
        }
    };
    let canonical_root = run_root
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("failed to resolve run root: {error}"))?;
    let canonical_report = report_path
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("failed to resolve owned report: {error}"))?;
    let expected_report = match kind {
        ReportKind::EnvironmentBatch => canonical_root.join(run_id).join("report.json"),
        ReportKind::FlowFanout => canonical_root
            .join("flows")
            .join(run_id)
            .join("report.json"),
        ReportKind::Environment
        | ReportKind::Flow
        | ReportKind::ImageImport
        | ReportKind::Unknown(_) => unreachable!(),
    };
    if canonical_report != expected_report {
        anyhow::bail!("owned report escapes its canonical run root");
    }
    let canonical_cases = canonical_root
        .join("cases")
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("failed to resolve case run root: {error}"))?;
    if canonical_cases != canonical_root.join("cases") {
        anyhow::bail!("case run root escapes its canonical environment root");
    }
    Ok(canonical_cases)
}

fn execute_shutdowns<T: Sync>(
    targets: &[T],
    execute: &(impl Fn(&T) -> anyhow::Result<()> + Sync),
) -> Vec<anyhow::Result<()>> {
    thread::scope(|scope| {
        let handles = targets
            .iter()
            .map(|target| scope.spawn(move || execute(target)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("lane shutdown worker panicked")))
            })
            .collect()
    })
}

fn request_candidate_shutdown(executable: &Path, root: &Path, id: &str) -> anyhow::Result<()> {
    let mut command = Command::new(executable);
    command
        .args(["emu", "down", "--"])
        .arg(id)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::commands::dev_env::clear_host_session(&mut command);
    let status = command.status()?;
    if status.success() {
        return Ok(());
    }
    anyhow::bail!("qol emu down exited with {status}")
}

fn request_exact_lane_shutdown(
    executable: &Path,
    shutdown: &ExactLaneShutdown,
) -> anyhow::Result<()> {
    let mut command = Command::new(executable);
    command
        .arg("emu")
        .arg("down")
        .arg("--run-root")
        .arg(&shutdown.run_root)
        .arg("--")
        .arg(&shutdown.run_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::commands::dev_env::clear_host_session(&mut command);
    let status = command.status()?;
    if status.success() {
        return Ok(());
    }
    anyhow::bail!("qol emu down exited with {status}")
}

pub(super) fn open_emu_detail(dash: &mut Dash) {
    if let Some(snapshot) = selected_environment(dash).cloned() {
        let id = snapshot.resolved.definition.id.clone();
        let mut info = emu_info_lines(&snapshot);
        let mut warnings = snapshot
            .attention_runs()
            .map(|(run, concern)| attention_run_line(run, concern))
            .collect::<Vec<_>>();
        warnings.reverse();
        if let Some(run) = dash.active_runs.get(&id) {
            if let Some(report_path) = run.report_path() {
                info.push(info_row(
                    "active report",
                    &report_path.display().to_string(),
                ));
            }
            if let Some(worker_log_path) = run.worker_log_path() {
                info.push(info_row(
                    "worker log",
                    &worker_log_path.display().to_string(),
                ));
            }
        }
        let log_path = snapshot.runs.iter().find_map(|run| run.log_path.clone());
        set_emu_detail(dash, id, info, warnings, log_path);
        return;
    }
    let Some(candidate) = selected_candidate(dash).cloned() else {
        return;
    };
    let detail = newest_run_detail(&candidate.id);
    let info = candidate_info_lines(&candidate, detail.as_ref());
    let log_path = detail.as_ref().map(RunDetail::run_log);
    set_emu_detail(dash, candidate.id, info, Vec::new(), log_path);
}

fn set_emu_detail(
    dash: &mut Dash,
    id: String,
    info: Vec<Line<'static>>,
    warnings: Vec<Line<'static>>,
    log_path: Option<std::path::PathBuf>,
) {
    let replay = if dash.active_runs.contains_key(&id) {
        None
    } else {
        log_path.as_deref().map(LogPane::replay)
    };
    dash.emu_detail = Some(EmuDetail {
        id,
        info,
        warnings,
        replay,
    });
    dash.view = View::EmuDetail;
    dash.scroll_offset = 0;
    dash.close_filters();
}

pub(super) fn emu_detail_ring(dash: &Dash) -> Option<&LogRing> {
    let detail = dash.emu_detail.as_ref()?;
    if let Some(run) = dash.active_runs.get(&detail.id) {
        return Some(&run.pane.ring);
    }
    detail.replay.as_ref().map(|pane| &pane.ring)
}

pub(super) fn emu_detail_shows_warnings(dash: &Dash) -> bool {
    dash.emu_detail.as_ref().is_some_and(|detail| {
        !detail.warnings.is_empty() && !dash.active_runs.contains_key(&detail.id)
    })
}

pub(super) fn emu_detail_scroll_len(dash: &Dash) -> usize {
    if emu_detail_shows_warnings(dash) {
        return dash
            .emu_detail
            .as_ref()
            .map_or(0, |detail| detail.warnings.len());
    }
    emu_detail_ring(dash).map_or(0, LogRing::len)
}

pub(super) fn live_verb(dash: &Dash, id: &str) -> Option<String> {
    let run = dash.active_runs.get(id)?;
    if !run.is_live() {
        return None;
    }
    if let Some(verb) = run.typed_run_verb() {
        return Some(verb);
    }
    let latest = run.pane.ring.lines.back()?;
    Some(
        latest
            .split_whitespace()
            .next()
            .unwrap_or("running")
            .to_string(),
    )
}

fn state_color(state: ResolutionState) -> Color {
    match state {
        ResolutionState::Ready => accent(),
        ResolutionState::Missing => Color::Yellow,
        ResolutionState::Unsupported => Color::Red,
    }
}

fn emu_info_lines(snapshot: &EnvironmentSnapshot) -> Vec<Line<'static>> {
    let environment = &snapshot.resolved;
    let color = state_color(environment.state);
    let mut head = vec![
        "● ".fg(color).bold(),
        environment.state.as_str().fg(color).bold(),
        format!(" · {}", environment.definition.backend).fg(Color::DarkGray),
    ];
    if let Some(arch) = environment.definition.image.arch.as_deref() {
        head.push(format!(" · {arch}").fg(Color::DarkGray));
    }
    let latest = snapshot.latest_session().or_else(|| snapshot.latest_run());
    head.extend(last_run_spans(latest));
    let mut lines = vec![Line::from(head)];
    for message in &environment.messages {
        lines.push(Line::from(vec![
            "  ".into(),
            message.clone().fg(Color::DarkGray),
        ]));
    }
    if let Some(image) = environment.image_path.as_deref() {
        lines.push(info_row("image", &image.display().to_string()));
    }
    if let Some(run) = latest {
        lines.push(info_row("run", &run.run_id));
        if let Some(task) = run.owner.task.as_deref() {
            lines.push(info_row("task", task));
        }
        if let Some(worktree) = run.owner.worktree.as_deref() {
            lines.push(info_row("worktree", &worktree.display().to_string()));
        }
        lines.push(info_row("report", &run.report_path.display().to_string()));
    } else {
        lines.push(Line::from("  no runs yet".fg(Color::DarkGray)));
    }
    let warning_count = snapshot.attention_runs().count();
    if warning_count > 0 {
        let unit = if warning_count == 1 { "run" } else { "runs" };
        lines.push(Line::from(vec![
            "  cleanup  ".fg(Color::White),
            format!("{warning_count} retained {unit} lack cleanup proof")
                .fg(Color::Yellow)
                .bold(),
        ]));
        lines.push(info_row("repair", "press c to sweep and re-verify"));
    }
    lines
}

fn candidate_info_lines(
    candidate: &ImageCandidate,
    detail: Option<&RunDetail>,
) -> Vec<Line<'static>> {
    let mut head = vec![
        "○ ".fg(accent()).bold(),
        "ready".fg(accent()).bold(),
        " · candidate".fg(Color::DarkGray),
    ];
    if let Some(detail) = detail {
        head.push(format!(" · {}", detail.arch).fg(Color::DarkGray));
    }
    let mut lines = vec![Line::from(head)];
    match candidate.arch {
        crate::commands::emu::ArchGuess::Known(arch) => {
            lines.push(info_row("arch", arch.as_str()));
        }
        crate::commands::emu::ArchGuess::Assumed(arch) => {
            lines.push(info_row("arch", &format!("assumed {}", arch.as_str())));
        }
    }
    match detail {
        Some(detail) => {
            lines.push(info_row("image", &detail.image_path));
            lines.push(info_row("accel", &detail.acceleration));
            lines.push(info_row("run dir", &detail.run_dir.display().to_string()));
        }
        None => {
            lines.push(info_row("image", &candidate.path.display().to_string()));
            lines.push(Line::from("  no runs yet".fg(Color::DarkGray)));
        }
    }
    lines
}

fn info_row(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        format!("  {label:<8} ").fg(Color::White),
        value.to_string().fg(Color::DarkGray),
    ])
}

pub(super) fn emu_status(state: &EmuState) -> (Color, Vec<Span<'static>>) {
    let inventory = match state {
        EmuState::Probing => {
            return (
                Color::Yellow,
                vec![
                    "scanning".fg(Color::Yellow).bold(),
                    " · → open".fg(Color::DarkGray),
                ],
            )
        }
        EmuState::Done(inventory) => inventory,
        EmuState::Failed(error) => {
            return (
                Color::Red,
                vec![
                    "registry error".fg(Color::Red).bold(),
                    format!(" · {error}").fg(Color::DarkGray),
                ],
            )
        }
    };
    let environments = &inventory.environments;
    if environments.is_empty() {
        return (
            Color::Yellow,
            vec![
                "no sandboxes".fg(Color::Yellow).bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    let ready = environments
        .iter()
        .filter(|snapshot| snapshot.resolved.state == ResolutionState::Ready)
        .count();
    let missing = environments
        .iter()
        .filter(|snapshot| snapshot.resolved.state == ResolutionState::Missing)
        .count();
    let unsupported = environments
        .iter()
        .filter(|snapshot| snapshot.resolved.state == ResolutionState::Unsupported)
        .count();
    let active = environments
        .iter()
        .map(EnvironmentSnapshot::live_lane_count)
        .sum::<u64>();
    let attention = environments
        .iter()
        .map(|snapshot| snapshot.attention_runs().count())
        .sum::<usize>()
        + inventory
            .unassigned_runs
            .iter()
            .filter(|run| run.needs_attention())
            .count()
        + inventory.issues.len();
    if attention > 0 {
        let summary = match active {
            0 => format!("{attention} flagged"),
            _ => format!("{active} active · {attention} flagged"),
        };
        return (
            Color::Yellow,
            vec![
                summary.fg(Color::Yellow).bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    if active > 0 {
        return (
            Color::Yellow,
            vec![
                format!("{active} active").fg(Color::Yellow).bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    if ready > 0 {
        return (
            accent(),
            vec![
                format!("{} envs · {ready} ready", environments.len())
                    .fg(accent())
                    .bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    let color = if unsupported == environments.len() {
        Color::Red
    } else {
        Color::Yellow
    };
    (
        color,
        vec![
            format!(
                "{} envs · {missing} missing · {unsupported} unsupported",
                environments.len()
            )
            .fg(color)
            .bold(),
            " · → open".fg(Color::DarkGray),
        ],
    )
}

pub(super) fn emu_empty_lines(config: &str) -> Vec<Line<'static>> {
    vec![
        Line::from("  no sandbox environments found".fg(Color::DarkGray)),
        Line::from(vec![
            "  config ".fg(Color::DarkGray),
            config.to_string().fg(Color::White),
        ]),
    ]
}

pub(super) fn candidate_line(
    candidate: &ImageCandidate,
    selected: bool,
    live_verb: Option<String>,
) -> Line<'static> {
    let caret = caret(selected);
    let id_span = if selected {
        candidate.id.clone().fg(Color::White).bold()
    } else {
        candidate.id.clone().fg(Color::White)
    };
    let mut spans = vec![caret, "○ ".fg(Color::DarkGray), id_span];
    match live_verb {
        Some(verb) => {
            spans.push(format!("  {verb}").fg(Color::Yellow).bold());
            spans.push(" · → log".fg(Color::DarkGray));
        }
        None => {
            spans.push("  ready".fg(accent()));
            if let crate::commands::emu::ArchGuess::Assumed(arch) = candidate.arch {
                spans.push(format!(" · arch assumed {}", arch.as_str()).fg(Color::DarkGray));
            }
        }
    }
    Line::from(spans)
}

fn environment_line(
    snapshot: &EnvironmentSnapshot,
    selected: bool,
    live_verb: Option<String>,
    flow_lanes: u32,
) -> Line<'static> {
    let environment = &snapshot.resolved;
    let color = state_color(environment.state);
    let caret = caret(selected);
    let id_span = if selected {
        environment.definition.id.clone().fg(Color::White).bold()
    } else {
        environment.definition.id.clone().fg(Color::White)
    };
    let mut spans = vec![
        caret,
        "● ".fg(color).bold(),
        id_span,
        format!("  {}", environment.state.as_str()).fg(color).bold(),
        format!(" · {}", environment.definition.backend).fg(Color::DarkGray),
    ];
    match live_verb {
        Some(verb) => {
            spans.push(format!(" · {verb}").fg(Color::Yellow).bold());
            spans.push(" · → log".fg(Color::DarkGray));
        }
        None if snapshot.live_lane_count() > 0 => {
            spans.push(
                format!(" · {} active", snapshot.live_lane_count())
                    .fg(Color::Yellow)
                    .bold(),
            );
        }
        None => spans.extend(last_run_spans(
            snapshot.latest_session().or_else(|| snapshot.latest_run()),
        )),
    }
    if selected {
        let unit = if flow_lanes == 1 { "lane" } else { "lanes" };
        spans.push(format!(" · flow {flow_lanes} {unit}").fg(accent()).bold());
    }
    Line::from(spans)
}

pub(super) fn draw_emu(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let selectable = emu_env_count(dash) + dash.emu_candidates.len();
    if selectable > 0 && dash.emu_cursor >= selectable {
        dash.emu_cursor = selectable - 1;
    }
    let (mut lines, mut selected_line) = match &dash.emu {
        EmuState::Probing => (
            vec![Line::from("  scanning sandboxes".fg(Color::Yellow))],
            None,
        ),
        EmuState::Done(inventory) => sandbox_inventory_lines(dash, inventory),
        EmuState::Failed(error) => (
            vec![Line::from(vec![
                "  registry error ".fg(Color::Red).bold(),
                error.clone().fg(Color::DarkGray),
            ])],
            None,
        ),
    };
    if !matches!(dash.emu, EmuState::Done(_)) {
        append_candidate_lines(dash, 0, &mut lines, &mut selected_line);
    }
    let total = lines.len();
    let height = list_capacity(area.height);
    dash.log_height = height;
    let start = cursor_window_start(total, height, selected_line.unwrap_or_default());
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();
    view_content(frame, area, visible);
    NavigationOverflow::from_window(start, height, total)
}

fn sandbox_inventory_lines(
    dash: &Dash,
    inventory: &Inventory,
) -> (Vec<Line<'static>>, Option<usize>) {
    let mut lines = Vec::new();
    let mut selected_line = None;
    if inventory.environments.is_empty() {
        let config = crate::commands::dev_env::config_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "~/.config/qol/dev-envs.toml".to_string());
        lines.extend(emu_empty_lines(&config));
    }
    for (index, snapshot) in inventory.environments.iter().enumerate() {
        let selected = index == dash.emu_cursor;
        if selected {
            selected_line = Some(lines.len());
        }
        let environment = &snapshot.resolved;
        lines.push(environment_line(
            snapshot,
            selected,
            live_verb(dash, &environment.definition.id),
            dash.sandbox_flow_lanes,
        ));
        if environment.state != ResolutionState::Ready {
            lines.extend(environment.messages.iter().map(|message| {
                Line::from(vec!["    ".into(), message.clone().fg(Color::DarkGray)])
            }));
        }
        let warning_count = snapshot.attention_runs().count();
        if warning_count > 0 {
            lines.push(cleanup_warning_summary_line(warning_count));
        }
    }
    append_candidate_lines(
        dash,
        inventory.environments.len(),
        &mut lines,
        &mut selected_line,
    );
    if !inventory.unassigned_runs.is_empty() {
        let count = inventory.unassigned_runs.len();
        let warnings = inventory
            .unassigned_runs
            .iter()
            .filter(|run| run.needs_attention())
            .count();
        if warnings == 0 {
            lines.push(Line::from(vec![
                "  ○ ".fg(Color::DarkGray),
                format!("{count} unassigned history").fg(Color::DarkGray),
            ]));
        } else {
            let unit = if warnings == 1 { "warning" } else { "warnings" };
            lines.push(Line::from(vec![
                "  ! ".fg(Color::Yellow).bold(),
                format!("{warnings} unassigned {unit}").fg(Color::Yellow),
            ]));
        }
    }
    if !inventory.issues.is_empty() {
        let count = inventory.issues.len();
        let unit = if count == 1 { "error" } else { "errors" };
        lines.push(Line::from(vec![
            "  ! ".fg(Color::Red).bold(),
            format!("{count} inventory {unit}").fg(Color::Red),
        ]));
    }
    (lines, selected_line)
}

fn append_candidate_lines(
    dash: &Dash,
    environment_count: usize,
    lines: &mut Vec<Line<'static>>,
    selected_line: &mut Option<usize>,
) {
    for (index, candidate) in dash.emu_candidates.iter().enumerate() {
        let selected = environment_count + index == dash.emu_cursor;
        if selected {
            *selected_line = Some(lines.len());
        }
        lines.push(candidate_line(
            candidate,
            selected,
            live_verb(dash, &candidate.id),
        ));
    }
}

fn cleanup_warning_summary_line(count: usize) -> Line<'static> {
    let unit = if count == 1 { "warning" } else { "warnings" };
    Line::from(vec![
        "    ! ".fg(Color::Yellow).bold(),
        format!("{count} cleanup {unit}").fg(Color::Yellow),
        " · → details".fg(Color::DarkGray),
    ])
}

fn attention_run_line(run: &RunSummary, concern: RunConcern) -> Line<'static> {
    let (label, concern_color) = match concern {
        RunConcern::HistoricalFailure => ("historical failure", Color::Red),
        RunConcern::UnresolvedCleanup => ("cleanup unresolved", Color::Yellow),
    };
    Line::from(vec![
        "  ! ".fg(concern_color).bold(),
        run.run_id.clone().fg(Color::White),
        format!(" · {label}").fg(concern_color),
        " · ".fg(Color::DarkGray),
        run.status
            .as_str()
            .to_string()
            .fg(run_status_color(&run.status))
            .bold(),
    ])
}

pub(super) fn draw_emu_detail(
    frame: &mut Frame,
    dash: &mut Dash,
    area: Rect,
) -> NavigationOverflow {
    let accent = frame_accent(dash);
    let Some((id, info)) = dash
        .emu_detail
        .as_ref()
        .map(|detail| (detail.id.clone(), detail.info.clone()))
    else {
        return NavigationOverflow::default();
    };
    let info_height = spaced_height(info.len(), ITEM_GAP).min(area.height);
    view_content(
        frame,
        Rect {
            height: info_height,
            ..area
        },
        info,
    );
    let used = info_height.saturating_add(1);
    if used >= area.height {
        return NavigationOverflow::default();
    }
    let log_area = Rect {
        y: area.y + used,
        height: area.height - used,
        ..area
    };
    if emu_detail_shows_warnings(dash) {
        let warning_count = dash
            .emu_detail
            .as_ref()
            .map_or(0, |detail| detail.warnings.len());
        let (start, height) = list_window(dash, log_area, warning_count);
        let visible = dash
            .emu_detail
            .as_ref()
            .map(|detail| {
                detail
                    .warnings
                    .iter()
                    .skip(start)
                    .take(height)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        view_content(frame, log_area, visible);
        return NavigationOverflow::from_window(start, height, warning_count);
    }
    let highlight = copy_highlight(dash);
    if let Some(run) = dash.active_runs.get(&id) {
        return draw_run_log(
            frame,
            log_area,
            &run.pane.ring,
            &dash.filters.emu,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            accent,
            highlight,
        );
    }
    match dash
        .emu_detail
        .as_ref()
        .and_then(|detail| detail.replay.as_ref())
    {
        Some(pane) => draw_run_log(
            frame,
            log_area,
            &pane.ring,
            &dash.filters.emu,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            accent,
            highlight,
        ),
        None => {
            view_content(
                frame,
                log_area,
                vec![Line::from(
                    "  no run.log yet · boot to create one".fg(Color::DarkGray),
                )],
            );
            NavigationOverflow::default()
        }
    }
}

fn last_run_spans(last_run: Option<&RunSummary>) -> Vec<Span<'static>> {
    let Some(run) = last_run else {
        return Vec::new();
    };
    let color = run_status_color(&run.status);
    let observed_at = run
        .finished_at_unix_ms
        .or(run.started_at_unix_ms)
        .unwrap_or_default();
    let mut spans = vec![
        " · ".fg(Color::DarkGray),
        run_status_label(&run.status).to_string().fg(color),
        format!(" {}", relative_age(now_unix_ms(), observed_at)).fg(Color::DarkGray),
    ];
    if let Some(task) = run.owner.task.as_deref() {
        spans.push(format!(" · {task}").fg(Color::DarkGray));
    }
    if let Some(worktree) = run
        .owner
        .worktree
        .as_deref()
        .and_then(std::path::Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
    {
        spans.push(format!(" @{worktree}").fg(Color::DarkGray));
    }
    spans
}

fn run_status_label(status: &ReportStatus) -> &str {
    match status {
        ReportStatus::Abandoned => "interrupted",
        status => status.as_str(),
    }
}

fn run_status_color(status: &ReportStatus) -> Color {
    match status {
        ReportStatus::Pass => accent(),
        ReportStatus::Failed
        | ReportStatus::CleanupIncomplete
        | ReportStatus::RollbackIncomplete
        | ReportStatus::CancellationCleanupIncomplete => Color::Red,
        ReportStatus::Preparing
        | ReportStatus::Starting
        | ReportStatus::Running
        | ReportStatus::Stopping
        | ReportStatus::Recovering
        | ReportStatus::Cancelling
        | ReportStatus::Cancelled => Color::Yellow,
        ReportStatus::Skipped
        | ReportStatus::Stopped
        | ReportStatus::Abandoned
        | ReportStatus::Unknown(_) => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::testkit::*;
    use serde_json::json;
    use std::sync::{mpsc, Arc, Condvar, Mutex};

    #[cfg(target_os = "linux")]
    fn process_guardian_command() -> std::process::Command {
        crate::process_guardian::command(&std::env::current_exe().unwrap())
    }

    fn report_summary(status: &str, teardown: Option<serde_json::Value>) -> RunSummary {
        let mut document = serde_json::json!({
            "kind": "environment",
            "run_id": format!("run-{status}"),
            "status": status,
            "environment": { "id": "linux/mint" },
            "finished_at_unix_ms": 1
        });
        if let Some(teardown) = teardown {
            document["teardown"] = teardown;
        }
        qol_dev_env::parse_report(
            std::path::Path::new("/runs/report.json"),
            &serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()
        .summary()
    }

    #[test]
    fn candidate_line_uses_plain_ready_label() {
        let line = candidate_line(&known_emu_candidate("plain"), false, None);
        assert_eq!(span_text(&line.spans), "  ○ plain  ready");
    }

    #[test]
    fn candidate_line_marks_assumed_arch() {
        let line = candidate_line(&emu_candidate("plain"), false, None);
        assert_eq!(
            span_text(&line.spans),
            "  ○ plain  ready · arch assumed x86_64"
        );
    }

    #[test]
    fn candidate_line_marks_live_run_with_log_hint() {
        let line = candidate_line(&emu_candidate("plain"), true, Some("boot".to_string()));
        assert_eq!(span_text(&line.spans), "▸ ○ plain  boot · → log");
    }

    #[test]
    fn missing_environment_maps_only_to_its_exact_qcow2_candidate() {
        let environment = emu_env("linux/mint", ResolutionState::Missing);
        let candidates = vec![emu_candidate("other"), emu_candidate("mint")];
        let inventory = emu_inventory(vec![environment]);

        let target = selected_image_import_target(&inventory, &candidates, 0).unwrap();

        assert_eq!(target.environment_id, "linux/mint");
        assert_eq!(target.environment_index, 0);
        assert_eq!(target.candidate_id.as_deref(), Some("mint"));
        assert_eq!(target.source, Path::new("/a/b/mint.qcow2"));
    }

    #[test]
    fn existing_associated_image_precedes_filename_candidate_matching() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("legacy-custom-name.qcow2");
        std::fs::write(&source, b"qcow2-placeholder").unwrap();
        let mut environment = emu_env("linux/mint", ResolutionState::Missing);
        environment.resolved.image_path = Some(source.clone());
        let inventory = emu_inventory(vec![environment]);

        let target = selected_image_import_target(&inventory, &[emu_candidate("mint")], 0).unwrap();

        assert_eq!(target.environment_id, "linux/mint");
        assert_eq!(target.source, source);
        assert_eq!(target.candidate_id, None);
    }

    #[test]
    fn qcow2_candidate_maps_only_to_its_exact_missing_environment() {
        let ready = emu_env("linux/ready", ResolutionState::Ready);
        let missing = emu_env("linux/mint", ResolutionState::Missing);
        let inventory = emu_inventory(vec![ready, missing]);
        let candidates = vec![emu_candidate("mint")];

        let target =
            selected_image_import_target(&inventory, &candidates, inventory.environments.len())
                .unwrap();

        assert_eq!(target.environment_id, "linux/mint");
        assert_eq!(target.environment_index, 1);
        assert_eq!(target.source, Path::new("/a/b/mint.qcow2"));
    }

    #[test]
    fn image_import_mapping_refuses_missing_and_ambiguous_matches() {
        let environment = emu_env("linux/mint", ResolutionState::Missing);
        let inventory = emu_inventory(vec![environment.clone()]);
        let missing =
            selected_image_import_target(&inventory, &[emu_candidate("other")], 0).unwrap_err();
        assert_eq!(
            missing,
            "no qcow2 candidate exactly matches linux/mint.qcow2"
        );

        let mut duplicate = emu_candidate("mint");
        duplicate.id = "mint-copy".to_string();
        duplicate.path = PathBuf::from("/other/mint.qcow2");
        let ambiguous =
            selected_image_import_target(&inventory, &[emu_candidate("mint"), duplicate], 0)
                .unwrap_err();
        assert_eq!(
            ambiguous,
            "multiple qcow2 candidates match linux/mint.qcow2: /a/b/mint.qcow2, /other/mint.qcow2"
        );

        let mut second = environment;
        second.resolved.definition.id = "linux/mint-copy".to_string();
        let duplicate_environments = emu_inventory(vec![
            emu_env("linux/mint", ResolutionState::Missing),
            second,
        ]);
        let candidate = emu_candidate("mint");
        let ambiguous = selected_image_import_target(
            &duplicate_environments,
            std::slice::from_ref(&candidate),
            duplicate_environments.environments.len(),
        )
        .unwrap_err();
        assert_eq!(
            ambiguous,
            "/a/b/mint.qcow2 matches multiple missing environments: linux/mint, linux/mint-copy"
        );
    }

    #[test]
    fn image_import_mapping_refuses_ready_and_non_qcow2_selections() {
        let ready = emu_inventory(vec![emu_env("linux/mint", ResolutionState::Ready)]);
        let ready_error =
            selected_image_import_target(&ready, &[emu_candidate("mint")], 0).unwrap_err();
        assert_eq!(
            ready_error,
            "linux/mint is ready · image verification requires a missing environment"
        );

        let missing = emu_inventory(vec![emu_env("linux/mint", ResolutionState::Missing)]);
        let mut iso = emu_candidate("mint");
        iso.path = PathBuf::from("/a/b/mint.iso");
        let candidate_error = selected_image_import_target(
            &missing,
            std::slice::from_ref(&iso),
            missing.environments.len(),
        )
        .unwrap_err();
        assert_eq!(
            candidate_error,
            "/a/b/mint.iso is not an exact qcow2 image candidate"
        );
    }

    #[test]
    fn selected_environment_line_shows_configured_flow_lane_count() {
        let environment = emu_env("linux/mint", ResolutionState::Ready);
        let line = environment_line(&environment, true, None, 4);

        assert_eq!(
            span_text(&line.spans),
            "▸ ● linux/mint  ready · qemu · flow 4 lanes"
        );
    }

    #[test]
    fn last_run_surfaces_task_and_worktree_identity() {
        let document = serde_json::json!({
            "kind": "flow-fanout",
            "run_id": "flow-1",
            "status": "pass",
            "environment": { "id": "linux/mint" },
            "owner": {
                "pid": 7,
                "state": "released",
                "task": "qol-shot-capture",
                "worktree": "/worktrees/shot-speed",
            },
            "finished_at_unix_ms": 1,
            "payload": { "cleanup": { "complete": true } },
            "lanes": [],
        });
        let run = qol_dev_env::parse_report(
            std::path::Path::new("/runs/flow-1/report.json"),
            &serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()
        .summary();

        assert!(span_text(&last_run_spans(Some(&run))).contains("· qol-shot-capture @shot-speed"));
    }

    #[test]
    fn clean_abandoned_run_is_presented_as_interrupted_history() {
        let run = report_summary(
            "abandoned",
            Some(json!({
                "status": "complete",
                "qemu_exit_verified": true,
                "tree_exit_verified": true,
                "removed": [],
            })),
        );

        assert!(span_text(&last_run_spans(Some(&run))).contains("· interrupted"));
        assert_eq!(run_status_color(&run.status), Color::DarkGray);
    }

    #[test]
    fn clean_unassigned_reports_are_neutral_history() {
        let mut inventory = emu_inventory(vec![emu_env("linux/mint", ResolutionState::Ready)]);
        inventory.unassigned_runs.push(report_summary(
            "stopped",
            Some(json!({
                "status": "complete",
                "qemu_exit_verified": true,
                "tree_exit_verified": true,
                "removed": [],
            })),
        ));
        let dash = Dash::new(Vec::new());

        let (lines, _) = sandbox_inventory_lines(&dash, &inventory);
        let rendered = lines
            .iter()
            .map(|line| span_text(&line.spans))
            .collect::<Vec<_>>();

        assert!(rendered
            .iter()
            .any(|line| line == "  ○ 1 unassigned history"));
    }

    #[test]
    fn typed_flow_start_uses_the_running_worktree_and_exact_lane_plan() {
        let cases = [
            ("base", Path::new("/qol/base")),
            ("named", Path::new("/qol/worktrees/shot-speed")),
        ];
        for (label, worktree) in cases {
            let start =
                sandbox_flow_start("qol-shot-capture", "linux/mint", "flow-run-1", 7, worktree);

            assert_eq!(start.worktree, worktree, "case: {label}");
            assert_eq!(start.workflow, "qol-shot-capture", "case: {label}");
            assert_eq!(start.environment_id, "linux/mint", "case: {label}");
            assert_eq!(start.run_id, "flow-run-1", "case: {label}");
            assert_eq!(start.repeat, 7, "case: {label}");
            assert_eq!(start.jobs, 7, "case: {label}");
            assert_eq!(start.memory_mb, None, "case: {label}");
            assert_eq!(start.cpus, None, "case: {label}");
            assert!(!start.force, "case: {label}");
            assert!(start.validate().is_ok(), "case: {label}");
        }
    }

    #[test]
    fn typed_flow_ticket_preserves_its_exact_report_identity() {
        let start = sandbox_flow_start(
            "qol-shot-capture",
            "linux/mint",
            "flow-1",
            1,
            Path::new("/qol/worktree"),
        );
        let ticket = start
            .ticket(Path::new("/qol/worktree/target/qol-env"))
            .unwrap();

        assert_eq!(
            ticket.report_path,
            Path::new("/qol/worktree/target/qol-env/flows/flow-1/report.json")
        );
    }

    #[test]
    fn image_import_start_uses_the_exact_running_worktree_and_neutral_run_id() {
        let start = sandbox_image_import_start(
            "linux/mint-cinnamon",
            Path::new("/images/linux-mint-cinnamon.qcow2"),
            "image-import-1234",
            Path::new("/qol/worktrees/shot-speed"),
        );

        assert_eq!(start.environment_id, "linux/mint-cinnamon");
        assert_eq!(start.source, Path::new("/images/linux-mint-cinnamon.qcow2"));
        assert_eq!(start.worktree, Path::new("/qol/worktrees/shot-speed"));
        assert_eq!(start.run_id, "image-import-1234");
        assert!(start.validate().is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn active_typed_flow_finishes_from_its_checked_report_and_worker_exit() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().canonicalize().unwrap();
        let start =
            sandbox_flow_start("qol-shot-capture", "linux/mint", "flow-typed", 1, &worktree);
        let ticket = start.ticket(&worktree.join("runs")).unwrap();
        std::fs::create_dir_all(ticket.report_path.parent().unwrap()).unwrap();
        std::fs::write(
            &ticket.report_path,
            serde_json::to_vec(&json!({
                "kind": "flow-fanout",
                "run_id": "flow-typed",
                "status": "pass",
                "workflow": { "repeat": 1 },
                "lanes": [{
                    "run_id": "lane-1",
                    "cleanup": { "complete": true }
                }],
                "payload": { "cleanup": { "complete": true } }
            }))
            .unwrap(),
        )
        .unwrap();
        let executable = worktree.join("worker.sh");
        std::fs::write(&executable, "#!/bin/sh\nread -r request\nexit 0\n").unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).unwrap();
        let request = qol_dev_orchestrator::FlowWorkerRequest {
            start,
            run_root: worktree.join("runs"),
            plan_fingerprint: "a".repeat(64),
            verbose: false,
        };
        let handle = qol_dev_orchestrator::start_flow_worker(
            &executable,
            process_guardian_command(),
            request,
            ticket,
        )
        .unwrap();
        let mut run = ActiveSandboxRun::flow(handle);

        assert!(run.is_live());
        assert!(run.wait_for_exit_until(Instant::now() + Duration::from_secs(2)));
        assert!(!run.is_live());
        assert!(run
            .pane
            .ring
            .lines
            .back()
            .is_some_and(|line| line.contains("done") && line.contains("pass")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reportless_typed_coordinator_can_be_stopped_with_owned_tree_proof() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().canonicalize().unwrap();
        let start = sandbox_flow_start(
            "qol-shot-capture",
            "linux/mint",
            "flow-starting",
            1,
            &worktree,
        );
        let ticket = start.ticket(&worktree.join("runs")).unwrap();
        let executable = worktree.join("worker.sh");
        std::fs::write(&executable, "#!/bin/sh\nread -r request\nsleep 30\n").unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).unwrap();
        let request = qol_dev_orchestrator::FlowWorkerRequest {
            start,
            run_root: worktree.join("runs"),
            plan_fingerprint: "a".repeat(64),
            verbose: false,
        };
        let handle = qol_dev_orchestrator::start_flow_worker(
            &executable,
            process_guardian_command(),
            request,
            ticket,
        )
        .unwrap();
        let mut run = ActiveSandboxRun::flow(handle);

        let reason = run.terminate_typed_coordinator_if_safe().unwrap();

        assert_eq!(reason, "no report or mutable guest state was published");
        assert!(!run.is_live());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn completed_typed_image_import_refreshes_inventory_from_its_exact_report() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let start = sandbox_image_import_start(
            "linux/mint-cinnamon",
            &root.join("linux-mint-cinnamon.qcow2"),
            "image-import-typed",
            &worktree,
        );
        let ticket = start.ticket(&root.join("images")).unwrap();
        std::fs::create_dir_all(ticket.report_path.parent().unwrap()).unwrap();
        std::fs::write(
            &ticket.report_path,
            serde_json::to_vec(&json!({
                "kind": "image-import",
                "run_id": "image-import-typed",
                "status": "pass",
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "staging_removed": true
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let executable = root.join("worker.sh");
        std::fs::write(&executable, "#!/bin/sh\nread -r request\nexit 0\n").unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).unwrap();
        let request = qol_dev_orchestrator::ImageImportWorkerRequest {
            start,
            image_root: root.join("images"),
            plan_fingerprint: "a".repeat(64),
            verbose: false,
        };
        let handle = qol_dev_orchestrator::start_image_import_worker(
            &executable,
            process_guardian_command(),
            request,
            ticket,
        )
        .unwrap();
        let run = ActiveSandboxRun::image_import(handle);
        let mut dash = Dash::new(Vec::new());
        dash.active_runs
            .insert("linux/mint-cinnamon".to_string(), run);

        assert!(is_running(&dash, "linux/mint-cinnamon"));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !dash.pokes.emu && Instant::now() < deadline {
            drain_emu_runs(&mut dash);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(dash.pokes.emu, "completed import must refresh inventory");
        let run = dash.active_runs.get("linux/mint-cinnamon").unwrap();
        assert_eq!(
            run.report_path(),
            Some(
                root.join("images/verified/imports/image-import-typed/report.json")
                    .as_path()
            )
        );
        assert!(!run.is_live());
        assert!(matches!(
            run.launch,
            SandboxLaunch::ImageImport { ref run_id } if run_id == "image-import-typed"
        ));
        assert!(run
            .pane
            .ring
            .lines
            .back()
            .is_some_and(|line| line.contains("done") && line.contains("pass")));
    }

    fn write_flow_report(
        run_root: &Path,
        directory_batch_id: &str,
        document: serde_json::Value,
    ) -> PathBuf {
        let report_path = flow_report_path(run_root, directory_batch_id);
        std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(run_root.join("cases")).unwrap();
        std::fs::write(&report_path, serde_json::to_vec(&document).unwrap()).unwrap();
        report_path
    }

    fn flow_stop_target(batch_id: &str, report_path: PathBuf) -> SandboxStopTarget {
        SandboxStopTarget {
            key: "linux/mint".to_string(),
            launch: SandboxLaunch::Flow {
                batch_id: batch_id.to_string(),
            },
            report_path: Some(report_path),
        }
    }

    fn environment_stop_target(batch_id: &str, report_path: PathBuf) -> SandboxStopTarget {
        SandboxStopTarget {
            key: "linux/mint".to_string(),
            launch: SandboxLaunch::Environment {
                batch_id: batch_id.to_string(),
            },
            report_path: Some(report_path),
        }
    }

    fn image_import_stop_target(run_id: &str, report_path: PathBuf) -> SandboxStopTarget {
        SandboxStopTarget {
            key: "linux/mint".to_string(),
            launch: SandboxLaunch::ImageImport {
                run_id: run_id.to_string(),
            },
            report_path: Some(report_path),
        }
    }

    #[test]
    fn image_import_shutdown_uses_its_exact_lane_report_without_session_escalation() {
        let temp = tempfile::tempdir().unwrap();
        let report_path = temp
            .path()
            .join("verified/imports/image-import-1/report.json");
        std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        std::fs::write(
            &report_path,
            serde_json::to_vec(&json!({
                "kind": "image-import",
                "run_id": "image-import-1",
                "status": "cancelled",
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "staging_removed": true
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let target = image_import_stop_target("image-import-1", report_path);

        let (_, kind) = target.report_identity().unwrap();
        assert_eq!(kind, ReportKind::ImageImport);
        assert!(kind.is_lane());
        assert!(!kind.is_session());
        assert!(verify_owned_cleanup(&target).is_ok());
        assert!(exact_owned_lane_shutdowns(&target).is_err());
    }

    #[test]
    fn timed_out_image_import_receives_a_second_bounded_wait() {
        let image_import = image_import_stop_target(
            "image-import-1",
            PathBuf::from("/images/verified/imports/image-import-1/report.json"),
        );
        let candidate = SandboxStopTarget {
            key: "manual-candidate".to_string(),
            launch: SandboxLaunch::Candidate,
            report_path: None,
        };
        let timed_out = BTreeSet::from(["linux/mint".to_string(), "manual-candidate".to_string()]);

        let targets = bounded_followup_targets(&[image_import, candidate], &timed_out);

        assert_eq!(targets.len(), 1);
        assert!(matches!(
            targets[0].launch,
            SandboxLaunch::ImageImport { ref run_id } if run_id == "image-import-1"
        ));
    }

    #[test]
    fn terminal_aggregate_requires_complete_cleanup_proof() {
        let temp = tempfile::tempdir().unwrap();
        let report_path = write_flow_report(
            temp.path(),
            "flow-1",
            json!({
                "kind": "flow-fanout",
                "run_id": "flow-1",
                "status": "cancelled",
                "workflow": { "repeat": 2 },
                "lanes": [
                    { "run_id": "lane-a", "cleanup": { "complete": true } },
                    { "run_id": "lane-b", "cleanup": { "complete": true } }
                ],
                "payload": { "cleanup": { "complete": true } }
            }),
        );
        let target = flow_stop_target("flow-1", report_path);

        let report = verify_owned_cleanup(&target).unwrap();

        assert_eq!(report.status.as_str(), "cancelled");
        assert!(report.cleanup.is_complete());
    }

    #[test]
    fn timed_out_flow_escalates_every_exact_lane_concurrently() {
        let temp = tempfile::tempdir().unwrap();
        let report_path = write_flow_report(
            temp.path(),
            "flow-1",
            json!({
                "kind": "flow-fanout",
                "run_id": "flow-1",
                "status": "cancelling",
                "workflow": { "repeat": 2 },
                "lanes": [{ "run_id": "lane-a" }, { "run_id": "lane-b" }]
            }),
        );
        let target = flow_stop_target("flow-1", report_path);
        let shutdowns = exact_owned_lane_shutdowns(&target).unwrap();
        let expected_root = temp.path().join("cases").canonicalize().unwrap();
        assert_eq!(
            shutdowns
                .iter()
                .map(|shutdown| (shutdown.run_id.as_str(), shutdown.run_root.as_path()))
                .collect::<Vec<_>>(),
            vec![
                ("lane-a", expected_root.as_path()),
                ("lane-b", expected_root.as_path())
            ]
        );

        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let worker_gate = Arc::clone(&gate);
        let worker = thread::spawn(move || {
            execute_shutdowns(&shutdowns, &|shutdown| {
                started_tx.send(shutdown.run_id.clone()).unwrap();
                let (lock, ready) = &*worker_gate;
                let released = lock.lock().unwrap();
                let (released, _) = ready
                    .wait_timeout_while(released, Duration::from_secs(2), |released| !*released)
                    .unwrap();
                if !*released {
                    anyhow::bail!("test gate timed out");
                }
                Ok(())
            })
        });
        let first = started_rx.recv_timeout(Duration::from_secs(1));
        let second = started_rx.recv_timeout(Duration::from_secs(1));
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_all();
        let results = worker.join().unwrap();

        assert!(first.is_ok(), "first exact shutdown did not start");
        assert!(second.is_ok(), "second exact shutdown waited for the first");
        assert!(results.into_iter().all(|result| result.is_ok()));
    }

    #[test]
    fn timed_out_environment_batch_escalates_its_exact_lane() {
        let temp = tempfile::tempdir().unwrap();
        let report_path = temp.path().join("environment-1/report.json");
        std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(temp.path().join("cases")).unwrap();
        std::fs::write(
            &report_path,
            serde_json::to_vec(&json!({
                "kind": "environment-batch",
                "run_id": "environment-1",
                "status": "cancelling",
                "launch": { "count": 1 },
                "runs": [{ "run_id": "lane-a" }]
            }))
            .unwrap(),
        )
        .unwrap();
        let target = environment_stop_target("environment-1", report_path);

        let shutdowns = exact_owned_lane_shutdowns(&target).unwrap();

        assert_eq!(shutdowns.len(), 1);
        assert_eq!(shutdowns[0].run_id, "lane-a");
        assert_eq!(
            shutdowns[0].run_root,
            temp.path().join("cases").canonicalize().unwrap()
        );
    }

    #[test]
    fn malformed_or_mismatched_flow_reports_refuse_lane_escalation() {
        let cases = [
            (
                "wrong-run",
                json!({
                    "kind": "flow-fanout",
                    "run_id": "another-flow",
                    "status": "cancelling",
                    "workflow": { "repeat": 1 },
                    "lanes": [{ "run_id": "lane-a" }]
                }),
            ),
            (
                "wrong-kind",
                json!({
                    "kind": "environment-batch",
                    "run_id": "flow-1",
                    "status": "cancelling",
                    "resources": { "requested_lanes": 1 },
                    "runs": [{ "run_id": "lane-a" }]
                }),
            ),
            (
                "unsafe-lane",
                json!({
                    "kind": "flow-fanout",
                    "run_id": "flow-1",
                    "status": "cancelling",
                    "workflow": { "repeat": 1 },
                    "lanes": [{ "run_id": "../foreign-lane" }]
                }),
            ),
        ];
        for (label, document) in cases {
            let temp = tempfile::tempdir().unwrap();
            let report_path = write_flow_report(temp.path(), "flow-1", document);
            let target = flow_stop_target("flow-1", report_path);

            assert!(exact_owned_lane_shutdowns(&target).is_err(), "{label}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn flow_case_root_symlinks_fail_closed() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let report_path = flow_report_path(temp.path(), "flow-1");
        std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        symlink(outside.path(), temp.path().join("cases")).unwrap();
        std::fs::write(
            &report_path,
            serde_json::to_vec(&json!({
                "kind": "flow-fanout",
                "run_id": "flow-1",
                "status": "cancelling",
                "workflow": { "repeat": 1 },
                "lanes": [{ "run_id": "lane-a" }]
            }))
            .unwrap(),
        )
        .unwrap();
        let target = flow_stop_target("flow-1", report_path);

        assert!(exact_owned_lane_shutdowns(&target).is_err());
    }

    #[test]
    fn keep_emu_line_drops_noise_lines() {
        let cases = [
            ("qol emu up", false),
            ("  hint: use -v/--verbose for detailed output", false),
            ("", false),
            ("   ", false),
            ("  boot     foo · qmp 127.0.0.1:1234", true),
            ("  verdict  pass · no qol traces survive", true),
        ];
        for (line, kept) in cases {
            assert_eq!(keep_emu_line(line), kept, "line: {line:?}");
        }
    }

    #[test]
    fn historical_failure_does_not_keep_the_sandbox_summary_red() {
        let mut environment = emu_env("linux/mint", ResolutionState::Ready);
        environment.runs = vec![report_summary(
            "failed",
            Some(serde_json::json!({
                "status": "complete",
                "qemu_exit_verified": true,
                "tree_exit_verified": true,
            })),
        )];
        let (color, spans) = emu_status(&EmuState::Done(emu_inventory(vec![environment])));

        assert_eq!(color, accent());
        assert_eq!(span_text(&spans), "1 envs · 1 ready · → open");
    }

    #[test]
    fn unresolved_cleanup_uses_a_compact_warning_summary() {
        let mut environment = emu_env("linux/mint", ResolutionState::Ready);
        environment.runs = vec![report_summary("pass", None)];
        let inventory = emu_inventory(vec![environment]);
        let run = &inventory.environments[0].runs[0];
        let concern = run.concern().unwrap();
        let line = attention_run_line(run, concern);
        let (color, spans) = emu_status(&EmuState::Done(inventory));

        assert_eq!(color, Color::Yellow);
        assert_eq!(span_text(&spans), "1 flagged · → open");
        assert_eq!(
            span_text(&line.spans),
            "  ! run-pass · cleanup unresolved · pass"
        );
        assert_eq!(line.spans[0].style.fg, Some(Color::Yellow));
        assert_eq!(line.spans[2].style.fg, Some(Color::Yellow));
        assert_eq!(line.spans[4].style.fg, Some(accent()));
    }

    #[test]
    fn sandbox_list_collapses_cleanup_history_and_keeps_selection_visible() {
        let mut first = emu_env("linux/first", ResolutionState::Ready);
        first.runs = (0..30).map(|_| report_summary("pass", None)).collect();
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![
            first,
            emu_env("linux/second", ResolutionState::Ready),
        ]));

        let rows = render_rows(&mut dash);

        assert!(
            rows.iter().any(|row| row.contains("▸ ● linux/first")),
            "the initial sandbox selection must remain visible"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("30 cleanup warnings · → details")),
            "cleanup history must collapse into one summary row"
        );
        assert!(
            !rows.iter().any(|row| row.contains("run-pass")),
            "historical report ids belong in the detail view"
        );
        open_emu_detail(&mut dash);
        let detail = dash.emu_detail.as_ref().unwrap();
        let warnings = &detail.warnings;
        assert_eq!(warnings.len(), 30);
        assert!(warnings
            .iter()
            .any(|line| span_text(&line.spans).contains("run-pass")));
        assert!(detail.info.iter().any(|line| {
            span_text(&line.spans).contains("30 retained runs lack cleanup proof")
        }));
        assert!(detail
            .info
            .iter()
            .any(|line| { span_text(&line.spans).contains("press c to sweep and re-verify") }));
    }

    #[test]
    fn active_sandbox_summary_omits_empty_attention_count() {
        let mut environment = emu_env("linux/mint", ResolutionState::Ready);
        environment.runs = vec![report_summary("running", None)];
        let (color, spans) = emu_status(&EmuState::Done(emu_inventory(vec![environment])));

        assert_eq!(color, Color::Yellow);
        assert_eq!(span_text(&spans), "1 active · → open");
    }

    #[test]
    fn emu_empty_lines_list_config_path() {
        let lines = emu_empty_lines("~/.config/qol-tray/emu.toml");
        assert_eq!(lines.len(), 2, "lines: {lines:?}");
    }
}
