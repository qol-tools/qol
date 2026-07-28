use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

use qol_conventions::doctor_wire::{
    FixReport as DoctorFixReport, Outcome, OutcomeStatus, Report as DoctorReport,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::poller::Poller;

use super::render_util::{
    accent, list_window_head, now_unix_ms, relative_age, view_content, NavigationOverflow,
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

impl ManualDoctor {
    fn progress_step(&self) -> String {
        self.progress
            .lock()
            .map(|step| step.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct DoctorRun {
    report: DoctorReport,
    lines: Vec<String>,
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
    dash.pokes.doctor = true;
}

pub(super) fn doctor_status(panel: &DoctorPanel, now_ms: u64) -> (Color, Vec<Span<'static>>) {
    if let Some(manual) = &panel.manual {
        let mut value = vec![manual.mode.gerund().fg(Color::Yellow)];
        let step = manual.progress_step();
        if !step.is_empty() {
            value.push(format!(" · {step}").fg(Color::DarkGray));
        }
        value.push(
            format!(" · {}", elapsed_label(now_ms, manual.started_at_ms)).fg(Color::DarkGray),
        );
        return (Color::Yellow, value);
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

pub(super) fn draw_doctor(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let lines = doctor_view_lines(&dash.doctor);
    if lines.is_empty() {
        let message = match &dash.doctor.manual {
            Some(manual) => {
                let step = manual.progress_step();
                if step.is_empty() {
                    manual.mode.progress_message().to_string()
                } else {
                    format!("{} · {step}", manual.mode.progress_message())
                }
            }
            None => "  no checks reported · press d to run".to_string(),
        };
        view_content(frame, area, vec![Line::from(message)]);
        return NavigationOverflow::default();
    }
    let total = lines.len();
    let (start, height) = list_window_head(dash, area, total);
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

fn elapsed_label(now_ms: u64, started_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(started_ms) / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    format!("{}m{:02}s", seconds / 60, seconds % 60)
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

    pub(super) fn progress_message(self) -> &'static str {
        match self {
            DoctorMode::Check => "  running checks",
            DoctorMode::Fix => "  applying fixes",
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
    set_progress(progress, "building doctor binary");
    build_doctor(&root)?;
    run_doctor_streaming(
        &crate::workspace::doctor_binary_path(&root),
        &root,
        mode,
        mode.full_command_args(),
        progress,
    )
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
    let binary = crate::workspace::doctor_binary_path(&root);
    if !binary.exists() {
        return Err("doctor binary not built · press d".to_string());
    }
    run_doctor_binary(
        &binary,
        &root,
        DoctorMode::Check,
        DoctorScope::Quick,
        QUICK_DOCTOR_CHECK_ARGS,
    )
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
    let (report, lines) = parse_doctor_output(stdout, mode).map_err(|error| {
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
        scope,
    })
}

fn build_doctor(root: &std::path::Path) -> Result<(), String> {
    let output = crate::workspace::cargo_build_command(root, &crate::workspace::DOCTOR_BUILD_ARGS)
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

fn parse_doctor_output(
    bytes: &[u8],
    mode: DoctorMode,
) -> Result<(DoctorReport, Vec<String>), serde_json::Error> {
    match mode {
        DoctorMode::Check => {
            let report: DoctorReport = serde_json::from_slice(bytes)?;
            let lines = report_lines(&report);
            Ok((report, lines))
        }
        DoctorMode::Fix => {
            let fix_report: DoctorFixReport = serde_json::from_slice(bytes)?;
            let mut lines = fix_report
                .failures
                .iter()
                .map(|failure| format!("[ERR] {}", first_line(failure)))
                .collect::<Vec<_>>();
            lines.extend(report_lines(&fix_report.after));
            Ok((fix_report.after, lines))
        }
    }
}

fn report_lines(report: &DoctorReport) -> Vec<String> {
    let mut outcomes: Vec<&Outcome> = report.outcomes.iter().collect();
    outcomes.sort_by_key(|outcome| status_rank(outcome.status));
    outcomes.into_iter().map(outcome_line).collect()
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

    fn span_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
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
            scope: DoctorScope::Full,
        };
        let quick_run = DoctorRun {
            report,
            lines: Vec::new(),
            scope: DoctorScope::Quick,
        };
        let warn_report = make_report(9, 2, 0, 0);
        let warn_run = DoctorRun {
            report: warn_report.clone(),
            lines: Vec::new(),
            scope: DoctorScope::Full,
        };
        let quick_warn_run = DoctorRun {
            report: warn_report,
            lines: Vec::new(),
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
    fn report_lines_sort_divergences_first() {
        let report = DoctorReport::new(vec![
            outcome("a", OutcomeStatus::Ok, "x", false),
            outcome("b", OutcomeStatus::Warn, "y", false),
            outcome("c", OutcomeStatus::Crash, "z", false),
            outcome("d", OutcomeStatus::Error, "w", false),
        ]);
        assert_eq!(
            report_lines(&report),
            vec!["[CRASH] c: z", "[ERR] d: w", "[WARN] b: y", "[OK] a: x"],
            "divergences must never hide below the ok lines"
        );
    }

    #[test]
    fn doctor_status_shows_manual_step_and_elapsed() {
        let (_tx, rx) = channel();
        let panel = DoctorPanel {
            last: None,
            last_at_ms: None,
            manual: Some(ManualDoctor {
                mode: DoctorMode::Fix,
                rx,
                progress: Arc::new(Mutex::new("check rust_clippy".to_string())),
                started_at_ms: 1_000_000_000 - 83_000,
            }),
            error: None,
        };
        let (color, spans) = doctor_status(&panel, 1_000_000_000);
        assert_eq!(color, Color::Yellow);
        assert_eq!(span_text(&spans), "fixing · check rust_clippy · 1m23s");
    }

    #[test]
    fn elapsed_label_formats_seconds_and_minutes() {
        let cases = [
            (5_000, "5s"),
            (59_999, "59s"),
            (60_000, "1m00s"),
            (683_000, "11m23s"),
        ];
        for (delta_ms, expected) in cases {
            assert_eq!(
                elapsed_label(1_000_000_000, 1_000_000_000 - delta_ms),
                expected
            );
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
    fn typed_check_output_preserves_outcomes() {
        let expected = DoctorReport::new(vec![
            outcome("a", OutcomeStatus::Warn, "x", true),
            outcome("b", OutcomeStatus::Ok, "y\ndetail", false),
        ]);
        let json = serde_json::to_vec(&expected).unwrap();

        let (actual, lines) = parse_doctor_output(&json, DoctorMode::Check).unwrap();

        assert_eq!(actual, expected);
        assert_eq!(
            lines,
            vec![
                "[WARN] a: x (fix available)".to_string(),
                "[OK] b: y".to_string(),
            ]
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

        let (actual, lines) = parse_doctor_output(&json, DoctorMode::Fix).unwrap();

        assert_eq!(actual, after);
        assert_eq!(
            lines,
            vec![
                "[ERR] a: rebuild failed".to_string(),
                "[WARN] a: x".to_string(),
            ],
            "fix failures must stay visible alongside the after block"
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
        let (actual, _) = parse_doctor_output(&json, DoctorMode::Check).unwrap();
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
