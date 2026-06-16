use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, IsTerminal, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::commands::emu::{
    emu_config_path, emu_dir, emu_scan, newest_run_detail, EnvironmentStatus, ImageCandidate,
    LastRun, ResolveState, RunDetail,
};
use crate::dev_server::{
    fetch_dev_links, health_ok, post_recompile_current, post_reload_plugins, probe_endpoints,
    web_ok, DevLink, EndpointStatus, WEBSITE_URL,
};
use crate::host_facade;
use crate::poller::Poller;

const LOG_CAP: usize = 2000;
const TICK: Duration = Duration::from_millis(150);
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const EMU_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const LINKS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const DOCTOR_BASE_INTERVAL: Duration = Duration::from_secs(10);
const DOCTOR_CAP_INTERVAL: Duration = Duration::from_secs(60);
const ENDPOINTS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(5);
const CRASH_TAIL: usize = 40;
const ACK_TTL: Duration = Duration::from_secs(6);

pub(crate) enum SessionEnd {
    ChildExited(ExitStatus),
    UserQuit,
    ReloadRequested,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Action {
    ToggleView,
    ToggleKeys,
    Rebuild,
    ReloadSelf,
    Doctor,
    Back,
    Activate,
    Dive,
    Quit,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Follow,
    Filter,
    Copy,
    OpenEmuDir,
    ToggleArch,
    Confirm,
    Ignore,
}

fn action_for(code: KeyCode, mods: KeyModifiers) -> Action {
    if mods.contains(KeyModifiers::CONTROL) {
        return match code {
            KeyCode::Char('r') => Action::Rebuild,
            KeyCode::Char('u') => Action::ReloadSelf,
            KeyCode::Char('c') => Action::Quit,
            _ => Action::Ignore,
        };
    }
    match code {
        KeyCode::Char('l') | KeyCode::Char('L') => Action::ToggleView,
        KeyCode::Char('k') | KeyCode::Char('K') => Action::ToggleKeys,
        KeyCode::Char('d') | KeyCode::Char('D') => Action::Doctor,
        KeyCode::Esc | KeyCode::Left => Action::Back,
        KeyCode::Enter => Action::Activate,
        KeyCode::Right => Action::Dive,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Up => Action::ScrollUp,
        KeyCode::Down => Action::ScrollDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::End | KeyCode::Char('f') => Action::Follow,
        KeyCode::Char('/') => Action::Filter,
        KeyCode::Char('c') | KeyCode::Char('C') => Action::Copy,
        KeyCode::Char('o') | KeyCode::Char('O') => Action::OpenEmuDir,
        KeyCode::Char('t') | KeyCode::Char('T') => Action::ToggleArch,
        KeyCode::Char('a') | KeyCode::Char('A') => Action::Confirm,
        _ => Action::Ignore,
    }
}

fn preserves_arm(action: Action) -> bool {
    matches!(
        action,
        Action::ScrollUp
            | Action::ScrollDown
            | Action::PageUp
            | Action::PageDown
            | Action::Dive
            | Action::Back
            | Action::ToggleView
            | Action::ToggleKeys
            | Action::Follow
    )
}

struct LogRing {
    lines: VecDeque<String>,
}

impl LogRing {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
        }
    }

    fn push(&mut self, line: String) {
        if self.lines.len() == LOG_CAP {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn len(&self) -> usize {
        self.lines.len()
    }
}

fn window_start(len: usize, height: usize, offset: usize) -> usize {
    len.saturating_sub(height.saturating_add(offset))
}

fn clamp_offset(len: usize, height: usize, offset: usize) -> usize {
    offset.min(len.saturating_sub(height))
}

struct OwnedSource {
    child: Child,
    rx: Receiver<String>,
}

struct LogPane {
    ring: LogRing,
    source: Option<OwnedSource>,
}

impl LogPane {
    fn new() -> Self {
        Self {
            ring: LogRing::new(),
            source: None,
        }
    }

    fn attach(&mut self, child: Child, rx: Receiver<String>) {
        self.source = Some(OwnedSource { child, rx });
    }

    fn is_live(&self) -> bool {
        self.source.is_some()
    }

    fn push(&mut self, line: String) {
        self.ring.push(line);
    }

    fn len(&self) -> usize {
        self.ring.len()
    }

    fn drain(&mut self, keep: impl Fn(&str) -> bool) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let mut received = Vec::new();
        while let Ok(line) = source.rx.try_recv() {
            received.push(line);
        }
        for line in received {
            if keep(&line) {
                self.ring.push(line);
            }
        }
    }

    fn stop(&mut self) {
        if let Some(mut source) = self.source.take() {
            let _ = source.child.kill();
            let _ = source.child.wait();
        }
    }

    fn replay(path: &Path) -> Self {
        let mut ring = LogRing::new();
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                ring.push(line.to_string());
            }
        }
        Self { ring, source: None }
    }

    fn poll_finished(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let mut received = Vec::new();
        let mut exited = None;
        if let Some(source) = self.source.as_mut() {
            while let Ok(line) = source.rx.try_recv() {
                received.push(line);
            }
            if let Ok(Some(status)) = source.child.try_wait() {
                while let Ok(line) = source.rx.try_recv() {
                    received.push(line);
                }
                exited = Some(status);
            }
        }
        for line in received {
            if keep(&line) {
                self.ring.push(line);
            }
        }
        match exited {
            Some(status) => {
                self.ring.push(emu_run_line("done", &status.to_string()));
                self.source = None;
                true
            }
            None => false,
        }
    }

    fn stop_graceful(&mut self) {
        let Some(mut source) = self.source.take() else {
            return;
        };
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if matches!(source.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = source.child.kill();
        let _ = source.child.wait();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Dashboard,
    Logs,
    Doctor,
    Plugins,
    Emu,
    EmuDetail,
    Trace,
    Endpoints,
}

#[derive(Clone, Copy)]
enum Row {
    Tray,
    Web,
    Plugins,
    Emu,
    Doctor,
    Logs,
    Trace,
}

const ROWS: [Row; 7] = [
    Row::Tray,
    Row::Web,
    Row::Plugins,
    Row::Emu,
    Row::Doctor,
    Row::Logs,
    Row::Trace,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Health {
    Checking,
    Up,
    Down,
}

struct HealthSnapshot {
    api: bool,
    web: bool,
}

type EmuScanResult = Result<(Vec<EnvironmentStatus>, Vec<ImageCandidate>), String>;

struct Probes {
    health: Poller<HealthSnapshot>,
    emu: Poller<EmuScanResult>,
    links: Poller<Result<Vec<DevLink>, String>>,
    doctor: Poller<Result<DoctorRun, String>>,
    endpoints: Option<Poller<Vec<EndpointStatus>>>,
}

impl Probes {
    fn spawn() -> Self {
        Self {
            health: Poller::spawn(HEALTH_PROBE_INTERVAL, || HealthSnapshot {
                api: health_ok(),
                web: web_ok(),
            }),
            emu: Poller::spawn(EMU_REFRESH_INTERVAL, || {
                emu_scan().map_err(|error| format!("{error:#}"))
            }),
            links: Poller::spawn(LINKS_REFRESH_INTERVAL, || {
                fetch_dev_links().map_err(|error| format!("{error:#}"))
            }),
            doctor: spawn_doctor_probe(),
            endpoints: None,
        }
    }
}

fn spawn_doctor_probe() -> Poller<Result<DoctorRun, String>> {
    Poller::spawn_adaptive(
        DOCTOR_BASE_INTERVAL,
        DOCTOR_CAP_INTERVAL,
        run_doctor_prebuilt,
    )
}

#[derive(Default)]
struct Pokes {
    emu: bool,
    links: bool,
    doctor: bool,
}

fn flush_pokes(dash: &mut Dash, probes: &Probes) {
    if std::mem::take(&mut dash.pokes.emu) {
        probes.emu.poke();
    }
    if std::mem::take(&mut dash.pokes.links) {
        probes.links.poke();
    }
    if std::mem::take(&mut dash.pokes.doctor) {
        probes.doctor.poke();
    }
}

enum EndpointsState {
    Probing,
    Done(Vec<EndpointStatus>),
}

enum EmuState {
    Probing,
    Done(Vec<EnvironmentStatus>),
    Failed(String),
}

enum LinksState {
    Unknown,
    Live(Vec<DevLink>),
    Unreachable,
}

struct EmuDetail {
    id: String,
    info: Vec<Line<'static>>,
    replay: Option<LogPane>,
}

#[derive(Clone, Copy, PartialEq)]
struct DoctorReport {
    ok: usize,
    warn: usize,
    error: usize,
    crash: usize,
}

impl DoctorReport {
    fn divergences(&self) -> usize {
        self.warn + self.error + self.crash
    }
}

struct DoctorPanel {
    last: Option<DoctorRun>,
    last_at_ms: Option<u64>,
    manual: Option<(DoctorMode, Receiver<Result<DoctorRun, String>>)>,
    error: Option<String>,
}

#[derive(Clone, PartialEq)]
struct DoctorRun {
    report: DoctorReport,
    lines: Vec<String>,
}

enum RebuildState {
    Idle,
    Requested(Instant),
    Failed(String),
}

enum Reload {
    Idle,
    Running {
        child: Child,
        rx: Receiver<String>,
        since: Instant,
    },
}

enum ReloadOutcome {
    Pending,
    Ready,
}

struct Dash {
    view: View,
    logs: LogPane,
    scroll_offset: usize,
    health: Health,
    web: Health,
    endpoints: EndpointsState,
    started: Instant,
    rebuild: RebuildState,
    plugin_reload: RebuildState,
    plugin_names: Vec<String>,
    emu: EmuState,
    active_runs: HashMap<String, LogPane>,
    emu_detail: Option<EmuDetail>,
    emu_cursor: usize,
    emu_candidates: Vec<ImageCandidate>,
    log_height: usize,
    cursor: usize,
    doctor: DoctorPanel,
    trace: LogPane,
    trace_unavailable: bool,
    boot_rx: Option<Receiver<String>>,
    keys_hidden: bool,
    filter: String,
    filtering: bool,
    copy_count: String,
    copying: bool,
    copy_ack: Option<(Instant, String)>,
    armed: bool,
    reload: Reload,
    pokes: Pokes,
    links: LinksState,
}

impl Dash {
    fn new(plugin_names: Vec<String>) -> Self {
        Self {
            view: View::Dashboard,
            logs: LogPane::new(),
            scroll_offset: 0,
            health: Health::Checking,
            web: Health::Checking,
            endpoints: EndpointsState::Probing,
            started: Instant::now(),
            rebuild: RebuildState::Idle,
            plugin_reload: RebuildState::Idle,
            plugin_names,
            emu: EmuState::Probing,
            active_runs: HashMap::new(),
            emu_detail: None,
            emu_cursor: 0,
            emu_candidates: Vec::new(),
            log_height: 0,
            cursor: 0,
            doctor: DoctorPanel {
                last: None,
                last_at_ms: None,
                manual: None,
                error: None,
            },
            trace: LogPane::new(),
            trace_unavailable: false,
            boot_rx: None,
            keys_hidden: false,
            filter: String::new(),
            filtering: false,
            copy_count: String::new(),
            copying: false,
            copy_ack: None,
            armed: false,
            reload: Reload::Idle,
            pokes: Pokes::default(),
            links: LinksState::Unknown,
        }
    }

    fn is_reloading(&self) -> bool {
        matches!(self.reload, Reload::Running { .. })
    }

    fn start_doctor(&mut self, mode: DoctorMode) {
        self.doctor.manual = Some((mode, spawn_doctor(mode)));
    }
}

pub(crate) fn run_session(
    child: &mut Child,
    verbose: bool,
    plugins: Vec<String>,
    boot: Option<Receiver<String>>,
) -> Result<SessionEnd> {
    let lines = spawn_forwarders(child);
    if verbose || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return plain_session(child, &lines, boot);
    }
    let mut probes = Probes::spawn();
    let mut dash = Dash::new(plugins);
    dash.boot_rx = boot;
    start_trace(&mut dash);
    let mut terminal = ratatui::init();
    let result = tui_session(&mut terminal, child, &lines, &mut probes, &mut dash);
    ratatui::restore();
    if let Ok(SessionEnd::ChildExited(status)) = &result {
        if !status.success() {
            print_crash_tail(&dash.logs.ring);
        }
    }
    result
}

fn print_crash_tail(logs: &LogRing) {
    let start = logs.len().saturating_sub(CRASH_TAIL);
    for line in logs.lines.iter().skip(start) {
        eprintln!("{line}");
    }
}

fn plain_session(
    child: &mut Child,
    lines: &Receiver<String>,
    boot: Option<Receiver<String>>,
) -> Result<SessionEnd> {
    loop {
        if let Some(rx) = boot.as_ref() {
            while let Ok(line) = rx.try_recv() {
                println!("{line}");
            }
        }
        match lines.recv_timeout(TICK) {
            Ok(line) => println!("{line}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let status = child
                    .wait()
                    .context("failed waiting for qol-tray dev process")?;
                return Ok(SessionEnd::ChildExited(status));
            }
        }
        if let Some(status) = try_wait(child)? {
            while let Ok(line) = lines.try_recv() {
                println!("{line}");
            }
            return Ok(SessionEnd::ChildExited(status));
        }
    }
}

fn tui_session(
    terminal: &mut DefaultTerminal,
    child: &mut Child,
    lines: &Receiver<String>,
    probes: &mut Probes,
    dash: &mut Dash,
) -> Result<SessionEnd> {
    loop {
        while let Ok(line) = lines.try_recv() {
            dash.logs.push(line);
        }
        if let Some(snapshot) = probes.health.latest() {
            apply_health(dash, snapshot);
        }
        match (dash.view == View::Endpoints, probes.endpoints.is_some()) {
            (true, false) => {
                probes.endpoints = Some(Poller::spawn(ENDPOINTS_REFRESH_INTERVAL, probe_endpoints));
            }
            (false, true) => probes.endpoints = None,
            (true, true) | (false, false) => {}
        }
        if let Some(results) = probes.endpoints.as_ref().and_then(|poller| poller.latest()) {
            dash.endpoints = EndpointsState::Done(results);
        }
        if let Some(outcome) = probes.emu.latest() {
            dash.emu = match outcome {
                Ok((statuses, candidates)) => {
                    dash.emu_candidates = candidates;
                    EmuState::Done(statuses)
                }
                Err(error) => EmuState::Failed(error),
            };
        }
        if let Some(outcome) = probes.links.latest() {
            dash.links = match outcome {
                Ok(links) => LinksState::Live(links),
                Err(_) => LinksState::Unreachable,
            };
        }
        let manual_outcome = dash
            .doctor
            .manual
            .as_ref()
            .and_then(|(_, rx)| rx.try_recv().ok());
        if let Some(outcome) = manual_outcome {
            dash.doctor.manual = None;
            apply_doctor_outcome(dash, outcome);
            probes.doctor = spawn_doctor_probe();
        } else if dash.doctor.manual.is_none() {
            if let Some(outcome) = probes.doctor.latest() {
                apply_doctor_outcome(dash, outcome);
            }
        } else {
            let _ = probes.doctor.latest();
        }
        dash.trace.drain(|_| true);
        drain_boot(dash);
        drain_emu_runs(dash);
        if let ReloadOutcome::Ready = poll_reload(dash) {
            stop_trace(dash);
            stop_emu_runs(dash);
            stop_child(child)?;
            return Ok(SessionEnd::ReloadRequested);
        }
        if let Some(status) = try_wait(child)? {
            while let Ok(line) = lines.try_recv() {
                dash.logs.push(line);
            }
            stop_trace(dash);
            stop_emu_runs(dash);
            return Ok(SessionEnd::ChildExited(status));
        }
        flush_pokes(dash, probes);
        terminal.draw(|frame| draw(frame, dash))?;
        if let Some((code, mods)) = poll_key()? {
            if dash.filtering {
                edit_filter(dash, code);
            } else if dash.copying {
                edit_copy(dash, code);
            } else if code == KeyCode::Char(' ') {
                dash.armed = !dash.armed;
            } else if dash.armed && code == KeyCode::Esc {
                dash.armed = false;
            } else {
                let modified = dash.armed;
                match action_for(code, mods) {
                    Action::Quit => {
                        stop_trace(dash);
                        stop_emu_runs(dash);
                        stop_child(child)?;
                        return Ok(SessionEnd::UserQuit);
                    }
                    Action::ReloadSelf => start_reload(dash),
                    action => {
                        apply_action(dash, action, modified);
                        if modified && !preserves_arm(action) {
                            dash.armed = false;
                        }
                    }
                }
            }
        }
    }
}

fn drain_boot(dash: &mut Dash) {
    let mut received = Vec::new();
    if let Some(rx) = dash.boot_rx.as_ref() {
        while let Ok(line) = rx.try_recv() {
            received.push(line);
        }
    }
    for line in received {
        dash.logs.push(line);
    }
}

fn edit_filter(dash: &mut Dash, code: KeyCode) {
    match code {
        KeyCode::Char(c) => dash.filter.push(c),
        KeyCode::Backspace => {
            dash.filter.pop();
        }
        KeyCode::Enter => dash.filtering = false,
        KeyCode::Esc => {
            dash.filter.clear();
            dash.filtering = false;
        }
        _ => {}
    }
}

fn edit_copy(dash: &mut Dash, code: KeyCode) {
    match code {
        KeyCode::Char(c) if c.is_ascii_digit() => dash.copy_count.push(c),
        KeyCode::Backspace => {
            dash.copy_count.pop();
        }
        KeyCode::Enter => finish_copy(dash),
        KeyCode::Esc => {
            dash.copy_count.clear();
            dash.copying = false;
        }
        _ => {}
    }
}

fn finish_copy(dash: &mut Dash) {
    dash.copying = false;
    let count = dash.copy_count.parse::<usize>().ok().filter(|&n| n > 0);
    dash.copy_count.clear();
    let Some(count) = count else {
        return;
    };
    let text = newest_lines(dash, count);
    let message = match host_facade::copy_to_clipboard(&text) {
        Ok(()) => format!("copied {} lines to clipboard", text.lines().count()),
        Err(error) => format!("copy failed: {error}"),
    };
    dash.copy_ack = Some((Instant::now(), message));
}

fn newest_lines(dash: &Dash, count: usize) -> String {
    let ring = match dash.view {
        View::Trace => Some(&dash.trace.ring),
        View::EmuDetail => emu_detail_ring(dash),
        View::Dashboard
        | View::Logs
        | View::Doctor
        | View::Plugins
        | View::Emu
        | View::Endpoints => Some(&dash.logs.ring),
    };
    let Some(ring) = ring else {
        return String::new();
    };
    let filtered: Vec<&String> = ring
        .lines
        .iter()
        .filter(|line| dash.filter.is_empty() || line.contains(&dash.filter))
        .collect();
    let start = filtered.len().saturating_sub(count);
    filtered[start..]
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_ansi(raw: &str) -> String {
    use ansi_to_tui::IntoText;
    let Ok(text) = raw.into_text() else {
        return raw.to_string();
    };
    text.lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
        .collect()
}

fn copy_highlight(dash: &Dash) -> Option<usize> {
    if !dash.copying {
        return None;
    }
    dash.copy_count.parse::<usize>().ok().filter(|&n| n > 0)
}

fn poll_key() -> Result<Option<(KeyCode, KeyModifiers)>> {
    if !event::poll(TICK)? {
        return Ok(None);
    }
    let Event::Key(key) = event::read()? else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(Some((key.code, key.modifiers)))
}

fn apply_action(dash: &mut Dash, action: Action, modified: bool) {
    let page = dash.log_height.max(1);
    match action {
        Action::ToggleView => {
            dash.view = match dash.view {
                View::Logs => View::Dashboard,
                _ => View::Logs,
            };
            dash.scroll_offset = 0;
            dash.filter.clear();
        }
        Action::ToggleKeys => dash.keys_hidden = !dash.keys_hidden,
        Action::Rebuild => {
            trigger_rebuild(dash);
            trigger_reload(dash);
        }
        Action::Doctor => open_doctor(dash),
        Action::Activate => match dash.view {
            View::Dashboard => act_row(dash, modified),
            View::Emu => act_emu(dash, modified),
            View::Logs
            | View::Doctor
            | View::Plugins
            | View::Trace
            | View::Endpoints
            | View::EmuDetail => {}
        },
        Action::Dive => match dash.view {
            View::Dashboard => dive_row(dash),
            View::Emu => open_emu_detail(dash),
            View::Logs
            | View::Doctor
            | View::Plugins
            | View::Trace
            | View::Endpoints
            | View::EmuDetail => {}
        },
        Action::Back => {
            dash.view = if dash.view == View::EmuDetail {
                View::Emu
            } else {
                View::Dashboard
            };
            dash.emu_detail = None;
            dash.scroll_offset = 0;
            dash.filter.clear();
            dash.filtering = false;
        }
        Action::ScrollUp => match dash.view {
            View::Dashboard => dash.cursor = dash.cursor.saturating_sub(1),
            View::Emu => dash.emu_cursor = dash.emu_cursor.saturating_sub(1),
            View::Logs
            | View::Doctor
            | View::Plugins
            | View::Trace
            | View::Endpoints
            | View::EmuDetail => dash.scroll_offset = dash.scroll_offset.saturating_add(1),
        },
        Action::ScrollDown => match dash.view {
            View::Dashboard => dash.cursor = (dash.cursor + 1).min(ROWS.len() - 1),
            View::Emu => {
                let total = emu_env_count(dash) + dash.emu_candidates.len();
                dash.emu_cursor = (dash.emu_cursor + 1).min(total.saturating_sub(1));
            }
            View::Logs
            | View::Doctor
            | View::Plugins
            | View::Trace
            | View::Endpoints
            | View::EmuDetail => dash.scroll_offset = dash.scroll_offset.saturating_sub(1),
        },
        Action::PageUp => dash.scroll_offset = dash.scroll_offset.saturating_add(page),
        Action::PageDown => dash.scroll_offset = dash.scroll_offset.saturating_sub(page),
        Action::Follow => dash.scroll_offset = 0,
        Action::Filter => {
            if matches!(dash.view, View::Logs | View::Trace | View::EmuDetail) {
                dash.filtering = true;
            }
        }
        Action::Copy => {
            if matches!(dash.view, View::Logs | View::Trace | View::EmuDetail) {
                dash.copying = true;
                dash.copy_count.clear();
                dash.scroll_offset = 0;
            }
        }
        Action::OpenEmuDir => {
            if dash.view == View::Emu {
                open_emu_dir();
            }
        }
        Action::ToggleArch => {
            if dash.view == View::Emu {
                if let Some(candidate) = selected_candidate_mut(dash) {
                    candidate.arch = candidate.arch.toggled();
                    candidate.arch_inferred = true;
                }
            }
        }
        Action::Confirm => {
            if dash.view == View::Emu {
                confirm_selected_candidate(dash);
            }
        }
        Action::Quit | Action::ReloadSelf | Action::Ignore => {}
    }
    let len = if dash.view == View::Trace {
        dash.trace.len()
    } else if dash.view == View::EmuDetail {
        emu_detail_ring(dash).map_or(0, LogRing::len)
    } else {
        dash.logs.len()
    };
    dash.scroll_offset = clamp_offset(len, dash.log_height, dash.scroll_offset);
}

fn act_row(dash: &mut Dash, modified: bool) {
    match ROWS[dash.cursor] {
        Row::Tray => {
            if !modified {
                trigger_rebuild(dash);
            }
        }
        Row::Web => {
            if !modified {
                host_facade::open_url(WEBSITE_URL);
            }
        }
        Row::Plugins => {
            if !modified {
                trigger_reload(dash);
            }
        }
        Row::Emu => {
            if !modified {
                open_emu(dash);
            }
        }
        Row::Doctor => {
            if modified {
                dash.start_doctor(DoctorMode::Fix);
            } else {
                dash.start_doctor(DoctorMode::Check);
            }
        }
        Row::Logs | Row::Trace => {}
    }
}

fn dive_row(dash: &mut Dash) {
    match ROWS[dash.cursor] {
        Row::Tray => {}
        Row::Web => open_endpoints(dash),
        Row::Plugins => {
            dash.view = View::Plugins;
            dash.scroll_offset = 0;
        }
        Row::Emu => open_emu(dash),
        Row::Doctor => open_doctor(dash),
        Row::Logs => {
            dash.view = View::Logs;
            dash.scroll_offset = 0;
        }
        Row::Trace => open_trace(dash),
    }
}

fn open_endpoints(dash: &mut Dash) {
    dash.view = View::Endpoints;
    dash.scroll_offset = 0;
}

fn open_emu(dash: &mut Dash) {
    dash.view = View::Emu;
    dash.scroll_offset = 0;
    dash.pokes.emu = true;
}

fn health_state(up: bool) -> Health {
    if up {
        Health::Up
    } else {
        Health::Down
    }
}

fn apply_doctor_outcome(dash: &mut Dash, outcome: Result<DoctorRun, String>) {
    match outcome {
        Ok(run) => {
            dash.doctor.last = Some(run);
            dash.doctor.last_at_ms = Some(now_unix_ms());
            dash.doctor.error = None;
        }
        Err(error) => dash.doctor.error = Some(error),
    }
}

fn apply_health(dash: &mut Dash, snapshot: HealthSnapshot) {
    let was_up = dash.health == Health::Up;
    dash.health = health_state(snapshot.api);
    dash.web = health_state(snapshot.web);
    if !was_up && dash.health == Health::Up {
        dash.pokes.links = true;
        dash.pokes.doctor = true;
    }
}

fn start_trace(dash: &mut Dash) {
    if dash.trace.is_live() {
        return;
    }
    match spawn_trace() {
        Some((child, rx)) => {
            dash.trace.attach(child, rx);
            dash.trace_unavailable = false;
        }
        None => {
            if !dash.trace_unavailable {
                dash.trace.push(
                    "[qol dev] could not start tracer (need python3 + tools/compact_trace.py)"
                        .to_string(),
                );
            }
            dash.trace_unavailable = true;
        }
    }
}

fn open_trace(dash: &mut Dash) {
    dash.view = View::Trace;
    dash.scroll_offset = 0;
    start_trace(dash);
}

fn spawn_trace() -> Option<(Child, Receiver<String>)> {
    let root = crate::workspace::repo_root().ok()?;
    let mut child = Command::new("python3")
        .arg("-u")
        .arg(root.join("tools/compact_trace.py"))
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn stop_trace(dash: &mut Dash) {
    dash.trace.stop();
}

fn emu_env_count(dash: &Dash) -> usize {
    match &dash.emu {
        EmuState::Done(statuses) => statuses.len(),
        EmuState::Probing | EmuState::Failed(_) => 0,
    }
}

fn selected_emu_status(dash: &Dash) -> Option<&EnvironmentStatus> {
    match &dash.emu {
        EmuState::Done(statuses) => statuses.get(dash.emu_cursor),
        EmuState::Probing | EmuState::Failed(_) => None,
    }
}

fn selected_candidate_mut(dash: &mut Dash) -> Option<&mut ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get_mut(index))
}

fn selected_candidate(dash: &Dash) -> Option<&ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get(index))
}

fn is_running(dash: &Dash, id: &str) -> bool {
    dash.active_runs.get(id).is_some_and(LogPane::is_live)
}

fn act_emu(dash: &mut Dash, modified: bool) {
    if let Some((id, ready)) = selected_emu_status(dash)
        .map(|status| (status.id.clone(), status.state == ResolveState::Ready))
    {
        if is_running(dash, &id) {
            fire_emu_down(dash, &id);
        } else if ready {
            let verb = if modified { "check" } else { "up" };
            launch_emu(dash, verb, id);
        }
        return;
    }
    let Some(id) = selected_candidate(dash).map(|candidate| candidate.id.clone()) else {
        return;
    };
    if is_running(dash, &id) {
        fire_emu_down(dash, &id);
    } else {
        launch_emu(dash, "up", id);
    }
}

fn launch_emu(dash: &mut Dash, verb: &'static str, id: String) {
    let mut pane = LogPane::new();
    match spawn_emu_verb(verb, &id) {
        Some((child, rx)) => pane.attach(child, rx),
        None => pane.push(emu_run_line(
            "error",
            &format!("could not launch qol emu {verb} {id}"),
        )),
    }
    dash.active_runs.insert(id, pane);
}

fn emu_run_line(verb: &str, detail: &str) -> String {
    format!("  {verb:<9}{detail}")
}

fn keep_emu_line(line: &str) -> bool {
    let trimmed = line.trim();
    !(trimmed.is_empty() || trimmed.starts_with("qol emu") || trimmed.starts_with("hint:"))
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

fn fire_emu_down(dash: &mut Dash, id: &str) {
    let line = match spawn_emu_verb("down", id) {
        Some((mut child, _)) => {
            let _ = child.wait();
            emu_run_line("down", &format!("sent to {id}"))
        }
        None => emu_run_line("error", &format!("could not send down to {id}")),
    };
    if let Some(pane) = dash.active_runs.get_mut(id) {
        pane.push(line);
    }
}

fn open_emu_dir() {
    let Some(dir) = emu_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    crate::host_facade::open_path(&dir);
}

fn confirm_selected_candidate(dash: &mut Dash) {
    let Some(qemu_img) = crate::commands::emu::find_on_path("qemu-img") else {
        return;
    };
    let Some(emu_toml) = emu_config_path() else {
        return;
    };
    let Some(candidate) = selected_candidate_mut(dash).map(|candidate| candidate.clone()) else {
        return;
    };
    if crate::commands::emu::register_image(&emu_toml, &candidate, &qemu_img).is_ok() {
        dash.pokes.emu = true;
    }
}

fn drain_emu_runs(dash: &mut Dash) {
    let mut finished = false;
    for pane in dash.active_runs.values_mut() {
        if pane.poll_finished(keep_emu_line) {
            finished = true;
        }
    }
    if finished {
        dash.pokes.emu = true;
        dash.pokes.doctor = true;
    }
}

fn stop_emu_runs(dash: &mut Dash) {
    let live: Vec<String> = dash
        .active_runs
        .iter()
        .filter(|(_, pane)| pane.is_live())
        .map(|(id, _)| id.clone())
        .collect();
    for id in live {
        fire_emu_down(dash, &id);
        if let Some(pane) = dash.active_runs.get_mut(&id) {
            pane.stop_graceful();
        }
    }
}

fn open_emu_detail(dash: &mut Dash) {
    let Some(status) = selected_emu_status(dash).cloned() else {
        return;
    };
    let detail = newest_run_detail(&status.id);
    let info = emu_info_lines(&status, detail.as_ref());
    let replay = if dash.active_runs.contains_key(&status.id) {
        None
    } else {
        detail.as_ref().map(|d| LogPane::replay(&d.run_log()))
    };
    dash.emu_detail = Some(EmuDetail {
        id: status.id,
        info,
        replay,
    });
    dash.view = View::EmuDetail;
    dash.scroll_offset = 0;
    dash.filter.clear();
}

fn emu_detail_ring(dash: &Dash) -> Option<&LogRing> {
    let detail = dash.emu_detail.as_ref()?;
    if let Some(pane) = dash.active_runs.get(&detail.id) {
        return Some(&pane.ring);
    }
    detail.replay.as_ref().map(|pane| &pane.ring)
}

fn live_verb(dash: &Dash, id: &str) -> Option<String> {
    let pane = dash.active_runs.get(id)?;
    if !pane.is_live() {
        return None;
    }
    let latest = pane.ring.lines.back()?;
    Some(
        latest
            .split_whitespace()
            .next()
            .unwrap_or("running")
            .to_string(),
    )
}

fn state_color(state: ResolveState) -> Color {
    match state {
        ResolveState::Ready => Color::Green,
        ResolveState::Missing => Color::Yellow,
        ResolveState::Unsupported => Color::Red,
    }
}

fn emu_info_lines(status: &EnvironmentStatus, detail: Option<&RunDetail>) -> Vec<Line<'static>> {
    let color = state_color(status.state);
    let mut head = vec![
        "● ".fg(color).bold(),
        status.state.as_str().fg(color).bold(),
        format!(" · {}", status.backend).fg(Color::DarkGray),
    ];
    if let Some(detail) = detail {
        head.push(format!(" · {}", detail.arch).fg(Color::DarkGray));
    }
    head.extend(last_run_spans(status.last_run.as_ref()));
    let mut lines = vec![Line::from(head)];
    if status.state != ResolveState::Ready {
        lines.push(Line::from(vec![
            "  ".into(),
            status.reason.clone().fg(Color::DarkGray),
        ]));
    }
    match detail {
        Some(detail) => {
            lines.push(info_row("image", &detail.image_path));
            lines.push(info_row("accel", &detail.acceleration));
            lines.push(info_row("run dir", &detail.run_dir.display().to_string()));
        }
        None => lines.push(Line::from("  no runs yet".fg(Color::DarkGray))),
    }
    lines
}

fn info_row(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        format!("  {label:<8} ").fg(Color::White),
        value.to_string().fg(Color::DarkGray),
    ])
}

fn start_reload(dash: &mut Dash) {
    if dash.is_reloading() {
        return;
    }
    match spawn_reload() {
        Some((child, rx)) => {
            dash.logs.push("[qol dev] reloading: qol setup".to_string());
            dash.reload = Reload::Running {
                child,
                rx,
                since: Instant::now(),
            };
        }
        None => dash
            .logs
            .push("[qol dev] reload failed to start".to_string()),
    }
}

fn spawn_reload() -> Option<(Child, Receiver<String>)> {
    let exe = std::env::current_exe().ok()?;
    let root = crate::workspace::repo_root().ok()?;
    let mut child = Command::new(exe)
        .arg("setup")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn poll_reload(dash: &mut Dash) -> ReloadOutcome {
    let mut drained = Vec::new();
    let status = match &mut dash.reload {
        Reload::Idle => return ReloadOutcome::Pending,
        Reload::Running { child, rx, .. } => {
            while let Ok(line) = rx.try_recv() {
                drained.push(line);
            }
            match child.try_wait() {
                Ok(Some(status)) => status,
                _ => {
                    for line in drained {
                        dash.logs.push(line);
                    }
                    return ReloadOutcome::Pending;
                }
            }
        }
    };
    for line in drained {
        dash.logs.push(line);
    }
    dash.reload = Reload::Idle;
    if status.success() {
        return ReloadOutcome::Ready;
    }
    dash.logs
        .push(format!("[qol dev] reload aborted: qol setup {status}"));
    ReloadOutcome::Pending
}

fn trigger_rebuild(dash: &mut Dash) {
    dash.rebuild = match post_recompile_current() {
        Ok(()) => {
            dash.pokes.doctor = true;
            RebuildState::Requested(Instant::now())
        }
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

fn trigger_reload(dash: &mut Dash) {
    dash.plugin_reload = match post_reload_plugins() {
        Ok(()) => {
            dash.pokes.links = true;
            dash.pokes.doctor = true;
            RebuildState::Requested(Instant::now())
        }
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

fn open_doctor(dash: &mut Dash) {
    dash.view = View::Doctor;
    dash.scroll_offset = 0;
    dash.pokes.doctor = true;
}

fn draw(frame: &mut Frame, dash: &mut Dash) {
    let [main, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let accent = frame_accent(dash);
    let [_, body] =
        Layout::vertical([Constraint::Length(TITLE_CAP), Constraint::Min(0)]).areas(main);
    let block = Block::bordered()
        .border_style(Style::new().fg(accent))
        .padding(PANEL_PADDING);
    let inner = block.inner(body);
    frame.render_widget(block, body);
    let content = page_header(frame, dash.view, inner);
    match dash.view {
        View::Dashboard => draw_dashboard(frame, dash, content),
        View::Logs => draw_logs(frame, dash, content),
        View::Doctor => draw_doctor(frame, dash, content),
        View::Plugins => draw_plugins(frame, dash, content),
        View::Emu => draw_emu(frame, dash, content),
        View::EmuDetail => draw_emu_detail(frame, dash, content),
        View::Trace => draw_trace(frame, dash, content),
        View::Endpoints => draw_endpoints(frame, dash, content),
    }
    Sign {
        content: breadcrumb(dash, accent),
    }
    .render(frame, body, accent);
    let status_style = if dash.is_reloading() {
        Style::new().fg(Color::Red).bold()
    } else if dash.armed || dash.filtering || dash.copying {
        Style::new().fg(Color::Yellow).bold()
    } else {
        Style::new().dim()
    };
    frame.render_widget(
        Paragraph::new(status_line(dash)).style(status_style),
        footer,
    );
    draw_keys_hud(frame, dash, inner);
}

fn page_header(frame: &mut Frame, view: View, inner: Rect) -> Rect {
    let Some(desc) = page_description(view) else {
        return inner;
    };
    frame.render_widget(
        Paragraph::new(Line::from(format!("  {desc}").fg(Color::DarkGray))),
        Rect { height: 1, ..inner },
    );
    Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2),
        ..inner
    }
}

fn page_description(view: View) -> Option<&'static str> {
    match view {
        View::Logs => Some("live daemon logs"),
        View::Trace => Some("runtime trace events"),
        View::Doctor => Some("install health checks"),
        View::Plugins => Some("dev-linked plugins"),
        View::Emu => Some("clean-os test envs"),
        View::Endpoints => Some("local service endpoints"),
        View::Dashboard | View::EmuDetail => None,
    }
}

fn breadcrumb(dash: &Dash, accent: Color) -> Line<'static> {
    let trail: Vec<String> = match dash.view {
        View::Dashboard => Vec::new(),
        View::Logs => vec!["logs".to_string()],
        View::Trace => vec!["trace".to_string()],
        View::Doctor => vec!["doctor".to_string()],
        View::Plugins => vec!["plugins".to_string()],
        View::Emu => vec!["emu".to_string()],
        View::Endpoints => vec!["endpoints".to_string()],
        View::EmuDetail => {
            let id = dash
                .emu_detail
                .as_ref()
                .map(|detail| detail.id.clone())
                .unwrap_or_default();
            vec!["emu".to_string(), id]
        }
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    if trail.is_empty() {
        spans.push("qol dev".fg(accent).bold());
    } else {
        spans.push("qol dev".fg(Color::DarkGray));
        let last = trail.len() - 1;
        for (index, segment) in trail.into_iter().enumerate() {
            spans.push(" · ".fg(Color::DarkGray));
            if index == last {
                spans.push(segment.fg(accent).bold());
            } else {
                spans.push(segment.fg(Color::DarkGray));
            }
        }
    }
    if dash.is_reloading() {
        spans.push(" · RELOADING".fg(Color::Red).bold());
    } else if dash.armed {
        spans.push(" · ARMED".fg(Color::Yellow).bold());
    }
    Line::from(spans)
}

fn keys_legend(view: View) -> Vec<(&'static str, &'static str)> {
    let view_keys: &[(&'static str, &'static str)] = match view {
        View::Dashboard => &[
            ("↑/↓", "move"),
            ("enter", "act on row"),
            ("→ / ←", "dive · back"),
            ("space", "arm, then enter"),
            ("l · d", "logs · doctor"),
        ],
        View::Emu => &[
            ("↑/↓", "select emu"),
            ("enter", "boot · stop"),
            ("→", "detail · log"),
            ("space", "arm: checks"),
            ("o", "open emu dir"),
            ("t", "toggle arch"),
            ("a", "add image"),
            ("←", "back"),
        ],
        View::Logs | View::Trace | View::EmuDetail => &[
            ("↑/↓", "scroll"),
            ("f / end", "follow tail"),
            ("/", "filter"),
            ("c", "copy last N"),
            ("←", "back"),
        ],
        View::Doctor => &[
            ("d", "refresh checks"),
            ("space", "arm: raw output"),
            ("↑/↓", "scroll"),
            ("←", "back"),
        ],
        View::Plugins | View::Endpoints => &[("↑/↓", "scroll"), ("←", "back")],
    };
    let mut keys = view_keys.to_vec();
    keys.extend([
        ("ctrl+r", "rebuild tray+plugins"),
        ("ctrl+u", "reload qol dev"),
        ("q", "quit"),
    ]);
    keys
}

fn draw_keys_hud(frame: &mut Frame, dash: &Dash, area: Rect) {
    if dash.keys_hidden {
        return;
    }
    let keys = keys_legend(dash.view);
    let lines: Vec<Line> = keys
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                format!(" {key:<9}").fg(Color::White).bold(),
                format!("{desc} ").fg(Color::DarkGray),
            ])
        })
        .collect();
    let content_width = keys
        .iter()
        .map(|(key, desc)| 1 + key.chars().count().max(9) + desc.chars().count() + 1)
        .max()
        .unwrap_or(0) as u16;
    let width = (content_width + 2).min(area.width);
    let height = (lines.len() as u16 + SignBox::CHROME_ROWS).min(area.height);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width),
        y: area.y,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    SignBox {
        title: "keys · k",
        rows: lines,
    }
    .render(frame, rect, frame_accent(dash));
}

struct SignBox<'a> {
    title: &'a str,
    rows: Vec<Line<'a>>,
}

impl SignBox<'_> {
    const CHROME_ROWS: u16 = 4;

    fn capacity(height: u16) -> usize {
        height.saturating_sub(Self::CHROME_ROWS) as usize
    }

    fn render(self, frame: &mut Frame, area: Rect, accent: Color) {
        let body = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(1),
            ..area
        };
        let block = Block::bordered().border_style(Style::new().fg(accent));
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let rows_area = Rect {
            y: inner.y + 1,
            height: inner.height.saturating_sub(1),
            ..inner
        };
        frame.render_widget(Paragraph::new(self.rows), rows_area);
        Sign {
            content: Line::from(self.title.to_string().fg(accent).bold()),
        }
        .render(frame, body, accent);
    }
}

const ITEM_GAP: u16 = 1;

fn space_rows(rows: Vec<Line>, gap: u16) -> Vec<Line> {
    if gap == 0 || rows.len() <= 1 {
        return rows;
    }
    let last = rows.len() - 1;
    let mut spaced = Vec::with_capacity(rows.len() + last * gap as usize);
    for (index, row) in rows.into_iter().enumerate() {
        spaced.push(row);
        if index != last {
            spaced.extend((0..gap).map(|_| Line::from("")));
        }
    }
    spaced
}

fn spaced_height(items: usize, gap: u16) -> u16 {
    items as u16 + items.saturating_sub(1) as u16 * gap
}

fn list_capacity(height: u16) -> usize {
    (height as usize + ITEM_GAP as usize) / (1 + ITEM_GAP as usize)
}

fn view_content(frame: &mut Frame, area: Rect, lines: Vec<Line>) {
    frame.render_widget(Paragraph::new(space_rows(lines, ITEM_GAP)), area);
}

fn status_line(dash: &Dash) -> String {
    if let Reload::Running { since, .. } = &dash.reload {
        return format!(
            " RELOADING qol dev · rebuilding · {}",
            format_duration(since.elapsed())
        );
    }
    if dash.copying {
        return format!(
            " copy last N: {}_ · enter copy · esc cancel",
            dash.copy_count
        );
    }
    if dash.filtering {
        return format!(" filter: {}_ · enter apply · esc cancel", dash.filter);
    }
    if dash.armed {
        return armed_status(dash);
    }
    if let Some((at, message)) = &dash.copy_ack {
        if at.elapsed() < ACK_TTL {
            return format!(" {message}");
        }
    }
    match dash.view {
        View::Dashboard => {
            let hints = match ROWS[dash.cursor] {
                Row::Tray => "enter rebuild tray",
                Row::Web => "enter open site · → endpoints",
                Row::Plugins => "enter reload · → list",
                Row::Emu => "enter open · → open",
                Row::Doctor => "enter run · → steps",
                Row::Logs => "→ open",
                Row::Trace => "→ open",
            };
            format!(" {hints}")
        }
        View::Emu => {
            let selected_id = selected_emu_status(dash)
                .map(|status| status.id.clone())
                .or_else(|| selected_candidate(dash).map(|candidate| candidate.id.clone()));
            match selected_id {
                Some(id) if is_running(dash, &id) => {
                    format!(" {id} running · enter stop · → log")
                }
                _ => " enter boots the selected emu · → detail".to_string(),
            }
        }
        View::EmuDetail => match (dash.emu_detail.as_ref(), dash.filter.is_empty()) {
            (Some(detail), true) => format!(" {}", detail.id),
            (Some(detail), false) => format!(" {} · filter: {}", detail.id, dash.filter),
            (None, _) => String::new(),
        },
        View::Logs | View::Trace => {
            if dash.filter.is_empty() {
                String::new()
            } else {
                format!(" filter: {}", dash.filter)
            }
        }
        View::Doctor | View::Plugins | View::Endpoints => String::new(),
    }
}

fn armed_status(dash: &Dash) -> String {
    match dash.view {
        View::Dashboard => {
            let hint = match ROWS[dash.cursor] {
                Row::Doctor => "enter fix",
                Row::Tray | Row::Web | Row::Plugins | Row::Emu | Row::Logs | Row::Trace => {
                    "no armed action"
                }
            };
            format!(" ARMED · {hint} · space/esc cancel ")
        }
        View::Doctor => " RAW · space friendly · esc done ".to_string(),
        View::Emu => " ARMED · enter check · space/esc cancel ".to_string(),
        View::Logs | View::Trace | View::Plugins | View::Endpoints | View::EmuDetail => {
            " ARMED · space/esc cancel ".to_string()
        }
    }
}

fn draw_dashboard(frame: &mut Frame, dash: &Dash, area: Rect) {
    let (tray_color, tray_value) = tray_status(dash);
    let (web_color, web_value) = web_status(dash.web);
    let (plugins_color, plugins_value) =
        plugins_status(&dash.plugin_reload, dash.plugin_names.len(), &dash.links);
    let (emu_color, emu_value) = emu_status(&dash.emu);
    let (doctor_color, doctor_value) = doctor_status(&dash.doctor, now_unix_ms());

    let rows = vec![
        dash_row(dash.cursor == 0, tray_color, "tray", tray_value),
        dash_row(dash.cursor == 1, web_color, "web", web_value),
        dash_row(dash.cursor == 2, plugins_color, "plugins", plugins_value),
        dash_row(dash.cursor == 3, emu_color, "emu", emu_value),
        dash_row(dash.cursor == 4, doctor_color, "doctor", doctor_value),
        dash_row(
            dash.cursor == 5,
            Color::DarkGray,
            "logs",
            vec![format!("{} buffered", dash.logs.len()).fg(Color::DarkGray)],
        ),
        dash_row(
            dash.cursor == 6,
            Color::DarkGray,
            "trace",
            trace_value(dash),
        ),
    ];

    view_content(frame, area, rows);
}

struct Sign {
    content: Line<'static>,
}

impl Sign {
    fn render(self, frame: &mut Frame, body: Rect, accent: Color) {
        let span = self.content.width() as u16 + 2;
        let width = span + 2;
        if width + 2 > body.width {
            return;
        }
        let x = body.x + (body.width - width) / 2;
        let bar = "─".repeat(span as usize);
        render_overlay(
            frame,
            x,
            body.y.saturating_sub(1),
            Line::from(format!("╭{bar}╮").fg(accent)),
        );
        let mut middle = vec!["┤ ".fg(accent)];
        middle.extend(self.content.spans);
        middle.push(" ├".fg(accent));
        render_overlay(frame, x, body.y, Line::from(middle));
        render_overlay(
            frame,
            x,
            body.y + 1,
            Line::from(format!("╰{bar}╯").fg(accent)),
        );
    }
}

fn render_overlay(frame: &mut Frame, x: u16, y: u16, line: Line<'static>) {
    let width = line.width() as u16;
    frame.render_widget(Paragraph::new(line), Rect::new(x, y, width, 1));
}

fn tray_status(dash: &Dash) -> (Color, Vec<Span<'static>>) {
    let (text, color) = match dash.health {
        Health::Checking => ("starting", Color::Yellow),
        Health::Up => ("running", Color::Green),
        Health::Down => ("down", Color::Red),
    };
    let mut value = vec![
        text.fg(color).bold(),
        format!(" · up {}", format_duration(dash.started.elapsed())).fg(Color::DarkGray),
    ];
    if dash.health == Health::Up {
        value.push(" · api ✓".fg(Color::Green));
    }
    match &dash.rebuild {
        RebuildState::Requested(at) if at.elapsed() < ACK_TTL => {
            value.push(" · rebuild sent".fg(Color::Yellow))
        }
        RebuildState::Idle | RebuildState::Requested(_) => {}
        RebuildState::Failed(error) => {
            value.push(" · rebuild ".fg(Color::DarkGray));
            value.push("failed".fg(Color::Red).bold());
            value.push(format!(" · {error}").fg(Color::DarkGray));
        }
    }
    (color, value)
}

fn web_status(web: Health) -> (Color, Vec<Span<'static>>) {
    match web {
        Health::Checking => (Color::Yellow, vec!["checking".fg(Color::Yellow)]),
        Health::Up => (
            Color::Green,
            vec![
                "up".fg(Color::Green).bold(),
                " · localhost:42700".fg(Color::DarkGray),
            ],
        ),
        Health::Down => (Color::Red, vec!["down".fg(Color::Red).bold()]),
    }
}

fn dash_row(selected: bool, color: Color, label: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let caret: Span<'static> = if selected {
        "▸ ".fg(Color::Green).bold()
    } else {
        "  ".into()
    };
    let label_span = if selected {
        format!(" {label:<8} ").fg(Color::White).bold()
    } else {
        format!(" {label:<8} ").fg(Color::DarkGray)
    };
    let mut spans: Vec<Span<'static>> = vec![caret, "●".fg(color).bold(), label_span];
    spans.extend(value);
    Line::from(spans)
}

fn frame_accent(dash: &Dash) -> Color {
    if dash.is_reloading() {
        Color::Red
    } else if dash.armed {
        Color::Yellow
    } else {
        Color::Green
    }
}

const PANEL_PADDING: Padding = Padding {
    left: 1,
    right: 1,
    top: 2,
    bottom: 1,
};

const TITLE_CAP: u16 = 1;

fn plugins_status(
    state: &RebuildState,
    boot_count: usize,
    links: &LinksState,
) -> (Color, Vec<Span<'static>>) {
    let (live_color, mut value) = match links {
        LinksState::Live(links) => {
            let stale = links.iter().filter(|link| link.needs_rebuild).count();
            if stale > 0 {
                (
                    Color::Yellow,
                    vec![
                        format!("{} linked", links.len()).fg(Color::Green),
                        format!(" · {stale} stale").fg(Color::Yellow).bold(),
                    ],
                )
            } else {
                (
                    Color::Green,
                    vec![format!("{} linked", links.len()).fg(Color::Green)],
                )
            }
        }
        LinksState::Unknown => (
            Color::Green,
            vec![format!("{boot_count} linked").fg(Color::DarkGray)],
        ),
        LinksState::Unreachable => (
            Color::Yellow,
            vec![
                format!("{boot_count} linked").fg(Color::DarkGray),
                " · api down".fg(Color::DarkGray),
            ],
        ),
    };
    let color = match state {
        RebuildState::Requested(at) if at.elapsed() < ACK_TTL => {
            value.push(" · reload sent".fg(Color::Yellow));
            live_color
        }
        RebuildState::Failed(error) => {
            value.push(" · reload ".fg(Color::DarkGray));
            value.push("failed".fg(Color::Red).bold());
            value.push(format!(" · {error}").fg(Color::DarkGray));
            Color::Red
        }
        RebuildState::Idle | RebuildState::Requested(_) => live_color,
    };
    (color, value)
}

fn emu_status(state: &EmuState) -> (Color, Vec<Span<'static>>) {
    let statuses = match state {
        EmuState::Probing => {
            return (
                Color::Yellow,
                vec![
                    "scanning".fg(Color::Yellow).bold(),
                    " · → open".fg(Color::DarkGray),
                ],
            )
        }
        EmuState::Done(statuses) => statuses,
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
    if statuses.is_empty() {
        return (
            Color::Yellow,
            vec![
                "no envs".fg(Color::Yellow).bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    let ready = statuses
        .iter()
        .filter(|status| status.state == ResolveState::Ready)
        .count();
    let missing = statuses
        .iter()
        .filter(|status| status.state == ResolveState::Missing)
        .count();
    let unsupported = statuses
        .iter()
        .filter(|status| status.state == ResolveState::Unsupported)
        .count();
    if ready > 0 {
        return (
            Color::Green,
            vec![
                format!("{} envs · {ready} ready", statuses.len())
                    .fg(Color::Green)
                    .bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    let color = if unsupported == statuses.len() {
        Color::Red
    } else {
        Color::Yellow
    };
    (
        color,
        vec![
            format!(
                "{} envs · {missing} missing · {unsupported} unsupported",
                statuses.len()
            )
            .fg(color)
            .bold(),
            " · → open".fg(Color::DarkGray),
        ],
    )
}

fn draw_plugins(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let entries = plugin_view_lines(dash);
    if entries.is_empty() {
        view_content(frame, area, vec![Line::from("  no dev-linked plugins")]);
        return;
    }
    let total = entries.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = entries.into_iter().skip(start).take(height).collect();
    view_content(frame, area, visible);
}

fn plugin_view_lines(dash: &Dash) -> Vec<Line<'static>> {
    match &dash.links {
        LinksState::Live(links) => links.iter().map(plugin_link_line).collect(),
        LinksState::Unknown | LinksState::Unreachable => dash
            .plugin_names
            .iter()
            .map(|name| {
                Line::from(vec![
                    "  ".into(),
                    "●".fg(Color::DarkGray).bold(),
                    format!(" {name}").fg(Color::White),
                    " · link state unknown".fg(Color::DarkGray),
                ])
            })
            .collect(),
    }
}

fn plugin_link_line(link: &DevLink) -> Line<'static> {
    if link.needs_rebuild {
        return Line::from(vec![
            "  ".into(),
            "●".fg(Color::Yellow).bold(),
            format!(" {}", link.name).fg(Color::White),
            " · stale · ".fg(Color::Yellow),
            link.rebuild_reason.clone().fg(Color::DarkGray),
        ]);
    }
    Line::from(vec![
        "  ".into(),
        "●".fg(Color::Green).bold(),
        format!(" {}", link.name).fg(Color::White),
        " · dev-linked".fg(Color::DarkGray),
    ])
}

fn emu_empty_lines(config: &str) -> Vec<Line<'static>> {
    vec![
        Line::from("  no emus found".fg(Color::DarkGray)),
        Line::from(vec![
            "  config ".fg(Color::DarkGray),
            config.to_string().fg(Color::White),
        ]),
    ]
}

fn candidate_row_label(arch: crate::commands::emu::GuestArch, arch_inferred: bool) -> String {
    if arch_inferred {
        format!("needs arch · {}", arch.as_str())
    } else {
        format!("needs arch · {} (host default)", arch.as_str())
    }
}

fn candidate_line(candidate: &ImageCandidate, selected: bool) -> Line<'static> {
    let caret: Span<'static> = if selected {
        "▸ ".fg(Color::Green).bold()
    } else {
        "  ".into()
    };
    let id_span = if selected {
        candidate.id.clone().fg(Color::White).bold()
    } else {
        candidate.id.clone().fg(Color::White)
    };
    Line::from(vec![
        caret,
        "○ ".fg(Color::DarkGray),
        id_span,
        format!(
            "  {}",
            candidate_row_label(candidate.arch, candidate.arch_inferred)
        )
        .fg(Color::DarkGray),
    ])
}

fn draw_emu(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let lines = match &dash.emu {
        EmuState::Probing => vec![Line::from("  scanning emus".fg(Color::Yellow))],
        EmuState::Done(statuses) if statuses.is_empty() => {
            let config = emu_config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "~/.config/qol-tray/emu.toml".to_string());
            emu_empty_lines(&config)
        }
        EmuState::Done(statuses) => statuses
            .iter()
            .enumerate()
            .flat_map(|(index, status)| {
                let selected = index == dash.emu_cursor;
                let color = state_color(status.state);
                let caret: Span<'static> = if selected {
                    "▸ ".fg(Color::Green).bold()
                } else {
                    "  ".into()
                };
                let id_span = if selected {
                    status.id.clone().fg(Color::White).bold()
                } else {
                    status.id.clone().fg(Color::White)
                };
                let mut header = vec![
                    caret,
                    "● ".fg(color).bold(),
                    id_span,
                    format!("  {}", status.state.as_str()).fg(color).bold(),
                    format!(" · {}", status.backend).fg(Color::DarkGray),
                ];
                match live_verb(dash, &status.id) {
                    Some(verb) => {
                        header.push(format!(" · {verb}").fg(Color::Yellow).bold());
                        header.push(" · → log".fg(Color::DarkGray));
                    }
                    None => header.extend(last_run_spans(status.last_run.as_ref())),
                }
                let mut entry = vec![Line::from(header)];
                if status.state != ResolveState::Ready {
                    entry.push(Line::from(vec![
                        "    ".into(),
                        status.reason.clone().fg(Color::DarkGray),
                    ]));
                }
                entry
            })
            .collect(),
        EmuState::Failed(error) => vec![Line::from(vec![
            "  registry error ".fg(Color::Red).bold(),
            error.clone().fg(Color::DarkGray),
        ])],
    };
    let mut lines = lines;
    let env_count = emu_env_count(dash);
    for (index, candidate) in dash.emu_candidates.iter().enumerate() {
        lines.push(candidate_line(
            candidate,
            env_count + index == dash.emu_cursor,
        ));
    }
    let total = lines.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();
    view_content(frame, area, visible);
}

fn draw_emu_detail(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let accent = frame_accent(dash);
    let Some((id, info)) = dash
        .emu_detail
        .as_ref()
        .map(|detail| (detail.id.clone(), detail.info.clone()))
    else {
        return;
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
        return;
    }
    let log_area = Rect {
        y: area.y + used,
        height: area.height - used,
        ..area
    };
    let highlight = copy_highlight(dash);
    if let Some(pane) = dash.active_runs.get(&id) {
        draw_run_log(
            frame,
            log_area,
            &pane.ring,
            &dash.filter,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            accent,
            highlight,
        );
        return;
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
            &dash.filter,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            accent,
            highlight,
        ),
        None => view_content(
            frame,
            log_area,
            vec![Line::from(
                "  no run.log yet · boot to create one".fg(Color::DarkGray),
            )],
        ),
    }
}

fn last_run_spans(last_run: Option<&LastRun>) -> Vec<Span<'static>> {
    let Some(run) = last_run else {
        return Vec::new();
    };
    let color = match run.status.as_str() {
        "pass" => Color::Green,
        "failed" => Color::Red,
        "running" => Color::Yellow,
        _ => Color::DarkGray,
    };
    vec![
        " · ".fg(Color::DarkGray),
        run.status.clone().fg(color),
        format!(" {}", relative_age(now_unix_ms(), run.finished_at_unix_ms)).fg(Color::DarkGray),
    ]
}

fn relative_age(now_ms: u64, then_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn doctor_status(panel: &DoctorPanel, now_ms: u64) -> (Color, Vec<Span<'static>>) {
    if let Some((mode, _)) = &panel.manual {
        return (Color::Yellow, vec![mode.gerund().fg(Color::Yellow)]);
    }
    let Some(run) = &panel.last else {
        let detail = panel
            .error
            .clone()
            .unwrap_or_else(|| "waiting for first check".to_string());
        return (Color::Yellow, vec![detail.fg(Color::DarkGray)]);
    };
    let report = run.report;
    let (color, mut value) = if report.divergences() == 0 {
        (
            Color::Green,
            vec![
                "all good".fg(Color::Green).bold(),
                format!(" · {} checks", report.ok).fg(Color::DarkGray),
            ],
        )
    } else {
        let color = if report.error + report.crash > 0 {
            Color::Red
        } else {
            Color::Yellow
        };
        (
            color,
            vec![
                format!("{} divergences", report.divergences())
                    .fg(color)
                    .bold(),
                format!(
                    " · {} warn · {} err",
                    report.warn,
                    report.error + report.crash
                )
                .fg(Color::DarkGray),
            ],
        )
    };
    if let Some(at) = panel.last_at_ms {
        value.push(format!(" · {}", relative_age(now_ms, at)).fg(Color::DarkGray));
    }
    if panel.error.is_some() {
        value.push(" · probe failed".fg(Color::DarkGray));
    }
    (color, value)
}

fn draw_logs(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let highlight = copy_highlight(dash);
    let (rows, _) = stream_rows(
        &dash.logs.ring,
        &dash.filter,
        &mut dash.scroll_offset,
        &mut dash.log_height,
        area.height as usize,
        highlight,
        area.width as usize,
    );
    frame.render_widget(Paragraph::new(rows), area);
}

fn draw_trace(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    if dash.trace.ring.lines.is_empty() && dash.filter.is_empty() {
        view_content(frame, area, vec![Line::from("  waiting for trace events")]);
        return;
    }
    let highlight = copy_highlight(dash);
    let (rows, _) = stream_rows(
        &dash.trace.ring,
        &dash.filter,
        &mut dash.scroll_offset,
        &mut dash.log_height,
        area.height as usize,
        highlight,
        area.width as usize,
    );
    frame.render_widget(Paragraph::new(rows), area);
}

#[allow(clippy::too_many_arguments)]
fn stream_rows<'a>(
    ring: &'a LogRing,
    filter: &str,
    scroll_offset: &mut usize,
    log_height: &mut usize,
    height: usize,
    highlight_tail: Option<usize>,
    inner_width: usize,
) -> (Vec<Line<'a>>, usize) {
    *log_height = height;
    let filtered: Vec<&'a String> = ring
        .lines
        .iter()
        .filter(|line| filter.is_empty() || line.contains(filter))
        .collect();
    let total = filtered.len();
    *scroll_offset = clamp_offset(total, height, *scroll_offset);
    let start = window_start(total, height, *scroll_offset);
    let highlight_from = highlight_tail.map(|n| total.saturating_sub(n));
    let rows = filtered
        .into_iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, line)| {
            let styled = styled_line(line);
            match highlight_from {
                Some(from) if index >= from => highlight_bar(styled, inner_width),
                _ => styled,
            }
        })
        .collect();
    (rows, total)
}

#[allow(clippy::too_many_arguments)]
fn draw_run_log(
    frame: &mut Frame,
    area: Rect,
    ring: &LogRing,
    filter: &str,
    scroll_offset: &mut usize,
    log_height: &mut usize,
    accent: Color,
    highlight_tail: Option<usize>,
) {
    let height = SignBox::capacity(area.height);
    let inner_width = area.width.saturating_sub(2) as usize;
    let (rows, total) = stream_rows(
        ring,
        filter,
        scroll_offset,
        log_height,
        height,
        highlight_tail,
        inner_width,
    );
    let title = format!("run.log · {}", list_status(total, *scroll_offset));
    SignBox {
        title: &title,
        rows,
    }
    .render(frame, area, accent);
}

fn highlight_bar(line: Line<'_>, inner_width: usize) -> Line<'_> {
    let pad = inner_width.saturating_sub(line.width());
    let mut spans = line.spans;
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    Line::from(spans).style(Style::new().bg(Color::Rgb(38, 44, 74)))
}

fn trace_value(dash: &Dash) -> Vec<Span<'static>> {
    if dash.trace.is_live() {
        return vec![format!("{} lines", dash.trace.len()).fg(Color::DarkGray)];
    }
    if dash.trace_unavailable {
        return vec!["tracer unavailable".fg(Color::DarkGray)];
    }
    vec!["idle · → open".fg(Color::DarkGray)]
}

fn draw_endpoints(frame: &mut Frame, dash: &Dash, area: Rect) {
    let lines: Vec<Line> = match &dash.endpoints {
        EndpointsState::Probing => vec![Line::from("  probing endpoints".fg(Color::DarkGray))],
        EndpointsState::Done(items) => items.iter().map(endpoint_line).collect(),
    };
    view_content(frame, area, lines);
}

fn endpoint_line(status: &EndpointStatus) -> Line<'static> {
    let (symbol, color) = if status.ok {
        ("✓", Color::Green)
    } else {
        ("✗", Color::Red)
    };
    Line::from(vec![
        format!("  {symbol} ").fg(color).bold(),
        format!("{:<8}", status.label).fg(Color::White),
        format!("  {}", status.url).fg(Color::DarkGray),
    ])
}

fn draw_doctor(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let lines = doctor_view_lines(&dash.doctor);
    if lines.is_empty() {
        let message = match &dash.doctor.manual {
            Some((mode, _)) => mode.progress_message(),
            None => "  no checks reported · press d to run",
        };
        view_content(frame, area, vec![Line::from(message)]);
        return;
    }
    let total = lines.len();
    let (start, height) = list_window(dash, area, total);
    let render = if dash.armed {
        styled_doctor_line
    } else {
        friendly_doctor_line
    };
    let visible: Vec<Line> = lines
        .iter()
        .skip(start)
        .take(height)
        .map(|line| render(line))
        .collect();
    view_content(frame, area, visible);
}

fn doctor_view_lines(panel: &DoctorPanel) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(error) = &panel.error {
        lines.push(format!("[ERR] doctor: {error}"));
    }
    if let Some(run) = &panel.last {
        lines.extend(run.lines.iter().cloned());
    }
    lines
}

fn list_window(dash: &mut Dash, area: Rect, total: usize) -> (usize, usize) {
    let height = list_capacity(area.height);
    dash.log_height = height;
    dash.scroll_offset = clamp_offset(total, height, dash.scroll_offset);
    (window_start(total, height, dash.scroll_offset), height)
}

fn list_status(total: usize, scroll_offset: usize) -> String {
    let mode = if scroll_offset == 0 {
        "follow"
    } else {
        "scroll"
    };
    format!("{total} lines · {mode}")
}

fn doctor_line_style(raw: &str) -> Option<(&'static str, Color)> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with("[OK]") {
        return Some(("✓", Color::Green));
    }
    if trimmed.starts_with("[WARN]") {
        return Some(("▲", Color::Yellow));
    }
    if trimmed.starts_with("[ERR]") {
        return Some(("✗", Color::Red));
    }
    if trimmed.starts_with("[CRASH]") {
        return Some(("✗", Color::Magenta));
    }
    None
}

fn styled_doctor_line(raw: &str) -> Line<'static> {
    let Some((symbol, color)) = doctor_line_style(raw) else {
        return Line::from(format!("  {}", raw.trim_start()));
    };
    let rest = raw
        .trim_start()
        .split_once(']')
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .trim_start();
    Line::from(vec![
        format!("  {symbol} ").fg(color).bold(),
        rest.to_string().into(),
    ])
}

fn friendly_doctor_line(raw: &str) -> Line<'static> {
    let Some((symbol, color)) = doctor_line_style(raw) else {
        return Line::from(format!("  {}", raw.trim_start()));
    };
    let rest = raw
        .trim_start()
        .split_once(']')
        .map(|(_, rest)| rest.trim_start())
        .unwrap_or("");
    let Some((id, message)) = rest.split_once(": ") else {
        return Line::from(vec![
            format!("  {symbol} ").fg(color).bold(),
            rest.to_string().into(),
        ]);
    };
    let head = format!("  {symbol} {}", humanize_check_id(id));
    if symbol == "✓" {
        return Line::from(head.fg(color).bold());
    }
    let detail = message
        .split_once('\u{2014}')
        .map_or(message, |(_, tail)| tail)
        .trim();
    Line::from(vec![
        format!("{head} - ").fg(color).bold(),
        detail.to_string().into(),
    ])
}

fn humanize_check_id(id: &str) -> String {
    let spaced = id.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

fn styled_line(raw: &str) -> Line<'_> {
    use ansi_to_tui::IntoText;
    let Ok(text) = raw.into_text() else {
        return Line::from(raw);
    };
    text.lines
        .into_iter()
        .next()
        .unwrap_or_else(|| Line::from(raw))
}

fn format_duration(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let (minutes, seconds) = (total / 60, total % 60);
    if minutes >= 60 {
        return format!("{}h{:02}m", minutes / 60, minutes % 60);
    }
    format!("{minutes}m{seconds:02}s")
}

fn try_wait(child: &mut Child) -> Result<Option<ExitStatus>> {
    child
        .try_wait()
        .context("failed polling qol-tray dev process")
}

fn stop_child(child: &mut Child) -> Result<()> {
    host_facade::stop_qol_tray()?;
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if try_wait(child)?.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    child.wait().context("failed to reap qol-tray after kill")?;
    Ok(())
}

fn spawn_forwarders(child: &mut Child) -> Receiver<String> {
    let (tx, rx) = channel();
    if let Some(stdout) = child.stdout.take() {
        spawn_forwarder(stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_forwarder(stderr, tx);
    }
    rx
}

fn spawn_forwarder(reader: impl Read + Send + 'static, tx: Sender<String>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => return,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if tx.send(line.to_string()).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = tx.send(format!("[qol dev] log stream error: {error}"));
                    return;
                }
            }
        }
    });
}

#[derive(Clone, Copy)]
enum DoctorMode {
    Check,
    Fix,
}

impl DoctorMode {
    fn arg(self) -> &'static str {
        match self {
            DoctorMode::Check => "check",
            DoctorMode::Fix => "fix",
        }
    }

    fn gerund(self) -> &'static str {
        match self {
            DoctorMode::Check => "checking",
            DoctorMode::Fix => "fixing",
        }
    }

    fn progress_message(self) -> &'static str {
        match self {
            DoctorMode::Check => "  running checks",
            DoctorMode::Fix => "  applying fixes",
        }
    }
}

fn spawn_doctor(mode: DoctorMode) -> Receiver<Result<DoctorRun, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_doctor(mode));
    });
    rx
}

fn run_doctor(mode: DoctorMode) -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    build_doctor(&root)?;
    run_doctor_binary(&doctor_binary(&root), &root, mode)
}

fn run_doctor_prebuilt() -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let binary = doctor_binary(&root);
    if !binary.exists() {
        return Err("doctor binary not built · press d".to_string());
    }
    run_doctor_binary(&binary, &root, DoctorMode::Check)
}

fn doctor_binary(root: &std::path::Path) -> std::path::PathBuf {
    root.join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray-doctor"))
}

fn run_doctor_binary(
    binary: &std::path::Path,
    root: &std::path::Path,
    mode: DoctorMode,
) -> Result<DoctorRun, String> {
    let output = Command::new(binary)
        .current_dir(root)
        .arg(mode.arg())
        .output()
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let report =
        parse_doctor_summary(&text).ok_or_else(|| "could not read doctor summary".to_string())?;
    Ok(DoctorRun {
        report,
        lines: doctor_lines(&text, mode),
    })
}

fn build_doctor(root: &std::path::Path) -> Result<(), String> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "qol-tray",
            "--features",
            "dev",
            "--bin",
            "qol-tray-doctor",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr
        .lines()
        .rfind(|line| line.contains("error"))
        .unwrap_or("no cargo error line captured");
    Err(format!("doctor build failed ({}): {detail}", output.status))
}

fn doctor_lines(text: &str, mode: DoctorMode) -> Vec<String> {
    let relevant = match mode {
        DoctorMode::Check => text,
        DoctorMode::Fix => text.rsplit("Doctor Check (After)").next().unwrap_or(text),
    };
    relevant
        .lines()
        .filter(|line| line.trim_start().starts_with('['))
        .map(|line| line.to_string())
        .collect()
}

fn parse_doctor_summary(text: &str) -> Option<DoctorReport> {
    let line = text
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("Summary:"))?;
    Some(DoctorReport {
        ok: parse_summary_field(line, "ok=")?,
        warn: parse_summary_field(line, "warn=")?,
        error: parse_summary_field(line, "error=")?,
        crash: parse_summary_field(line, "crash=")?,
    })
}

fn parse_summary_field(line: &str, key: &str) -> Option<usize> {
    let rest = line.split(key).nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_age_buckets_seconds_minutes_hours_days() {
        let cases = [
            (5_000, "just now"),
            (59_000, "59s ago"),
            (60_000, "1m ago"),
            (3_599_000, "59m ago"),
            (3_600_000, "1h ago"),
            (86_399_000, "23h ago"),
            (86_400_000, "1d ago"),
            (0, "just now"),
        ];
        for (elapsed_ms, expected) in cases {
            assert_eq!(
                relative_age(1_000_000_000 + elapsed_ms, 1_000_000_000),
                expected,
                "elapsed_ms: {elapsed_ms}"
            );
        }
    }

    fn span_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn relative_age_gains_a_seconds_bucket() {
        let cases = [
            (5_000, "just now"),
            (10_000, "10s ago"),
            (59_000, "59s ago"),
            (60_000, "1m ago"),
        ];
        for (elapsed_ms, expected) in cases {
            assert_eq!(
                relative_age(1_000_000_000 + elapsed_ms, 1_000_000_000),
                expected,
                "elapsed_ms: {elapsed_ms}"
            );
        }
    }

    #[test]
    fn doctor_status_covers_panel_states() {
        let report = DoctorReport {
            ok: 11,
            warn: 0,
            error: 0,
            crash: 0,
        };
        let run = DoctorRun {
            report,
            lines: Vec::new(),
        };
        let warn_report = DoctorReport {
            ok: 9,
            warn: 2,
            error: 0,
            crash: 0,
        };
        let warn_run = DoctorRun {
            report: warn_report,
            lines: Vec::new(),
        };
        let now = 1_000_000_000;
        let cases = [
            (
                DoctorPanel {
                    last: None,
                    last_at_ms: None,
                    manual: None,
                    error: None,
                },
                Color::Yellow,
                "waiting for first check",
            ),
            (
                DoctorPanel {
                    last: Some(run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: None,
                },
                Color::Green,
                "all good · 11 checks · 15s ago",
            ),
            (
                DoctorPanel {
                    last: Some(warn_run.clone()),
                    last_at_ms: Some(now - 5_000),
                    manual: None,
                    error: None,
                },
                Color::Yellow,
                "2 divergences · 2 warn · 0 err · just now",
            ),
            (
                DoctorPanel {
                    last: Some(run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: Some("boom".to_string()),
                },
                Color::Green,
                "all good · 11 checks · 15s ago · probe failed",
            ),
            (
                DoctorPanel {
                    last: None,
                    last_at_ms: None,
                    manual: None,
                    error: Some("doctor binary not built · press d".to_string()),
                },
                Color::Yellow,
                "doctor binary not built · press d",
            ),
        ];
        for (panel, expected_color, expected_text) in cases {
            let (color, spans) = doctor_status(&panel, now);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }

    #[test]
    fn plugins_status_reflects_link_state() {
        let fresh = DevLink {
            name: "foo".to_string(),
            needs_rebuild: false,
            rebuild_reason: "Up to date".to_string(),
        };
        let stale = DevLink {
            name: "bar".to_string(),
            needs_rebuild: true,
            rebuild_reason: "Source changed".to_string(),
        };
        let cases = [
            (
                LinksState::Live(vec![fresh.clone(), stale.clone()]),
                Color::Yellow,
                "2 linked · 1 stale",
            ),
            (
                LinksState::Live(vec![fresh.clone()]),
                Color::Green,
                "1 linked",
            ),
            (LinksState::Unknown, Color::Green, "3 linked"),
            (
                LinksState::Unreachable,
                Color::Yellow,
                "3 linked · api down",
            ),
        ];
        for (links, expected_color, expected_text) in cases {
            let (color, spans) = plugins_status(&RebuildState::Idle, 3, &links);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }

    #[test]
    fn plugins_status_appends_reload_failure() {
        let (color, spans) = plugins_status(
            &RebuildState::Failed("boom".to_string()),
            3,
            &LinksState::Unknown,
        );
        assert_eq!(color, Color::Red, "failed reload turns the row red");
        assert!(
            span_text(&spans).contains("reload failed · boom"),
            "spans: {}",
            span_text(&spans)
        );
    }

    #[test]
    fn diving_into_emu_requests_an_emu_poke() {
        let mut dash = Dash::new(Vec::new());
        dash.cursor = 3;
        apply_action(&mut dash, Action::Dive, false);
        assert!(dash.pokes.emu, "emu dive marks the emu probe dirty");
        assert!(matches!(dash.view, View::Emu), "dive opened the emu view");
    }

    #[test]
    fn action_for_maps_keys() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let cases = [
            (KeyCode::Char('l'), none, Action::ToggleView),
            (KeyCode::Char('L'), none, Action::ToggleView),
            (KeyCode::Char('d'), none, Action::Doctor),
            (KeyCode::Char('D'), none, Action::Doctor),
            (KeyCode::Esc, none, Action::Back),
            (KeyCode::Left, none, Action::Back),
            (KeyCode::Enter, none, Action::Activate),
            (KeyCode::Right, none, Action::Dive),
            (KeyCode::Char('r'), ctrl, Action::Rebuild),
            (KeyCode::Char('p'), ctrl, Action::Ignore),
            (KeyCode::Char('u'), ctrl, Action::ReloadSelf),
            (KeyCode::Char('c'), ctrl, Action::Quit),
            (KeyCode::Char('q'), none, Action::Quit),
            (KeyCode::Up, none, Action::ScrollUp),
            (KeyCode::Down, none, Action::ScrollDown),
            (KeyCode::PageUp, none, Action::PageUp),
            (KeyCode::PageDown, none, Action::PageDown),
            (KeyCode::End, none, Action::Follow),
            (KeyCode::Char('f'), none, Action::Follow),
            (KeyCode::Char('/'), none, Action::Filter),
            (KeyCode::Char('c'), none, Action::Copy),
            (KeyCode::Char('C'), none, Action::Copy),
            (KeyCode::Char('k'), none, Action::ToggleKeys),
            (KeyCode::Char('K'), none, Action::ToggleKeys),
            (KeyCode::Char('r'), none, Action::Ignore),
            (KeyCode::Char('p'), none, Action::Ignore),
            (KeyCode::Char('x'), none, Action::Ignore),
            (KeyCode::Char('u'), none, Action::Ignore),
        ];
        for (code, mods, expected) in cases {
            assert_eq!(action_for(code, mods), expected, "{code:?} {mods:?}");
        }
    }

    #[test]
    fn dashboard_cursor_moves_and_clamps() {
        let mut dash = Dash::new(Vec::new());
        assert_eq!(dash.cursor, 0);
        apply_action(&mut dash, Action::ScrollUp, false);
        assert_eq!(dash.cursor, 0, "clamps at top");
        for _ in 0..10 {
            apply_action(&mut dash, Action::ScrollDown, false);
        }
        assert_eq!(dash.cursor, ROWS.len() - 1, "clamps at bottom");
        apply_action(&mut dash, Action::ScrollUp, false);
        assert_eq!(dash.cursor, ROWS.len() - 2);
    }

    #[test]
    fn armed_status_offers_fix_only_on_doctor_row() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        let cases = [(4usize, "enter fix"), (0, "no armed action")];
        for (cursor, expected) in cases {
            dash.cursor = cursor;
            let footer = armed_status(&dash);
            assert!(footer.contains(expected), "cursor {cursor}: {footer}");
        }
    }

    #[test]
    fn emu_row_opens_emu_view() {
        let mut dash = Dash::new(Vec::new());
        dash.cursor = 3;
        apply_action(&mut dash, Action::Activate, false);
        assert!(matches!(dash.view, View::Emu));
    }

    fn emu_env(id: &str, state: ResolveState) -> EnvironmentStatus {
        EnvironmentStatus {
            id: id.to_string(),
            backend: "qemu".to_string(),
            state,
            reason: String::new(),
            last_run: None,
        }
    }

    #[test]
    fn emu_cursor_moves_and_clamps_without_scrolling() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]);
        let moves = [
            (Action::ScrollDown, 1),
            (Action::ScrollDown, 1),
            (Action::ScrollUp, 0),
            (Action::ScrollUp, 0),
        ];
        for (action, expected) in moves {
            apply_action(&mut dash, action, false);
            assert_eq!(dash.emu_cursor, expected, "after {action:?}");
            assert_eq!(dash.scroll_offset, 0, "after {action:?}");
        }
    }

    fn emu_candidate(id: &str) -> ImageCandidate {
        use crate::commands::emu::{BootMedia, Firmware, GuestArch};
        ImageCandidate {
            id: id.to_string(),
            path: std::path::PathBuf::from(format!("/a/b/{id}.qcow2")),
            display_name: id.to_string(),
            arch: GuestArch::X86_64,
            arch_inferred: true,
            firmware: Firmware::Uefi,
            media: BootMedia::Disk,
        }
    }

    #[test]
    fn emu_cursor_extends_into_candidate_rows_and_clamps() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]);
        dash.emu_candidates = vec![emu_candidate("baz"), emu_candidate("qux")];
        let moves = [
            (Action::ScrollDown, 1),
            (Action::ScrollDown, 2),
            (Action::ScrollDown, 3),
            (Action::ScrollDown, 3),
        ];
        for (action, expected) in moves {
            apply_action(&mut dash, action, false);
            assert_eq!(dash.emu_cursor, expected, "after {action:?}");
        }
    }

    #[test]
    fn candidate_row_label_marks_inferred_and_host_default() {
        use crate::commands::emu::GuestArch;
        let cases = [
            (GuestArch::X86_64, true, "needs arch · x86_64"),
            (GuestArch::Aarch64, true, "needs arch · aarch64"),
            (
                GuestArch::X86_64,
                false,
                "needs arch · x86_64 (host default)",
            ),
            (
                GuestArch::Aarch64,
                false,
                "needs arch · aarch64 (host default)",
            ),
        ];
        for (arch, inferred, expected) in cases {
            assert_eq!(
                candidate_row_label(arch, inferred),
                expected,
                "arch: {arch:?} inferred: {inferred}"
            );
        }
    }

    #[test]
    fn emu_keys_map_open_toggle_and_confirm() {
        let cases = [
            (KeyCode::Char('o'), Action::OpenEmuDir),
            (KeyCode::Char('t'), Action::ToggleArch),
            (KeyCode::Char('a'), Action::Confirm),
        ];
        for (code, expected) in cases {
            assert_eq!(
                action_for(code, KeyModifiers::NONE),
                expected,
                "code: {code:?}"
            );
        }
    }

    #[test]
    fn toggle_arch_flips_selected_candidate_only() {
        use crate::commands::emu::GuestArch;
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]);
        dash.emu_candidates = vec![emu_candidate("baz"), emu_candidate("qux")];

        dash.emu_cursor = 0;
        apply_action(&mut dash, Action::ToggleArch, false);
        assert_eq!(
            dash.emu_candidates
                .iter()
                .map(|candidate| candidate.arch)
                .collect::<Vec<_>>(),
            vec![GuestArch::X86_64, GuestArch::X86_64],
            "cursor on an env row must not mutate any candidate"
        );

        dash.emu_cursor = 3;
        apply_action(&mut dash, Action::ToggleArch, false);
        assert_eq!(dash.emu_candidates[0].arch, GuestArch::X86_64, "untouched");
        assert_eq!(
            dash.emu_candidates[1].arch,
            GuestArch::Aarch64,
            "selected candidate flips"
        );
        assert!(
            dash.emu_candidates[1].arch_inferred,
            "toggle sets arch_inferred"
        );
    }

    #[test]
    fn act_emu_refuses_envs_that_are_not_ready() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Missing)]);
        act_emu(&mut dash, false);
        assert!(
            dash.active_runs.is_empty(),
            "a not-ready emu does not start a run"
        );
    }

    fn live_pane(line: &str) -> LogPane {
        let mut pane = LogPane::new();
        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel::<String>();
        pane.attach(child, rx);
        pane.push(line.to_string());
        pane
    }

    #[test]
    fn diving_into_an_emu_opens_its_detail() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        apply_action(&mut dash, Action::Dive, false);
        assert!(
            matches!(dash.view, View::EmuDetail),
            "dive opened the detail"
        );
        assert_eq!(
            dash.emu_detail.as_ref().map(|detail| detail.id.as_str()),
            Some("foo")
        );
        apply_action(&mut dash, Action::Back, false);
        assert!(matches!(dash.view, View::Emu), "back returns to the list");
        assert!(dash.emu_detail.is_none(), "back clears the detail");
    }

    #[test]
    fn list_and_status_reflect_a_live_run() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        dash.active_runs
            .insert("foo".to_string(), live_pane("  boot     foo · qmp"));
        assert!(is_running(&dash, "foo"));
        assert_eq!(live_verb(&dash, "foo").as_deref(), Some("boot"));
        assert_eq!(status_line(&dash), " foo running · enter stop · → log");
    }

    fn render_text(dash: &mut Dash) -> String {
        use ratatui::backend::TestBackend;
        let mut terminal = ratatui::Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| draw(frame, dash)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn render_rows(dash: &mut Dash) -> Vec<String> {
        use ratatui::backend::TestBackend;
        let mut terminal = ratatui::Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| draw(frame, dash)).unwrap();
        let backend = terminal.backend();
        let buffer = backend.buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    #[test]
    fn every_view_shows_its_page_as_a_breadcrumb_on_the_qol_dev_sign() {
        let cases = [
            (View::Endpoints, "qol dev · endpoints"),
            (View::Plugins, "qol dev · plugins"),
            (View::Doctor, "qol dev · doctor"),
            (View::Emu, "qol dev · emu"),
        ];
        for (view, crumb) in cases {
            let mut dash = Dash::new(Vec::new());
            dash.view = view;
            let rows = render_rows(&mut dash);
            let sign = rows
                .iter()
                .position(|row| row.contains(crumb))
                .unwrap_or_else(|| panic!("{crumb} not rendered on the shell sign"));
            assert!(
                rows[sign].contains('┤') && rows[sign].contains('├'),
                "{crumb}: breadcrumb not framed as a sign"
            );
            assert!(
                rows[sign - 1].contains('╭') && rows[sign - 1].contains('╮'),
                "{crumb}: missing poke-up cap above the sign"
            );
            assert!(
                rows[sign + 1].contains('╰') && rows[sign + 1].contains('╯'),
                "{crumb}: missing sign base below"
            );
            assert!(
                !rows.iter().any(|row| row.contains("┤ menu ├")),
                "{crumb}: page should not nest its own sign-box"
            );
            let desc = page_description(view).expect("listed views carry a description");
            assert!(
                rows.iter().any(|row| row.contains(desc)),
                "{crumb}: description {desc:?} not rendered under the title"
            );
        }
    }

    #[test]
    fn qol_dev_shell_sign_is_centered() {
        let mut dash = Dash::new(Vec::new());
        let rows = render_rows(&mut dash);
        let border = rows
            .iter()
            .position(|row| row.contains("┤ qol dev ├"))
            .expect("shell sign present");
        let row = &rows[border];
        let left = row.chars().take_while(|&c| c == '┌' || c == '─').count() - 1;
        let right = row
            .chars()
            .skip_while(|&c| c != '├')
            .skip(1)
            .take_while(|&c| c == '─')
            .count();
        assert!(
            left.abs_diff(right) <= 1,
            "shell sign not centered ({left} dashes left, {right} right)"
        );
    }

    #[test]
    fn keys_hud_renders_with_view_keys_and_globals() {
        let cases = [
            (View::Dashboard, "arm, then enter"),
            (View::Emu, "boot · stop"),
        ];
        for (view, expected) in cases {
            let mut dash = Dash::new(Vec::new());
            dash.view = view;
            let text = render_text(&mut dash);
            assert!(text.contains(expected), "missing {expected:?}");
            assert!(text.contains("ctrl+u"), "missing globals");
            assert!(text.contains("reload qol dev"), "missing globals");
            assert!(text.contains("keys · k"), "missing toggle badge");
            if matches!(view, View::Emu) {
                assert!(text.contains("toggle arch"), "missing emu o/t/a keys");
            }
        }
    }

    #[test]
    fn toggle_keys_hides_and_restores_the_hud() {
        let mut dash = Dash::new(Vec::new());
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(dash.keys_hidden);
        let text = render_text(&mut dash);
        assert!(!text.contains("reload qol dev"), "hud still rendered");
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(!dash.keys_hidden);
        let text = render_text(&mut dash);
        assert!(text.contains("reload qol dev"), "hud did not come back");
    }

    #[test]
    fn reloading_state_drives_red_accent_and_status() {
        let mut dash = Dash::new(Vec::new());
        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel();
        dash.reload = Reload::Running {
            child,
            rx,
            since: Instant::now(),
        };
        assert!(dash.is_reloading());
        assert_eq!(frame_accent(&dash), Color::Red);
        let status = status_line(&dash);
        assert!(status.contains("RELOADING"), "{status}");
        if let Reload::Running { mut child, .. } = dash.reload {
            let _ = child.wait();
        }
    }

    #[test]
    fn status_line_prompts_to_boot_when_no_emu_runs() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        assert_eq!(
            status_line(&dash),
            " enter boots the selected emu · → detail"
        );
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
    fn humanize_check_id_titlecases_first_word_only() {
        let cases = [
            ("plugin_staleness", "Plugin staleness"),
            ("install_identity", "Install identity"),
            ("rust_formatting", "Rust formatting"),
        ];
        for (id, expected) in cases {
            assert_eq!(humanize_check_id(id), expected, "id: {id}");
        }
    }

    #[test]
    fn friendly_doctor_line_hides_detail_for_ok_and_keeps_it_for_warn() {
        let ok = friendly_doctor_line("[OK] install_identity: marker and id are aligned (x)");
        assert_eq!(
            ok.to_string(),
            "  ✓ Install identity",
            "ok hides the detail"
        );

        let warn = friendly_doctor_line(
            "[WARN] plugin_staleness: plugin staleness detected \u{2014} rebuild required: a, b",
        );
        assert_eq!(
            warn.to_string(),
            "  ▲ Plugin staleness - rebuild required: a, b",
            "warn humanizes the name and keeps the post-dash detail",
        );
    }

    #[test]
    fn doctor_lines_fix_mode_keeps_only_after_block() {
        let text = "Doctor Check (Before)\n[WARN] a: x\nSummary: ok=0\n\nDoctor Check (After)\n[OK] a: x\nSummary: ok=1\n";
        assert_eq!(
            doctor_lines(text, DoctorMode::Check).len(),
            2,
            "check keeps every bracket line"
        );
        assert_eq!(
            doctor_lines(text, DoctorMode::Fix),
            vec!["[OK] a: x".to_string()],
            "fix keeps only the after block"
        );
    }

    #[test]
    fn edit_filter_types_backspaces_applies_and_cancels() {
        let mut dash = Dash::new(Vec::new());
        dash.filtering = true;
        for c in "focus".chars() {
            edit_filter(&mut dash, KeyCode::Char(c));
        }
        assert_eq!(dash.filter, "focus");
        edit_filter(&mut dash, KeyCode::Backspace);
        assert_eq!(dash.filter, "focu", "backspace deletes last char");
        edit_filter(&mut dash, KeyCode::Enter);
        assert!(!dash.filtering, "enter exits typing mode");
        assert_eq!(dash.filter, "focu", "enter keeps the applied query");
        dash.filtering = true;
        edit_filter(&mut dash, KeyCode::Esc);
        assert!(dash.filter.is_empty(), "esc clears the query");
        assert!(!dash.filtering, "esc exits typing mode");
    }

    #[test]
    fn edit_copy_accepts_only_digits_and_cancels() {
        let mut dash = Dash::new(Vec::new());
        dash.copying = true;
        for code in [KeyCode::Char('4'), KeyCode::Char('x'), KeyCode::Char('2')] {
            edit_copy(&mut dash, code);
        }
        assert_eq!(dash.copy_count, "42", "non-digits are ignored");
        edit_copy(&mut dash, KeyCode::Backspace);
        assert_eq!(dash.copy_count, "4", "backspace deletes last digit");
        edit_copy(&mut dash, KeyCode::Esc);
        assert!(dash.copy_count.is_empty(), "esc clears the count");
        assert!(!dash.copying, "esc exits copy mode");
    }

    #[test]
    fn newest_lines_takes_filtered_tail_and_strips_ansi() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;
        for line in ["alpha", "\u{1b}[31mbeta\u{1b}[0m", "gamma", "delta"] {
            dash.trace.push(line.to_string());
        }
        assert_eq!(
            newest_lines(&dash, 2),
            "gamma\ndelta",
            "tail of N newest lines"
        );
        dash.filter = "a".to_string();
        assert_eq!(
            newest_lines(&dash, 2),
            "gamma\ndelta",
            "filter keeps only matching lines before taking the tail"
        );
        dash.filter = "beta".to_string();
        assert_eq!(
            newest_lines(&dash, 5),
            "beta",
            "ansi codes are stripped from the copied text"
        );
    }

    #[test]
    fn doctor_line_style_colors_by_status() {
        let cases = [
            ("[OK] x: y", Some(("✓", Color::Green))),
            ("[WARN] x: y", Some(("▲", Color::Yellow))),
            ("[ERR] x: y", Some(("✗", Color::Red))),
            ("[CRASH] x: y", Some(("✗", Color::Magenta))),
            ("Summary: ok=1", None),
        ];
        for (input, expected) in cases {
            assert_eq!(doctor_line_style(input), expected, "input: {input}");
        }
    }

    #[test]
    fn parse_doctor_summary_reads_counts() {
        let report = parse_doctor_summary(
            "Doctor Check\n[OK] x: y\nSummary: ok=9, warn=2, error=1, crash=0",
        )
        .expect("summary present");
        assert_eq!(
            (report.ok, report.warn, report.error, report.crash),
            (9, 2, 1, 0)
        );
        assert_eq!(report.divergences(), 3);
        assert!(parse_doctor_summary("no summary line here").is_none());
    }

    #[test]
    fn forwarder_decodes_non_utf8_lossily_and_ends_on_eof() {
        let (tx, rx) = channel();
        let data = b"ok\n\xFF\xFEbad\nlast".to_vec();
        spawn_forwarder(std::io::Cursor::new(data), tx);
        let lines: Vec<String> = rx.iter().collect();
        assert_eq!(lines.len(), 3, "all lines delivered: {lines:?}");
        assert_eq!(lines[0], "ok");
        assert!(lines[1].contains("bad"), "lossy line kept: {:?}", lines[1]);
        assert_eq!(lines[2], "last", "no trailing newline still delivered");
    }

    #[test]
    fn log_ring_caps_and_evicts_oldest() {
        let mut ring = LogRing::new();
        for i in 0..(LOG_CAP + 3) {
            ring.push(format!("line-{i}"));
        }
        assert_eq!(ring.len(), LOG_CAP);
        assert_eq!(ring.lines.front().map(String::as_str), Some("line-3"));
    }

    #[test]
    fn window_math_keeps_tail_and_clamps() {
        let cases = [
            ("follow shows tail", 100, 10, 0, 90),
            ("scrolled up shifts window", 100, 10, 25, 65),
            ("short log starts at zero", 5, 10, 0, 0),
            ("offset beyond start clamps to zero", 100, 10, 500, 0),
        ];
        for (label, len, height, offset, want_start) in cases {
            assert_eq!(window_start(len, height, offset), want_start, "{label}");
        }
        assert_eq!(clamp_offset(100, 10, 500), 90, "clamp to len-height");
        assert_eq!(clamp_offset(5, 10, 3), 0, "short log clamps to zero");
    }

    #[test]
    fn emu_empty_lines_list_config_path() {
        let lines = emu_empty_lines("~/.config/qol-tray/emu.toml");
        assert_eq!(lines.len(), 2, "lines: {lines:?}");
    }
}
