use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qol_conventions::doctor_wire::{
    FixReport as DoctorFixReport, Outcome, OutcomeStatus, Report as DoctorReport,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::poller::Poller;

use super::activity::Activity;
use super::render_util::{
    accent, caret, cursor_window_start, ellipsize_line, list_capacity, now_unix_ms, panel_width,
    relative_age, render_bottom_panel, view_content, wrapped_rows, NavigationOverflow,
};
use super::{Dash, View, DOCTOR_BASE_INTERVAL, DOCTOR_CAP_INTERVAL};

pub(super) fn spawn_doctor_probe() -> Poller<Result<DoctorRun, String>> {
    Poller::spawn_adaptive(
        DOCTOR_BASE_INTERVAL,
        DOCTOR_CAP_INTERVAL,
        run_doctor_prebuilt,
    )
}

pub(super) struct DoctorPanel {
    pub(super) last: Option<DoctorRun>,
    pub(super) last_at_ms: Option<u64>,
    pub(super) manual: Option<ManualDoctor>,
    pub(super) error: Option<String>,
}

pub(super) struct ManualDoctor {
    pub(super) mode: DoctorMode,
    pub(super) rx: Receiver<Result<DoctorRun, String>>,
    progress: Arc<Mutex<String>>,
    started_at_ms: u64,
}

const DOCTOR_PREBUILT_ERROR: &str =
    "doctor binary is not prebuilt or is stale · press ctrl+r to compile all dev binaries";

impl ManualDoctor {
    fn progress_step(&self) -> String {
        self.progress
            .lock()
            .map(|step| step.clone())
            .unwrap_or_default()
    }

    pub(super) fn activity(&self, now_ms: u64) -> Activity {
        Activity {
            title: "doctor",
            phase: self.mode.gerund().to_string(),
            detail: self.progress_step(),
            elapsed: Duration::from_millis(now_ms.saturating_sub(self.started_at_ms)),
        }
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct DoctorRun {
    report: DoctorReport,
    lines: Vec<String>,
    details: Vec<String>,
    scope: DoctorScope,
}

pub(super) fn apply_doctor_outcome(dash: &mut Dash, outcome: Result<DoctorRun, String>) {
    match outcome {
        Ok(run) => {
            dash.doctor.last = Some(run);
            dash.doctor.last_at_ms = Some(now_unix_ms());
            dash.doctor.error = None;
        }
        Err(error) => dash.doctor.error = Some(error),
    }
}

pub(super) fn open_doctor(dash: &mut Dash) {
    dash.view = View::Doctor;
    dash.scroll_offset = 0;
    dash.doctor_cursor = 0;
    dash.doctor_detail_open = false;
    dash.pokes.doctor = true;
}

pub(super) fn toggle_doctor_detail(dash: &mut Dash) {
    if dash.doctor_detail_open {
        dash.doctor_detail_open = false;
        return;
    }
    dash.doctor_detail_open = doctor_detail_text(&dash.doctor, dash.doctor_cursor).is_some();
}

pub(super) fn doctor_detail_text(panel: &DoctorPanel, cursor: usize) -> Option<String> {
    let error_rows = usize::from(panel.error.is_some());
    if cursor < error_rows {
        return panel.error.clone();
    }
    let run = panel.last.as_ref()?;
    run.details.get(cursor - error_rows).cloned()
}

pub(super) fn doctor_status(panel: &DoctorPanel, now_ms: u64) -> (Color, Vec<Span<'static>>) {
    if let Some(manual) = &panel.manual {
        return (Color::Yellow, vec![manual.mode.gerund().fg(Color::Yellow)]);
    }
    let Some(run) = &panel.last else {
        let detail = panel
            .error
            .clone()
            .unwrap_or_else(|| "waiting for first check".to_string());
        return (Color::Yellow, vec![detail.fg(Color::DarkGray)]);
    };
    let report = &run.report;
    let (color, mut value) = if report.divergence_count() == 0 {
        (
            accent(),
            vec![
                "all good".fg(accent()).bold(),
                match run.scope {
                    DoctorScope::Full => format!(" · {} checks", report.count_ok()),
                    DoctorScope::Quick => format!(" · {} quick checks", report.count_ok()),
                }
                .fg(Color::DarkGray),
            ],
        )
    } else {
        let error_count = report.count_error() + report.count_crash();
        let color = if error_count > 0 {
            Color::Red
        } else {
            Color::Yellow
        };
        let label = format!("{} divergences", report.divergence_count());
        (
            color,
            vec![
                label.fg(color).bold(),
                format!(" · {} warn · {} err", report.count_warn(), error_count)
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

const ROW_MAX_WIDTH: usize = 80;

pub(super) fn draw_doctor(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let lines = doctor_view_lines(&dash.doctor);
    if lines.is_empty() {
        if dash.doctor.manual.is_none() {
            view_content(
                frame,
                area,
                vec![Line::from("  no checks reported · press d to run")],
            );
        }
        return NavigationOverflow::default();
    }
    let total = lines.len();
    let height = list_capacity(area.height);
    dash.log_height = height;
    if dash.doctor_cursor >= total {
        dash.doctor_cursor = total - 1;
    }
    let cursor = dash.doctor_cursor;
    let start = cursor_window_start(total, height, cursor);
    let render = if dash.armed {
        styled_doctor_line
    } else {
        friendly_doctor_line
    };
    let row_width = (area.width as usize).min(ROW_MAX_WIDTH);
    let visible: Vec<Line> = lines
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, line)| ellipsize_line(render(line, index == cursor), row_width))
        .collect();
    view_content(frame, area, visible);
    let selected_overflows = lines
        .get(cursor)
        .is_some_and(|line| render(line, true).width() > row_width);
    if dash.doctor_detail_open || selected_overflows {
        if let Some(detail) = doctor_detail_text(&dash.doctor, cursor) {
            let width = panel_width(area).saturating_sub(2) as usize;
            let detail_color = lines
                .get(cursor)
                .and_then(|line| doctor_line_style(line).map(|(_, color)| color))
                .unwrap_or(accent());
            render_bottom_panel(
                frame,
                area,
                "details",
                wrapped_rows(&detail, width),
                detail_color,
            );
        }
    }
    NavigationOverflow::from_window(start, height, total)
}

pub(super) fn doctor_scroll_len(panel: &DoctorPanel) -> usize {
    doctor_view_lines(panel).len()
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

fn doctor_line_style(raw: &str) -> Option<(&'static str, Color)> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with("[OK]") {
        return Some(("✓", accent()));
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

fn styled_doctor_line(raw: &str, selected: bool) -> Line<'static> {
    let Some((symbol, color)) = doctor_line_style(raw) else {
        return Line::from(vec![caret(selected), raw.trim_start().to_string().into()]);
    };
    let rest = raw
        .trim_start()
        .split_once(']')
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .trim_start();
    Line::from(vec![
        caret(selected),
        format!("{symbol} ").fg(color).bold(),
        rest.to_string().into(),
    ])
}

fn friendly_doctor_line(raw: &str, selected: bool) -> Line<'static> {
    let Some((symbol, color)) = doctor_line_style(raw) else {
        return Line::from(vec![caret(selected), raw.trim_start().to_string().into()]);
    };
    let rest = raw
        .trim_start()
        .split_once(']')
        .map(|(_, rest)| rest.trim_start())
        .unwrap_or("");
    let Some((id, message)) = rest.split_once(": ") else {
        return Line::from(vec![
            caret(selected),
            format!("{symbol} ").fg(color).bold(),
            rest.to_string().into(),
        ]);
    };
    let head = format!("{symbol} {}", humanize_check_id(id));
    if symbol == "✓" {
        return Line::from(vec![caret(selected), head.fg(color).bold()]);
    }
    let detail = message
        .split_once('\u{2014}')
        .map_or(message, |(_, tail)| tail)
        .trim();
    Line::from(vec![
        caret(selected),
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

#[derive(Clone, Copy)]
pub(super) enum DoctorMode {
    Check,
    Fix,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DoctorScope {
    Full,
    Quick,
}

const QUICK_DOCTOR_CHECK_ARGS: &[&str] = &[
    qol_conventions::doctor_cli::ARG_CHECK,
    qol_conventions::doctor_cli::ARG_QUICK,
    qol_conventions::doctor_cli::ARG_JSON,
];

impl DoctorMode {
    fn full_command_args(self) -> &'static [&'static str] {
        match self {
            DoctorMode::Check => &[
                qol_conventions::doctor_cli::ARG_CHECK,
                qol_conventions::doctor_cli::ARG_JSON,
            ],
            DoctorMode::Fix => &[
                qol_conventions::doctor_cli::ARG_FIX,
                qol_conventions::doctor_cli::ARG_JSON,
            ],
        }
    }

    fn gerund(self) -> &'static str {
        match self {
            DoctorMode::Check => "checking",
            DoctorMode::Fix => "fixing",
        }
    }
}

pub(super) fn spawn_doctor(mode: DoctorMode) -> ManualDoctor {
    let (tx, rx) = channel();
    let progress = Arc::new(Mutex::new(String::new()));
    let worker_progress = Arc::clone(&progress);
    std::thread::spawn(move || {
        let _ = tx.send(run_doctor(mode, &worker_progress));
    });
    ManualDoctor {
        mode,
        rx,
        progress,
        started_at_ms: now_unix_ms(),
    }
}

fn run_doctor(mode: DoctorMode, progress: &Arc<Mutex<String>>) -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let binary = resolve_manual_doctor_binary(&root, progress)?;
    run_doctor_streaming(&binary, &root, mode, mode.full_command_args(), progress)
}

fn resolve_manual_doctor_binary(
    root: &std::path::Path,
    progress: &Arc<Mutex<String>>,
) -> Result<std::path::PathBuf, String> {
    set_progress(progress, "using prebuilt doctor binary");
    let binary = prebuilt_doctor_binary_path(root);
    verify_prebuilt_doctor_binary(root, &binary)?;
    Ok(binary)
}

fn set_progress(progress: &Arc<Mutex<String>>, step: &str) {
    if let Ok(mut current) = progress.lock() {
        *current = step.to_string();
    }
}

fn run_doctor_streaming(
    binary: &std::path::Path,
    root: &std::path::Path,
    mode: DoctorMode,
    args: &[&str],
    progress: &Arc<Mutex<String>>,
) -> Result<DoctorRun, String> {
    let mut child = Command::new(binary)
        .current_dir(root)
        .args(args)
        .env(qol_conventions::doctor_cli::PROGRESS_ENV_VAR, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stderr = child.stderr.take().ok_or("doctor stderr unavailable")?;
    let reader_progress = Arc::clone(progress);
    let stderr_reader = std::thread::spawn(move || {
        let mut other_lines = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            match line.strip_prefix(qol_conventions::doctor_cli::PROGRESS_LINE_PREFIX) {
                Some(step) => set_progress(&reader_progress, step),
                None => {
                    other_lines.push_str(&line);
                    other_lines.push('\n');
                }
            }
        }
        other_lines
    });
    let mut stdout = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    let _ = child.wait().map_err(|error| error.to_string())?;
    let stderr_rest = stderr_reader.join().unwrap_or_default();
    parse_doctor_run(&stdout, &stderr_rest, mode, DoctorScope::Full)
}

fn run_doctor_prebuilt() -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let binary = prebuilt_doctor_binary_path(&root);
    if !binary.is_file() {
        return Err(DOCTOR_PREBUILT_ERROR.to_string());
    }
    run_doctor_binary(
        &binary,
        &root,
        DoctorMode::Check,
        DoctorScope::Quick,
        QUICK_DOCTOR_CHECK_ARGS,
    )
}

fn prebuilt_doctor_binary_path(root: &std::path::Path) -> std::path::PathBuf {
    qol_dev_build::tray::debug_binary_path(root, qol_conventions::artifact::TRAY_DOCTOR_BINARY_NAME)
}

fn verify_prebuilt_doctor_binary(
    root: &std::path::Path,
    binary: &std::path::Path,
) -> Result<(), String> {
    if !binary.is_file() {
        return Err(DOCTOR_PREBUILT_ERROR.to_string());
    }
    let identity = qol_build_identity::BuildIdentityEnvironment::development(
        &qol_dev_build::tray::artifact_root(root),
    )
    .map_err(|error| format!("cannot verify prebuilt doctor binary: {error}"))?;
    let expectation = qol_artifact::ArtifactExpectation::development_debug(
        qol_conventions::artifact::TRAY_DOCTOR_BINARY_NAME,
        qol_conventions::artifact::TRAY_PACKAGE_NAME,
        qol_conventions::artifact::BuildRole::Doctor,
        true,
    )
    .with_exact_source(identity.source());
    qol_artifact::verify_path(binary, &expectation)
        .map(|_| ())
        .map_err(|error| format!("{DOCTOR_PREBUILT_ERROR}: {error}"))
}

fn run_doctor_binary(
    binary: &std::path::Path,
    root: &std::path::Path,
    mode: DoctorMode,
    scope: DoctorScope,
    args: &[&str],
) -> Result<DoctorRun, String> {
    let output = Command::new(binary)
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    parse_doctor_run(
        &output.stdout,
        &String::from_utf8_lossy(&output.stderr),
        mode,
        scope,
    )
}

fn parse_doctor_run(
    stdout: &[u8],
    stderr: &str,
    mode: DoctorMode,
    scope: DoctorScope,
) -> Result<DoctorRun, String> {
    let (report, lines, details) = parse_doctor_output(stdout, mode).map_err(|error| {
        let detail = stderr.trim();
        if detail.is_empty() {
            format!("could not read doctor report: {error}")
        } else {
            format!("could not read doctor report: {error}: {detail}")
        }
    })?;
    Ok(DoctorRun {
        report,
        lines,
        details,
        scope,
    })
}

type ParsedDoctorOutput = (DoctorReport, Vec<String>, Vec<String>);

fn parse_doctor_output(
    bytes: &[u8],
    mode: DoctorMode,
) -> Result<ParsedDoctorOutput, serde_json::Error> {
    match mode {
        DoctorMode::Check => {
            let report: DoctorReport = serde_json::from_slice(bytes)?;
            let (lines, details) = report_entries(&report);
            Ok((report, lines, details))
        }
        DoctorMode::Fix => {
            let fix_report: DoctorFixReport = serde_json::from_slice(bytes)?;
            let mut lines = fix_report
                .failures
                .iter()
                .map(|failure| format!("[ERR] {}", first_line(failure)))
                .collect::<Vec<_>>();
            let mut details = fix_report.failures.clone();
            let (after_lines, after_details) = report_entries(&fix_report.after);
            lines.extend(after_lines);
            details.extend(after_details);
            Ok((fix_report.after, lines, details))
        }
    }
}

fn report_entries(report: &DoctorReport) -> (Vec<String>, Vec<String>) {
    let mut outcomes: Vec<&Outcome> = report.outcomes.iter().collect();
    outcomes.sort_by_key(|outcome| status_rank(outcome.status));
    outcomes
        .into_iter()
        .map(|outcome| (outcome_line(outcome), outcome.message.clone()))
        .unzip()
}

fn status_rank(status: OutcomeStatus) -> u8 {
    match status {
        OutcomeStatus::Crash => 0,
        OutcomeStatus::Error => 1,
        OutcomeStatus::Warn => 2,
        OutcomeStatus::Ok => 3,
    }
}

fn outcome_line(outcome: &Outcome) -> String {
    let fix = if outcome.fix_available {
        " (fix available)"
    } else {
        ""
    };
    format!(
        "[{}] {}: {}{fix}",
        outcome_status_label(outcome.status),
        outcome.id,
        first_line(&outcome.message)
    )
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or_default()
}

fn outcome_status_label(status: OutcomeStatus) -> &'static str {
    match status {
        OutcomeStatus::Ok => "OK",
        OutcomeStatus::Warn => "WARN",
        OutcomeStatus::Error => "ERR",
        OutcomeStatus::Crash => "CRASH",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::key_bindings::Action;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use std::fs;

    fn span_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn manual_doctor_resolves_the_prebuilt_dev_binary_path() {
        let root = tempfile::tempdir().unwrap();
        let binary = qol_dev_build::tray::debug_binary_path(
            root.path(),
            qol_conventions::artifact::TRAY_DOCTOR_BINARY_NAME,
        );
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"prebuilt").unwrap();
        let progress = Arc::new(Mutex::new(String::new()));

        let resolved = prebuilt_doctor_binary_path(root.path());

        assert_eq!(resolved, binary);
        assert_eq!(progress.lock().unwrap().as_str(), "");
    }

    #[test]
    fn manual_doctor_reports_when_the_prebuilt_dev_binary_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let progress = Arc::new(Mutex::new(String::new()));

        let error = resolve_manual_doctor_binary(root.path(), &progress).unwrap_err();

        assert_eq!(error, DOCTOR_PREBUILT_ERROR);
        assert_eq!(
            progress.lock().unwrap().as_str(),
            "using prebuilt doctor binary"
        );
    }

    fn outcome(id: &str, status: OutcomeStatus, message: &str, fix_available: bool) -> Outcome {
        Outcome {
            id: id.to_string(),
            status,
            message: message.to_string(),
            fix_available,
        }
    }

    fn make_report(ok: usize, warn: usize, error: usize, crash: usize) -> DoctorReport {
        let mut outcomes = Vec::new();
        for (label, status, count) in [
            ("ok", OutcomeStatus::Ok, ok),
            ("warn", OutcomeStatus::Warn, warn),
            ("error", OutcomeStatus::Error, error),
            ("crash", OutcomeStatus::Crash, crash),
        ] {
            outcomes.extend((0..count).map(|index| Outcome {
                id: format!("{label}_{index}"),
                status,
                message: "message".to_string(),
                fix_available: false,
            }));
        }
        DoctorReport::new(outcomes)
    }

    #[test]
    fn doctor_status_covers_panel_states() {
        let report = make_report(11, 0, 0, 0);
        let run = DoctorRun {
            report: report.clone(),
            lines: Vec::new(),
            details: Vec::new(),
            scope: DoctorScope::Full,
        };
        let quick_run = DoctorRun {
            report,
            lines: Vec::new(),
            details: Vec::new(),
            scope: DoctorScope::Quick,
        };
        let warn_report = make_report(9, 2, 0, 0);
        let warn_run = DoctorRun {
            report: warn_report.clone(),
            lines: Vec::new(),
            details: Vec::new(),
            scope: DoctorScope::Full,
        };
        let quick_warn_run = DoctorRun {
            report: warn_report,
            lines: Vec::new(),
            details: Vec::new(),
            scope: DoctorScope::Quick,
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
                    last: Some(quick_run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: None,
                },
                Color::Green,
                "all good · 11 quick checks · 15s ago",
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
                    last: Some(quick_warn_run),
                    last_at_ms: Some(now - 5_000),
                    manual: None,
                    error: None,
                },
                Color::Yellow,
                "2 divergences · 2 warn · 0 err · just now",
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
                    last: Some(quick_run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: Some("boom".to_string()),
                },
                Color::Green,
                "all good · 11 quick checks · 15s ago · probe failed",
            ),
            (
                DoctorPanel {
                    last: None,
                    last_at_ms: None,
                    manual: None,
                    error: Some(DOCTOR_PREBUILT_ERROR.to_string()),
                },
                Color::Yellow,
                DOCTOR_PREBUILT_ERROR,
            ),
        ];
        for (panel, expected_color, expected_text) in cases {
            let (color, spans) = doctor_status(&panel, now);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }

    #[test]
    fn report_lines_sort_divergences_first() {
        let report = DoctorReport::new(vec![
            outcome("a", OutcomeStatus::Ok, "x", false),
            outcome("b", OutcomeStatus::Warn, "y", false),
            outcome("c", OutcomeStatus::Crash, "z", false),
            outcome("d", OutcomeStatus::Error, "w", false),
        ]);
        let (lines, details) = report_entries(&report);
        assert_eq!(
            lines,
            vec!["[CRASH] c: z", "[ERR] d: w", "[WARN] b: y", "[OK] a: x"],
            "divergences must never hide below the ok lines"
        );
        assert_eq!(
            details,
            vec!["z", "w", "y", "x"],
            "details must stay aligned with their sorted rows"
        );
    }

    fn manual_doctor(mode: DoctorMode, step: &str, started_at_ms: u64) -> ManualDoctor {
        let (_, rx) = channel();
        ManualDoctor {
            mode,
            rx,
            progress: Arc::new(Mutex::new(step.to_string())),
            started_at_ms,
        }
    }

    #[test]
    fn doctor_status_leaves_the_running_step_to_the_activity_sign() {
        let panel = DoctorPanel {
            last: None,
            last_at_ms: None,
            manual: Some(manual_doctor(
                DoctorMode::Fix,
                "check rust_clippy",
                1_000_000_000 - 83_000,
            )),
            error: None,
        };
        let (color, spans) = doctor_status(&panel, 1_000_000_000);
        assert_eq!(color, Color::Yellow);
        assert_eq!(span_text(&spans), "fixing");
    }

    #[test]
    fn running_fixes_hold_the_frame_yellow_after_the_arm_is_consumed() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = false;
        dash.doctor.manual = Some(manual_doctor(DoctorMode::Fix, "", now_unix_ms()));
        assert!(dash.is_busy());
        assert_eq!(
            super::super::draw::frame_accent(&dash),
            Color::Yellow,
            "the frame must not fall back to green while fixes run"
        );

        dash.doctor.manual = None;
        assert_eq!(super::super::draw::frame_accent(&dash), Color::Green);
    }

    #[test]
    fn running_fixes_show_in_the_activity_sign_instead_of_the_page_body() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Doctor;
        dash.doctor.manual = Some(manual_doctor(
            DoctorMode::Fix,
            "check rust_clippy",
            now_unix_ms(),
        ));

        let rows = super::super::testkit::render_rows_at(&mut dash, 110, 28);

        let border = &rows[rows.len() - 2];
        assert!(
            border.contains("┤ doctor"),
            "doctor activity sign title missing from the bottom border: {border}"
        );
        assert!(
            border.contains("fixing · check rust_clippy"),
            "doctor activity step missing from the sign: {border}"
        );
        assert!(
            rows.iter().all(|row| !row.contains("no checks reported")),
            "the idle empty state must not claim idleness while fixes run"
        );
    }

    #[test]
    fn manual_doctor_reports_mode_step_and_elapsed_as_an_activity() {
        let cases = [
            (DoctorMode::Fix, "check rust_clippy", "fixing"),
            (DoctorMode::Check, "", "checking"),
        ];
        for (mode, step, expected_phase) in cases {
            let manual = manual_doctor(mode, step, 1_000_000_000 - 83_000);
            let activity = manual.activity(1_000_000_000);
            assert_eq!(activity.title, "doctor");
            assert_eq!(activity.phase, expected_phase);
            assert_eq!(activity.detail, step);
            assert_eq!(activity.elapsed, Duration::from_secs(83));
        }
    }

    #[test]
    fn friendly_doctor_line_hides_detail_for_ok_and_keeps_it_for_warn() {
        let ok = friendly_doctor_line(
            "[OK] install_identity: marker and id are aligned (x)",
            false,
        );
        assert_eq!(
            ok.to_string(),
            "  ✓ Install identity",
            "ok hides the detail"
        );

        let warn = friendly_doctor_line(
            "[WARN] plugin_staleness: plugin staleness detected \u{2014} rebuild required: a, b",
            false,
        );
        assert_eq!(
            warn.to_string(),
            "  ▲ Plugin staleness - rebuild required: a, b",
            "warn humanizes the name and keeps the post-dash detail",
        );
    }

    #[test]
    fn doctor_cursor_moves_and_clamps_without_scrolling() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Doctor;
        dash.doctor.last = Some(DoctorRun {
            report: make_report(3, 0, 0, 0),
            lines: vec![
                "[OK] a: x".to_string(),
                "[OK] b: y".to_string(),
                "[OK] c: z".to_string(),
            ],
            details: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            scope: DoctorScope::Full,
        });
        let moves = [
            (Action::ScrollDown, 1),
            (Action::ScrollDown, 2),
            (Action::ScrollDown, 2),
            (Action::ScrollUp, 1),
            (Action::ScrollUp, 0),
            (Action::ScrollUp, 0),
        ];
        for (action, expected) in moves {
            super::super::session::apply_action(&mut dash, action, false);
            assert_eq!(dash.doctor_cursor, expected, "after {action:?}");
            assert_eq!(dash.scroll_offset, 0, "after {action:?}");
        }
    }

    fn panel_with_details(details: Vec<&str>, error: Option<&str>) -> DoctorPanel {
        DoctorPanel {
            last: Some(DoctorRun {
                report: make_report(details.len(), 0, 0, 0),
                lines: details.iter().map(|d| format!("[OK] a: {d}")).collect(),
                details: details.iter().map(|d| d.to_string()).collect(),
                scope: DoctorScope::Full,
            }),
            last_at_ms: None,
            manual: None,
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn doctor_detail_text_offsets_past_the_error_row() {
        let cases = [
            (panel_with_details(vec!["x", "y"], None), 0, Some("x")),
            (panel_with_details(vec!["x", "y"], None), 1, Some("y")),
            (panel_with_details(vec!["x", "y"], None), 2, None),
            (panel_with_details(vec!["x"], Some("boom")), 0, Some("boom")),
            (panel_with_details(vec!["x"], Some("boom")), 1, Some("x")),
        ];
        for (index, (panel, cursor, expected)) in cases.into_iter().enumerate() {
            assert_eq!(
                doctor_detail_text(&panel, cursor).as_deref(),
                expected,
                "case: {index}"
            );
        }
    }

    #[test]
    fn enter_opens_detail_panel_and_left_leaves_the_doctor_page() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Doctor;
        dash.doctor = panel_with_details(vec!["full message"], None);

        super::super::session::handle_key(&mut dash, KeyCode::Enter, KeyModifiers::NONE);
        assert!(dash.doctor_detail_open, "enter opens the detail panel");

        super::super::session::handle_key(&mut dash, KeyCode::Left, KeyModifiers::NONE);
        assert!(
            !dash.doctor_detail_open,
            "leaving the page clears detail state"
        );
        assert_eq!(dash.view, View::Dashboard, "left leaves the doctor page");
    }

    #[test]
    fn overflowing_selected_row_caps_at_the_row_width_and_auto_opens_details() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Doctor;
        dash.keys_hidden = true;
        let long = format!("stale grab for Super+Space {}", "detail ".repeat(20));
        dash.doctor.last = Some(DoctorRun {
            report: make_report(1, 0, 0, 0),
            lines: vec![format!("[WARN] hotkey_shadows: {long}")],
            details: vec![long.clone()],
            scope: DoctorScope::Full,
        });

        let rows = super::super::testkit::render_rows_at(&mut dash, 110, 28);

        let row = rows
            .iter()
            .find(|row| row.contains("▲"))
            .expect("warn row missing");
        assert!(
            row.contains('…'),
            "capped row must end in an ellipsis: {row:?}"
        );
        let ellipsis_at = row.chars().position(|ch| ch == '…').unwrap();
        assert!(
            ellipsis_at <= ROW_MAX_WIDTH + 4,
            "row must cap at {ROW_MAX_WIDTH} cells inside a 110-wide frame: {row:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("┤ details ├")),
            "overflowing selection must auto-open the detail panel"
        );
    }

    #[test]
    fn open_detail_panel_keeps_the_navigation_cue_visible() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Doctor;
        dash.keys_hidden = true;
        let details: Vec<String> = (0..40).map(|index| format!("detail {index}")).collect();
        dash.doctor = panel_with_details(details.iter().map(String::as_str).collect(), None);
        dash.doctor_detail_open = true;

        let rows = super::super::testkit::render_rows_at(&mut dash, 110, 28);

        assert!(
            rows.iter().any(|row| row.contains("┤ details ├")),
            "detail panel missing"
        );
        assert!(
            rows.iter().any(|row| row.contains("│ v │")),
            "below-overflow cue must stay visible while the panel is open"
        );
    }

    #[test]
    fn activity_sign_sits_on_the_border_instead_of_covering_the_detail_panel() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Doctor;
        dash.keys_hidden = true;
        dash.doctor = panel_with_details(vec!["full message shown in the panel"], None);
        dash.doctor_detail_open = true;
        dash.doctor.manual = Some(manual_doctor(
            DoctorMode::Fix,
            "check rust_clippy",
            now_unix_ms(),
        ));

        let rows = super::super::testkit::render_rows_at(&mut dash, 110, 28);

        let details_row = rows
            .iter()
            .position(|row| row.contains("┤ details ├"))
            .expect("details title missing");
        let activity_row = rows
            .iter()
            .position(|row| row.contains("┤ doctor"))
            .expect("activity title missing");
        assert!(
            activity_row > details_row,
            "the activity sign must sit on the bottom border, below the detail panel"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("full message shown in the panel")),
            "detail body must stay intact under the stacked signs"
        );
    }

    #[test]
    fn detail_panel_shows_the_full_message() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Doctor;
        dash.doctor = panel_with_details(
            vec!["first line\nsecond line that only the detail panel shows"],
            None,
        );
        dash.doctor_detail_open = true;

        let rows = super::super::testkit::render_rows_at(&mut dash, 110, 28);

        assert!(
            rows.iter().any(|row| row.contains("┤ details ├")),
            "detail panel title missing"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("second line that only the detail panel shows")),
            "full message missing from the detail panel"
        );
    }

    #[test]
    fn doctor_lines_carry_the_shared_selection_caret() {
        let cases = [
            (
                friendly_doctor_line("[OK] install_identity: aligned", true),
                "▸ ✓ Install identity",
            ),
            (
                styled_doctor_line("[WARN] plugin_staleness: stale", true),
                "▸ ▲ plugin_staleness: stale",
            ),
            (styled_doctor_line("Summary: ok=1", true), "▸ Summary: ok=1"),
        ];
        for (line, expected) in cases {
            assert_eq!(line.to_string(), expected);
        }
    }

    #[test]
    fn typed_check_output_preserves_outcomes() {
        let expected = DoctorReport::new(vec![
            outcome("a", OutcomeStatus::Warn, "x", true),
            outcome("b", OutcomeStatus::Ok, "y\ndetail", false),
        ]);
        let json = serde_json::to_vec(&expected).unwrap();

        let (actual, lines, details) = parse_doctor_output(&json, DoctorMode::Check).unwrap();

        assert_eq!(actual, expected);
        assert_eq!(
            lines,
            vec![
                "[WARN] a: x (fix available)".to_string(),
                "[OK] b: y".to_string(),
            ]
        );
        assert_eq!(
            details,
            vec!["x".to_string(), "y\ndetail".to_string()],
            "details must keep the full multiline message"
        );
    }

    #[test]
    fn typed_fix_output_keeps_failures_and_after_report() {
        let before = DoctorReport::new(vec![outcome("a", OutcomeStatus::Warn, "x", true)]);
        let after = DoctorReport::new(vec![outcome("a", OutcomeStatus::Warn, "x", false)]);
        let fix_report = DoctorFixReport {
            before,
            after: after.clone(),
            attempted: 1,
            applied: 0,
            skipped: 0,
            failures: vec!["a: rebuild failed\nnoise".to_string()],
        };
        let json = serde_json::to_vec(&fix_report).unwrap();

        let (actual, lines, details) = parse_doctor_output(&json, DoctorMode::Fix).unwrap();

        assert_eq!(actual, after);
        assert_eq!(
            lines,
            vec![
                "[ERR] a: rebuild failed".to_string(),
                "[WARN] a: x".to_string(),
            ],
            "fix failures must stay visible alongside the after block"
        );
        assert_eq!(
            details,
            vec!["a: rebuild failed\nnoise".to_string(), "x".to_string()],
            "failure details must keep the full text"
        );
    }

    #[test]
    fn background_doctor_uses_quick_scope() {
        assert_eq!(
            QUICK_DOCTOR_CHECK_ARGS,
            ["check", "--quick", "--json"],
            "the dashboard poller must not run full DevBuild checks in the background"
        );
        assert_eq!(DoctorMode::Check.full_command_args(), ["check", "--json"]);
        assert_eq!(DoctorMode::Fix.full_command_args(), ["fix", "--json"]);
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
    fn typed_report_counts_and_invalid_output_are_handled() {
        let expected = make_report(9, 2, 1, 0);
        let json = serde_json::to_vec(&expected).unwrap();
        let (actual, _, _) = parse_doctor_output(&json, DoctorMode::Check).unwrap();
        assert_eq!(
            (
                actual.count_ok(),
                actual.count_warn(),
                actual.count_error(),
                actual.count_crash(),
            ),
            (9, 2, 1, 0)
        );
        assert_eq!(actual.divergence_count(), 3);
        assert!(parse_doctor_output(b"not json", DoctorMode::Check).is_err());
    }
}
