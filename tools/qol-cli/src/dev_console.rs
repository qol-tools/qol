use std::collections::VecDeque;
use std::io::{BufRead, BufReader, IsTerminal, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::commands::emu::{
    emu_config_path, environment_statuses, EnvironmentStatus, LastRun, ResolveState,
};
use crate::dev_server::{
    health_ok, post_recompile_current, post_reload_plugins, probe_endpoints, web_ok,
    EndpointStatus, WEBSITE_URL,
};
use crate::host_facade;

const LOG_CAP: usize = 2000;
const TICK: Duration = Duration::from_millis(150);
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const EMU_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(5);
const CRASH_TAIL: usize = 40;
const ACK_TTL: Duration = Duration::from_secs(6);

pub(crate) enum SessionEnd {
    ChildExited(ExitStatus),
    UserQuit,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Action {
    ToggleView,
    Rebuild,
    ReloadPlugins,
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
    Ignore,
}

fn action_for(code: KeyCode, mods: KeyModifiers) -> Action {
    if mods.contains(KeyModifiers::CONTROL) {
        return match code {
            KeyCode::Char('r') => Action::Rebuild,
            KeyCode::Char('p') => Action::ReloadPlugins,
            KeyCode::Char('c') => Action::Quit,
            _ => Action::Ignore,
        };
    }
    match code {
        KeyCode::Char('l') | KeyCode::Char('L') => Action::ToggleView,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Dashboard,
    Logs,
    Doctor,
    Plugins,
    Emu,
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

enum EndpointsState {
    Probing,
    Done(Vec<EndpointStatus>),
}

enum EmuState {
    Probing,
    Done(Vec<EnvironmentStatus>),
    Failed(String),
}

#[derive(Clone, Copy)]
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

enum DoctorState {
    Running(DoctorMode),
    Done(DoctorReport),
    Failed(String),
}

struct DoctorRun {
    report: DoctorReport,
    lines: Vec<String>,
}

enum RebuildState {
    Idle,
    Requested(Instant),
    Failed(String),
}

struct Dash {
    view: View,
    logs: LogRing,
    scroll_offset: usize,
    health: Health,
    web: Health,
    endpoints: EndpointsState,
    endpoints_rx: Option<Receiver<Vec<EndpointStatus>>>,
    started: Instant,
    rebuild: RebuildState,
    plugin_reload: RebuildState,
    plugin_names: Vec<String>,
    emu: EmuState,
    emu_rx: Option<Receiver<Result<Vec<EnvironmentStatus>, String>>>,
    emu_last_refresh: Option<Instant>,
    log_height: usize,
    cursor: usize,
    doctor: DoctorState,
    doctor_lines: Vec<String>,
    doctor_rx: Option<Receiver<Result<DoctorRun, String>>>,
    trace: LogRing,
    trace_child: Option<Child>,
    trace_rx: Option<Receiver<String>>,
    filter: String,
    filtering: bool,
    copy_count: String,
    copying: bool,
    copy_ack: Option<(Instant, String)>,
    armed: bool,
}

impl Dash {
    fn new(plugin_names: Vec<String>) -> Self {
        Self {
            view: View::Dashboard,
            logs: LogRing::new(),
            scroll_offset: 0,
            health: Health::Checking,
            web: Health::Checking,
            endpoints: EndpointsState::Probing,
            endpoints_rx: None,
            started: Instant::now(),
            rebuild: RebuildState::Idle,
            plugin_reload: RebuildState::Idle,
            plugin_names,
            emu: EmuState::Probing,
            emu_rx: None,
            emu_last_refresh: None,
            log_height: 0,
            cursor: 0,
            doctor: DoctorState::Running(DoctorMode::Check),
            doctor_lines: Vec::new(),
            doctor_rx: None,
            trace: LogRing::new(),
            trace_child: None,
            trace_rx: None,
            filter: String::new(),
            filtering: false,
            copy_count: String::new(),
            copying: false,
            copy_ack: None,
            armed: false,
        }
    }

    fn start_doctor(&mut self) {
        self.doctor = DoctorState::Running(DoctorMode::Check);
        self.doctor_rx = Some(spawn_doctor(DoctorMode::Check));
    }

    fn start_doctor_fix(&mut self) {
        self.doctor = DoctorState::Running(DoctorMode::Fix);
        self.doctor_rx = Some(spawn_doctor(DoctorMode::Fix));
    }

    fn start_emu_refresh(&mut self, force: bool) {
        if self.emu_rx.is_some() {
            return;
        }
        let due = self
            .emu_last_refresh
            .is_none_or(|last| last.elapsed() >= EMU_REFRESH_INTERVAL);
        if !force && !due {
            return;
        }
        if force || self.emu_last_refresh.is_none() {
            self.emu = EmuState::Probing;
        }
        self.emu_last_refresh = Some(Instant::now());
        self.emu_rx = Some(spawn_emu_probe());
    }
}

pub(crate) fn run_session(
    child: &mut Child,
    verbose: bool,
    plugins: Vec<String>,
) -> Result<SessionEnd> {
    let lines = spawn_forwarders(child);
    if verbose || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return plain_session(child, &lines);
    }
    let health = spawn_health_probe();
    let mut dash = Dash::new(plugins);
    dash.start_doctor();
    let mut terminal = ratatui::init();
    let result = tui_session(&mut terminal, child, &lines, &health, &mut dash);
    ratatui::restore();
    if let Ok(SessionEnd::ChildExited(status)) = &result {
        if !status.success() {
            print_crash_tail(&dash.logs);
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

fn plain_session(child: &mut Child, lines: &Receiver<String>) -> Result<SessionEnd> {
    loop {
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
    health: &Receiver<HealthSnapshot>,
    dash: &mut Dash,
) -> Result<SessionEnd> {
    loop {
        dash.start_emu_refresh(false);
        while let Ok(line) = lines.try_recv() {
            dash.logs.push(line);
        }
        while let Ok(snapshot) = health.try_recv() {
            dash.health = health_state(snapshot.api);
            dash.web = health_state(snapshot.web);
        }
        if let Some(results) = dash.endpoints_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            dash.endpoints = EndpointsState::Done(results);
            dash.endpoints_rx = None;
        }
        if let Some(outcome) = dash.emu_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            dash.emu = match outcome {
                Ok(statuses) => EmuState::Done(statuses),
                Err(error) => EmuState::Failed(error),
            };
            dash.emu_rx = None;
        }
        let doctor_outcome = dash.doctor_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(outcome) = doctor_outcome {
            match outcome {
                Ok(run) => {
                    dash.doctor = DoctorState::Done(run.report);
                    dash.doctor_lines = run.lines;
                }
                Err(error) => {
                    dash.doctor_lines = vec![format!("[ERR] doctor: {error}")];
                    dash.doctor = DoctorState::Failed(error);
                }
            }
            dash.doctor_rx = None;
        }
        drain_trace(dash);
        if let Some(status) = try_wait(child)? {
            while let Ok(line) = lines.try_recv() {
                dash.logs.push(line);
            }
            stop_trace(dash);
            return Ok(SessionEnd::ChildExited(status));
        }
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
                        stop_child(child)?;
                        return Ok(SessionEnd::UserQuit);
                    }
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

fn drain_trace(dash: &mut Dash) {
    let mut received = Vec::new();
    if let Some(rx) = dash.trace_rx.as_ref() {
        while let Ok(line) = rx.try_recv() {
            received.push(line);
        }
    }
    for line in received {
        dash.trace.push(line);
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
    let ring = if dash.view == View::Trace {
        &dash.trace
    } else {
        &dash.logs
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
        Action::Rebuild => trigger_rebuild(dash),
        Action::ReloadPlugins => trigger_reload(dash),
        Action::Doctor => open_doctor(dash),
        Action::Activate => {
            if dash.view == View::Dashboard {
                act_row(dash, modified);
            }
        }
        Action::Dive => {
            if dash.view == View::Dashboard {
                dive_row(dash);
            }
        }
        Action::Back => {
            if dash.view == View::Trace {
                stop_trace(dash);
            }
            dash.view = View::Dashboard;
            dash.scroll_offset = 0;
            dash.filter.clear();
            dash.filtering = false;
        }
        Action::ScrollUp => {
            if dash.view == View::Dashboard {
                dash.cursor = dash.cursor.saturating_sub(1);
            } else {
                dash.scroll_offset = dash.scroll_offset.saturating_add(1);
            }
        }
        Action::ScrollDown => {
            if dash.view == View::Dashboard {
                dash.cursor = (dash.cursor + 1).min(ROWS.len() - 1);
            } else {
                dash.scroll_offset = dash.scroll_offset.saturating_sub(1);
            }
        }
        Action::PageUp => dash.scroll_offset = dash.scroll_offset.saturating_add(page),
        Action::PageDown => dash.scroll_offset = dash.scroll_offset.saturating_sub(page),
        Action::Follow => dash.scroll_offset = 0,
        Action::Filter => {
            if matches!(dash.view, View::Logs | View::Trace) {
                dash.filtering = true;
            }
        }
        Action::Copy => {
            if matches!(dash.view, View::Logs | View::Trace) {
                dash.copying = true;
                dash.copy_count.clear();
                dash.scroll_offset = 0;
            }
        }
        Action::Quit | Action::Ignore => {}
    }
    let len = if dash.view == View::Trace {
        dash.trace.len()
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
                dash.start_doctor_fix();
            } else {
                dash.start_doctor();
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
    dash.endpoints = EndpointsState::Probing;
    dash.endpoints_rx = Some(spawn_endpoints_probe());
}

fn open_emu(dash: &mut Dash) {
    dash.view = View::Emu;
    dash.scroll_offset = 0;
    dash.start_emu_refresh(true);
}

fn health_state(up: bool) -> Health {
    if up {
        Health::Up
    } else {
        Health::Down
    }
}

fn open_trace(dash: &mut Dash) {
    dash.view = View::Trace;
    dash.scroll_offset = 0;
    if dash.trace_child.is_some() {
        return;
    }
    match spawn_trace() {
        Some((child, rx)) => {
            dash.trace_child = Some(child);
            dash.trace_rx = Some(rx);
        }
        None => dash.trace.push(
            "[qol dev] could not start tracer (need python3 + tools/compact_trace.py)".to_string(),
        ),
    }
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
    if let Some(mut child) = dash.trace_child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    dash.trace_rx = None;
}

fn trigger_rebuild(dash: &mut Dash) {
    dash.rebuild = match post_recompile_current() {
        Ok(()) => RebuildState::Requested(Instant::now()),
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

fn trigger_reload(dash: &mut Dash) {
    dash.plugin_reload = match post_reload_plugins() {
        Ok(()) => RebuildState::Requested(Instant::now()),
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

fn open_doctor(dash: &mut Dash) {
    dash.view = View::Doctor;
    dash.scroll_offset = 0;
    dash.start_doctor();
}

fn draw(frame: &mut Frame, dash: &mut Dash) {
    let [main, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    match dash.view {
        View::Dashboard => draw_dashboard(frame, dash, main),
        View::Logs => draw_logs(frame, dash, main),
        View::Doctor => draw_doctor(frame, dash, main),
        View::Plugins => draw_plugins(frame, dash, main),
        View::Emu => draw_emu(frame, dash, main),
        View::Trace => draw_trace(frame, dash, main),
        View::Endpoints => draw_endpoints(frame, dash, main),
    }
    let footer_style = if dash.armed {
        Style::new().fg(Color::Yellow).bold()
    } else {
        Style::new().dim()
    };
    frame.render_widget(
        Paragraph::new(footer_text(dash)).style(footer_style),
        footer,
    );
}

fn footer_text(dash: &Dash) -> String {
    if dash.copying {
        return format!(
            " copy last N: {}_ · enter copy · esc cancel ",
            dash.copy_count
        );
    }
    if dash.filtering {
        return format!(" filter: {}_ · enter apply · esc cancel ", dash.filter);
    }
    if dash.armed {
        return armed_footer(dash);
    }
    if let Some((at, message)) = &dash.copy_ack {
        if at.elapsed() < ACK_TTL {
            return format!(" {message} ");
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
            format!(" ↑/↓ move · {hints} · space modify · q quit ")
        }
        View::Logs | View::Trace => stream_footer(dash),
        View::Doctor => " space raw · d refresh · ↑/↓ scroll · ← back · q quit ".to_string(),
        View::Plugins | View::Emu => " ↑/↓ scroll · ← back · q quit ".to_string(),
        View::Endpoints => " ← back · q quit ".to_string(),
    }
}

fn armed_footer(dash: &Dash) -> String {
    match dash.view {
        View::Dashboard => {
            let hint = match ROWS[dash.cursor] {
                Row::Doctor => "enter fix",
                Row::Tray | Row::Web | Row::Plugins | Row::Emu | Row::Logs | Row::Trace => {
                    "no modified action"
                }
            };
            format!(" MODIFIED · {hint} · space/esc cancel ")
        }
        View::Doctor => " RAW · space friendly · esc done ".to_string(),
        View::Logs | View::Trace | View::Plugins | View::Emu | View::Endpoints => {
            " MODIFIED · space/esc cancel ".to_string()
        }
    }
}

fn stream_footer(dash: &Dash) -> String {
    let filter = if dash.filter.is_empty() {
        String::new()
    } else {
        format!(" · filter: {}", dash.filter)
    };
    format!(" ↑/↓ scroll · f follow · c copy · / filter{filter} · ← back · q quit ")
}

fn draw_dashboard(frame: &mut Frame, dash: &Dash, area: Rect) {
    let (tray_color, tray_value) = tray_status(dash);
    let (web_color, web_value) = web_status(dash.web);
    let (plugins_color, plugins_value) =
        plugins_status(&dash.plugin_reload, dash.plugin_names.len());
    let (emu_color, emu_value) = emu_status(&dash.emu);
    let (doctor_color, doctor_value) = doctor_status(&dash.doctor);

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

    let accent = accent_color(dash.armed);
    let label = if dash.armed {
        "qol dev · MODIFIED"
    } else {
        "qol dev"
    };
    let [cap, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let block = Block::bordered().border_style(Style::new().fg(accent));
    let inner = block.inner(body);
    frame.render_widget(block, body);
    let rows_area = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };
    frame.render_widget(Paragraph::new(rows), rows_area);
    draw_title_badge(frame, cap, body, label, accent);
}

fn draw_title_badge(frame: &mut Frame, cap: Rect, body: Rect, label: &str, accent: Color) {
    let span = label.chars().count() as u16 + 2;
    let width = span + 2;
    if width + 2 > body.width {
        return;
    }
    let x = body.x + (body.width - width) / 2;
    let bar = "─".repeat(span as usize);
    let edge = accent;
    render_overlay(frame, x, cap.y, Line::from(format!("╭{bar}╮").fg(edge)));
    render_overlay(
        frame,
        x,
        body.y,
        Line::from(vec![
            "┤ ".fg(edge),
            label.to_string().fg(accent).bold(),
            " ├".fg(edge),
        ]),
    );
    render_overlay(
        frame,
        x,
        body.y + 1,
        Line::from(format!("╰{bar}╯").fg(edge)),
    );
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

fn accent_color(armed: bool) -> Color {
    if armed {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn panel(title: &str, accent: Color) -> Block<'static> {
    Block::bordered()
        .border_style(Style::new().fg(accent))
        .title(
            Line::from(vec![
                "┤ ".fg(accent),
                title.trim().to_string().fg(accent).bold(),
                " ├".fg(accent),
            ])
            .centered(),
        )
}

fn plugins_status(state: &RebuildState, plugins: usize) -> (Color, Vec<Span<'static>>) {
    let base = format!("{plugins} linked");
    match state {
        RebuildState::Requested(at) if at.elapsed() < ACK_TTL => (
            Color::Green,
            vec![base.fg(Color::Green), " · reload sent".fg(Color::Yellow)],
        ),
        RebuildState::Failed(error) => (
            Color::Red,
            vec![
                format!("{base} · reload ").fg(Color::DarkGray),
                "failed".fg(Color::Red).bold(),
                format!(" · {error}").fg(Color::DarkGray),
            ],
        ),
        RebuildState::Idle | RebuildState::Requested(_) => {
            (Color::Green, vec![base.fg(Color::Green)])
        }
    }
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
    let accent = accent_color(dash.armed);
    if dash.plugin_names.is_empty() {
        frame.render_widget(
            Paragraph::new("  no dev-linked plugins").block(panel(" plugins ", accent)),
            area,
        );
        return;
    }
    let total = dash.plugin_names.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = dash
        .plugin_names
        .iter()
        .skip(start)
        .take(height)
        .map(|name| {
            Line::from(vec![
                "  ".into(),
                "●".fg(Color::Green).bold(),
                format!(" {name}").fg(Color::White),
                " · dev-linked".fg(Color::DarkGray),
            ])
        })
        .collect();
    let title = format!(" plugins · {} ", list_status(total, dash.scroll_offset));
    frame.render_widget(Paragraph::new(visible).block(panel(&title, accent)), area);
}

fn draw_emu(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let accent = accent_color(dash.armed);
    let config = emu_config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "~/.config/qol-tray/emu.toml".to_string());
    let mut lines = match &dash.emu {
        EmuState::Probing => vec![Line::from("  scanning emus".fg(Color::Yellow))],
        EmuState::Done(statuses) if statuses.is_empty() => {
            vec![Line::from("  no emus found".fg(Color::DarkGray))]
        }
        EmuState::Done(statuses) => statuses
            .iter()
            .flat_map(|status| {
                let color = match status.state {
                    ResolveState::Ready => Color::Green,
                    ResolveState::Missing => Color::Yellow,
                    ResolveState::Unsupported => Color::Red,
                };
                let mut entry = vec![
                    Line::from(vec![
                        "  ● ".fg(color).bold(),
                        format!("{:<12}", status.id).fg(Color::White),
                        format!(" {} ", status.state.as_str()).fg(color).bold(),
                        format!("· {} · {} · {}", status.name, status.backend, status.arch)
                            .fg(Color::DarkGray),
                    ]),
                    Line::from(vec![
                        "    ".into(),
                        status.reason.clone().fg(Color::DarkGray),
                    ]),
                ];
                entry.push(last_run_line(status.last_run.as_ref()));
                entry
            })
            .collect(),
        EmuState::Failed(error) => vec![Line::from(vec![
            "  registry error ".fg(Color::Red).bold(),
            error.clone().fg(Color::DarkGray),
        ])],
    };
    lines.extend([
        Line::from(""),
        Line::from(vec![
            "  config  ".fg(Color::DarkGray),
            config.fg(Color::White),
        ]),
        Line::from(vec![
            "  runs    ".fg(Color::DarkGray),
            "target/qol-emu".fg(Color::White),
        ]),
    ]);
    let total = lines.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();
    let title = format!(" emu · {} ", list_status(total, dash.scroll_offset));
    frame.render_widget(Paragraph::new(visible).block(panel(&title, accent)), area);
}

fn last_run_line(last_run: Option<&LastRun>) -> Line<'static> {
    let Some(run) = last_run else {
        return Line::from(vec![
            "    last ".fg(Color::DarkGray),
            "never run".fg(Color::DarkGray),
        ]);
    };
    let color = match run.status.as_str() {
        "pass" => Color::Green,
        "failed" => Color::Red,
        "running" => Color::Yellow,
        _ => Color::DarkGray,
    };
    let mut spans = vec![
        "    last ".fg(Color::DarkGray),
        run.status.clone().fg(color).bold(),
        format!(
            " · {}",
            relative_age(now_unix_ms(), run.finished_at_unix_ms)
        )
        .fg(Color::DarkGray),
    ];
    if let Some(version) = &run.qemu_version {
        spans.push(format!(" · qemu {version}").fg(Color::DarkGray));
    }
    Line::from(spans)
}

fn relative_age(now_ms: u64, then_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..=59 => "just now".to_string(),
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

fn doctor_status(state: &DoctorState) -> (Color, Vec<Span<'static>>) {
    match state {
        DoctorState::Running(mode) => (Color::Yellow, vec![mode.gerund().fg(Color::Yellow)]),
        DoctorState::Done(report) if report.divergences() == 0 => (
            Color::Green,
            vec![
                "all good".fg(Color::Green).bold(),
                format!(" · {} checks", report.ok).fg(Color::DarkGray),
            ],
        ),
        DoctorState::Done(report) => {
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
        }
        DoctorState::Failed(error) => (
            Color::Red,
            vec![
                "failed".fg(Color::Red).bold(),
                format!(" · {error}").fg(Color::DarkGray),
            ],
        ),
    }
}

fn draw_logs(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let accent = accent_color(dash.armed);
    let highlight = copy_highlight(dash);
    draw_stream(
        frame,
        area,
        &dash.logs,
        &dash.filter,
        &mut dash.scroll_offset,
        &mut dash.log_height,
        "logs",
        accent,
        highlight,
    );
}

fn draw_trace(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let accent = accent_color(dash.armed);
    if dash.trace.lines.is_empty() && dash.filter.is_empty() {
        frame.render_widget(
            Paragraph::new("  waiting for trace events").block(panel(" trace ", accent)),
            area,
        );
        return;
    }
    let highlight = copy_highlight(dash);
    draw_stream(
        frame,
        area,
        &dash.trace,
        &dash.filter,
        &mut dash.scroll_offset,
        &mut dash.log_height,
        "trace",
        accent,
        highlight,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_stream(
    frame: &mut Frame,
    area: Rect,
    ring: &LogRing,
    filter: &str,
    scroll_offset: &mut usize,
    log_height: &mut usize,
    title_word: &str,
    accent: Color,
    highlight_tail: Option<usize>,
) {
    let height = area.height.saturating_sub(2) as usize;
    *log_height = height;
    let filtered: Vec<&String> = ring
        .lines
        .iter()
        .filter(|line| filter.is_empty() || line.contains(filter))
        .collect();
    let total = filtered.len();
    *scroll_offset = clamp_offset(total, height, *scroll_offset);
    let start = window_start(total, height, *scroll_offset);
    let highlight_from = highlight_tail.map(|n| total.saturating_sub(n));
    let inner_width = area.width.saturating_sub(2) as usize;
    let visible: Vec<Line> = filtered
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
    let title = format!(" {title_word} · {} ", list_status(total, *scroll_offset));
    frame.render_widget(Paragraph::new(visible).block(panel(&title, accent)), area);
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
    if dash.trace_child.is_some() {
        vec![format!("{} lines", dash.trace.len()).fg(Color::DarkGray)]
    } else {
        vec!["idle · → open".fg(Color::DarkGray)]
    }
}

fn draw_endpoints(frame: &mut Frame, dash: &Dash, area: Rect) {
    let accent = accent_color(dash.armed);
    let lines: Vec<Line> = match &dash.endpoints {
        EndpointsState::Probing => vec![Line::from("  probing endpoints".fg(Color::DarkGray))],
        EndpointsState::Done(items) => items.iter().map(endpoint_line).collect(),
    };
    frame.render_widget(
        Paragraph::new(lines).block(panel(" endpoints ", accent)),
        area,
    );
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
    let accent = accent_color(dash.armed);
    if dash.doctor_lines.is_empty() {
        let message = match dash.doctor {
            DoctorState::Running(mode) => mode.progress_message(),
            DoctorState::Done(_) | DoctorState::Failed(_) => {
                "  no checks reported · press d to run"
            }
        };
        frame.render_widget(
            Paragraph::new(message).block(panel(" doctor ", accent)),
            area,
        );
        return;
    }
    let total = dash.doctor_lines.len();
    let (start, height) = list_window(dash, area, total);
    let render = if dash.armed {
        styled_doctor_line
    } else {
        friendly_doctor_line
    };
    let visible: Vec<Line> = dash
        .doctor_lines
        .iter()
        .skip(start)
        .take(height)
        .map(|line| render(line))
        .collect();
    let title = format!(" doctor · {} ", list_status(total, dash.scroll_offset));
    frame.render_widget(Paragraph::new(visible).block(panel(&title, accent)), area);
}

fn list_window(dash: &mut Dash, area: Rect, total: usize) -> (usize, usize) {
    let height = area.height.saturating_sub(2) as usize;
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

fn spawn_health_probe() -> Receiver<HealthSnapshot> {
    let (tx, rx) = channel();
    std::thread::spawn(move || loop {
        let snapshot = HealthSnapshot {
            api: health_ok(),
            web: web_ok(),
        };
        if tx.send(snapshot).is_err() {
            return;
        }
        std::thread::sleep(HEALTH_PROBE_INTERVAL);
    });
    rx
}

fn spawn_endpoints_probe() -> Receiver<Vec<EndpointStatus>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(probe_endpoints());
    });
    rx
}

fn spawn_emu_probe() -> Receiver<Result<Vec<EnvironmentStatus>, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let outcome = environment_statuses().map_err(|error| format!("{error:#}"));
        let _ = tx.send(outcome);
    });
    rx
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
    let binary = root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray-doctor"));
    let output = Command::new(&binary)
        .current_dir(&root)
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
            (59_000, "just now"),
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
            (KeyCode::Char('p'), ctrl, Action::ReloadPlugins),
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
            (KeyCode::Char('r'), none, Action::Ignore),
            (KeyCode::Char('p'), none, Action::Ignore),
            (KeyCode::Char('x'), none, Action::Ignore),
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
    fn armed_footer_offers_fix_only_on_doctor_row() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        let cases = [(4usize, "enter fix"), (0, "no modified action")];
        for (cursor, expected) in cases {
            dash.cursor = cursor;
            let footer = armed_footer(&dash);
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
}
