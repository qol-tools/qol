use std::collections::VecDeque;
use std::io::{BufRead, BufReader, IsTerminal, Read};
use std::process::{Child, Command, ExitStatus};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::dev_server::{health_ok, post_recompile_current, post_reload_plugins};
use crate::host_facade;

const LOG_CAP: usize = 2000;
const TICK: Duration = Duration::from_millis(150);
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const STOP_GRACE: Duration = Duration::from_secs(5);
const CRASH_TAIL: usize = 40;

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
    Quit,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Follow,
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
        KeyCode::Esc => Action::Back,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Up => Action::ScrollUp,
        KeyCode::Down => Action::ScrollDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::End | KeyCode::Char('f') => Action::Follow,
        _ => Action::Ignore,
    }
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Health {
    Checking,
    Up,
    Down,
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
    Running,
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
    started: Instant,
    rebuild: RebuildState,
    plugin_reload: RebuildState,
    plugins: usize,
    log_height: usize,
    doctor: DoctorState,
    doctor_lines: Vec<String>,
    doctor_rx: Option<Receiver<Result<DoctorRun, String>>>,
}

impl Dash {
    fn new(plugins: usize) -> Self {
        Self {
            view: View::Dashboard,
            logs: LogRing::new(),
            scroll_offset: 0,
            health: Health::Checking,
            started: Instant::now(),
            rebuild: RebuildState::Idle,
            plugin_reload: RebuildState::Idle,
            plugins,
            log_height: 0,
            doctor: DoctorState::Running,
            doctor_lines: Vec::new(),
            doctor_rx: None,
        }
    }

    fn start_doctor(&mut self) {
        self.doctor = DoctorState::Running;
        self.doctor_rx = Some(spawn_doctor_run());
    }
}

pub(crate) fn run_session(child: &mut Child, verbose: bool, plugins: usize) -> Result<SessionEnd> {
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
    health: &Receiver<bool>,
    dash: &mut Dash,
) -> Result<SessionEnd> {
    loop {
        while let Ok(line) = lines.try_recv() {
            dash.logs.push(line);
        }
        while let Ok(up) = health.try_recv() {
            dash.health = if up { Health::Up } else { Health::Down };
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
        if let Some(status) = try_wait(child)? {
            while let Ok(line) = lines.try_recv() {
                dash.logs.push(line);
            }
            return Ok(SessionEnd::ChildExited(status));
        }
        terminal.draw(|frame| draw(frame, dash))?;
        if let Some(action) = poll_action()? {
            match action {
                Action::Quit => {
                    stop_child(child)?;
                    return Ok(SessionEnd::UserQuit);
                }
                other => apply_action(dash, other),
            }
        }
    }
}

fn poll_action() -> Result<Option<Action>> {
    if !event::poll(TICK)? {
        return Ok(None);
    }
    let Event::Key(key) = event::read()? else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(Some(action_for(key.code, key.modifiers)))
}

fn apply_action(dash: &mut Dash, action: Action) {
    let page = dash.log_height.max(1);
    match action {
        Action::ToggleView => {
            dash.view = match dash.view {
                View::Logs => View::Dashboard,
                _ => View::Logs,
            };
            dash.scroll_offset = 0;
        }
        Action::Rebuild => {
            dash.rebuild = match post_recompile_current() {
                Ok(()) => RebuildState::Requested(Instant::now()),
                Err(error) => RebuildState::Failed(format!("{error:#}")),
            };
        }
        Action::ReloadPlugins => {
            dash.plugin_reload = match post_reload_plugins() {
                Ok(()) => RebuildState::Requested(Instant::now()),
                Err(error) => RebuildState::Failed(format!("{error:#}")),
            };
        }
        Action::Doctor => {
            dash.view = View::Doctor;
            dash.scroll_offset = 0;
            dash.start_doctor();
        }
        Action::Back => {
            dash.view = View::Dashboard;
            dash.scroll_offset = 0;
        }
        Action::ScrollUp => dash.scroll_offset = dash.scroll_offset.saturating_add(1),
        Action::ScrollDown => dash.scroll_offset = dash.scroll_offset.saturating_sub(1),
        Action::PageUp => dash.scroll_offset = dash.scroll_offset.saturating_add(page),
        Action::PageDown => dash.scroll_offset = dash.scroll_offset.saturating_sub(page),
        Action::Follow => dash.scroll_offset = 0,
        Action::Quit | Action::Ignore => {}
    }
    dash.scroll_offset = clamp_offset(dash.logs.len(), dash.log_height, dash.scroll_offset);
}

fn draw(frame: &mut Frame, dash: &mut Dash) {
    let [main, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    match dash.view {
        View::Dashboard => draw_dashboard(frame, dash, main),
        View::Logs => draw_logs(frame, dash, main),
        View::Doctor => draw_doctor(frame, dash, main),
    }
    let keys = match dash.view {
        View::Dashboard => " L logs · ^R rebuild · ^P plugins · d doctor · q quit ",
        View::Logs => {
            " L dashboard · ^R rebuild · ^P plugins · d doctor · ↑/↓ scroll · f follow · q quit "
        }
        View::Doctor => " d refresh · L logs · ↑/↓ scroll · esc back · q quit ",
    };
    frame.render_widget(Paragraph::new(keys).style(Style::new().dim()), footer);
}

fn draw_dashboard(frame: &mut Frame, dash: &Dash, area: ratatui::layout::Rect) {
    let (health_text, health_color) = match dash.health {
        Health::Checking => ("starting", Color::Yellow),
        Health::Up => ("running", Color::Green),
        Health::Down => ("down", Color::Red),
    };
    let rebuild_row = match &dash.rebuild {
        RebuildState::Idle => Line::from("  rebuild   ctrl+r to trigger"),
        RebuildState::Requested(at) => Line::from(format!(
            "  rebuild   requested {} ago",
            format_duration(at.elapsed())
        )),
        RebuildState::Failed(error) => Line::from(vec![
            "  rebuild   ".into(),
            "failed".fg(Color::Red).bold(),
            format!(" · {error}").into(),
        ]),
    };
    let plugins_row = match &dash.plugin_reload {
        RebuildState::Idle => Line::from(format!(
            "  plugins   {} linked · ctrl+p to reload",
            dash.plugins
        )),
        RebuildState::Requested(at) => Line::from(format!(
            "  plugins   {} linked · reload requested {} ago",
            dash.plugins,
            format_duration(at.elapsed())
        )),
        RebuildState::Failed(error) => Line::from(vec![
            format!("  plugins   {} linked · reload ", dash.plugins).into(),
            "failed".fg(Color::Red).bold(),
            format!(" · {error}").into(),
        ]),
    };
    let doctor_row = match &dash.doctor {
        DoctorState::Running => Line::from("  doctor    checking · d to run"),
        DoctorState::Done(report) if report.divergences() == 0 => Line::from(vec![
            "  doctor    ".into(),
            "all good".fg(Color::Green).bold(),
            format!(" · {} checks · d to run", report.ok).into(),
        ]),
        DoctorState::Done(report) => Line::from(vec![
            "  doctor    ".into(),
            format!("{} divergences", report.divergences())
                .fg(Color::Yellow)
                .bold(),
            format!(
                " · {} warn · {} err · d to run",
                report.warn,
                report.error + report.crash
            )
            .into(),
        ]),
        DoctorState::Failed(error) => Line::from(vec![
            "  doctor    ".into(),
            "failed".fg(Color::Red).bold(),
            format!(" · {error} · d to run").into(),
        ]),
    };
    let rows = vec![
        Line::from(vec![
            "  tray      ".into(),
            health_text.fg(health_color).bold(),
            format!(" · up {}", format_duration(dash.started.elapsed())).into(),
        ]),
        plugins_row,
        rebuild_row,
        doctor_row,
        Line::from(format!(
            "  logs      {} buffered · L to view",
            dash.logs.len()
        )),
    ];
    let block = Block::bordered().title(" qol dev ");
    frame.render_widget(Paragraph::new(rows).block(block), area);
}

fn draw_logs(frame: &mut Frame, dash: &mut Dash, area: ratatui::layout::Rect) {
    let total = dash.logs.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = dash
        .logs
        .lines
        .iter()
        .skip(start)
        .take(height)
        .map(|line| styled_line(line))
        .collect();
    let title = format!(" logs · {} ", list_status(total, dash.scroll_offset));
    frame.render_widget(
        Paragraph::new(visible).block(Block::bordered().title(title)),
        area,
    );
}

fn draw_doctor(frame: &mut Frame, dash: &mut Dash, area: ratatui::layout::Rect) {
    if dash.doctor_lines.is_empty() {
        let message = match dash.doctor {
            DoctorState::Running => "  running checks",
            _ => "  no checks reported · press d to run",
        };
        frame.render_widget(
            Paragraph::new(message).block(Block::bordered().title(" doctor ")),
            area,
        );
        return;
    }
    let total = dash.doctor_lines.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = dash
        .doctor_lines
        .iter()
        .skip(start)
        .take(height)
        .map(|line| styled_doctor_line(line))
        .collect();
    let title = format!(" doctor · {} ", list_status(total, dash.scroll_offset));
    frame.render_widget(
        Paragraph::new(visible).block(Block::bordered().title(title)),
        area,
    );
}

fn list_window(dash: &mut Dash, area: ratatui::layout::Rect, total: usize) -> (usize, usize) {
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

fn spawn_health_probe() -> Receiver<bool> {
    let (tx, rx) = channel();
    std::thread::spawn(move || loop {
        if tx.send(health_ok()).is_err() {
            return;
        }
        std::thread::sleep(HEALTH_PROBE_INTERVAL);
    });
    rx
}

fn spawn_doctor_run() -> Receiver<Result<DoctorRun, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_doctor_once());
    });
    rx
}

fn run_doctor_once() -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let binary = root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray-doctor"));
    let output = Command::new(&binary)
        .current_dir(&root)
        .arg("check")
        .output()
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let report =
        parse_doctor_summary(&text).ok_or_else(|| "could not read doctor summary".to_string())?;
    let lines = text
        .lines()
        .filter(|line| line.trim_start().starts_with('['))
        .map(|line| line.to_string())
        .collect();
    Ok(DoctorRun { report, lines })
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
    fn action_for_maps_keys() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let cases = [
            (KeyCode::Char('l'), none, Action::ToggleView),
            (KeyCode::Char('L'), none, Action::ToggleView),
            (KeyCode::Char('d'), none, Action::Doctor),
            (KeyCode::Char('D'), none, Action::Doctor),
            (KeyCode::Esc, none, Action::Back),
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
            (KeyCode::Char('r'), none, Action::Ignore),
            (KeyCode::Char('p'), none, Action::Ignore),
            (KeyCode::Char('x'), none, Action::Ignore),
        ];
        for (code, mods, expected) in cases {
            assert_eq!(action_for(code, mods), expected, "{code:?} {mods:?}");
        }
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
