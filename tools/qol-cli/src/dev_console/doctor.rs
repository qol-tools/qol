use std::process::Command;
use std::sync::mpsc::{channel, Receiver};

use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::poller::Poller;

use super::render_util::{list_window, now_unix_ms, relative_age, view_content};
use super::{Dash, View, DOCTOR_BASE_INTERVAL, DOCTOR_CAP_INTERVAL};

pub(super) fn spawn_doctor_probe() -> Poller<Result<DoctorRun, String>> {
    Poller::spawn_adaptive(
        DOCTOR_BASE_INTERVAL,
        DOCTOR_CAP_INTERVAL,
        run_doctor_prebuilt,
    )
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct DoctorReport {
    pub(super) ok: usize,
    pub(super) warn: usize,
    pub(super) error: usize,
    pub(super) crash: usize,
}

impl DoctorReport {
    pub(super) fn divergences(&self) -> usize {
        self.warn + self.error + self.crash
    }
}

pub(super) struct DoctorPanel {
    pub(super) last: Option<DoctorRun>,
    pub(super) last_at_ms: Option<u64>,
    pub(super) manual: Option<(DoctorMode, Receiver<Result<DoctorRun, String>>)>,
    pub(super) error: Option<String>,
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
        let label = match run.scope {
            DoctorScope::Full => "all good",
            DoctorScope::Startup => "startup good",
        };
        (
            Color::Green,
            vec![
                label.fg(Color::Green).bold(),
                match run.scope {
                    DoctorScope::Full => format!(" · {} checks", report.ok),
                    DoctorScope::Startup => format!(" · {} startup checks", report.ok),
                }
                .fg(Color::DarkGray),
            ],
        )
    } else {
        let color = if report.error + report.crash > 0 {
            Color::Red
        } else {
            Color::Yellow
        };
        let label = match run.scope {
            DoctorScope::Full => format!("{} divergences", report.divergences()),
            DoctorScope::Startup => format!("startup {} divergences", report.divergences()),
        };
        (
            color,
            vec![
                label.fg(color).bold(),
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

pub(super) fn draw_doctor(frame: &mut Frame, dash: &mut Dash, area: Rect) {
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

#[derive(Clone, Copy)]
pub(super) enum DoctorMode {
    Check,
    Fix,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DoctorScope {
    Full,
    Startup,
}

const STARTUP_DOCTOR_CHECK_ARGS: &[&str] = &[
    qol_conventions::doctor_cli::ARG_CHECK,
    qol_conventions::doctor_cli::ARG_STARTUP,
];

impl DoctorMode {
    fn full_command_args(self) -> &'static [&'static str] {
        match self {
            DoctorMode::Check => &[qol_conventions::doctor_cli::ARG_CHECK],
            DoctorMode::Fix => &[qol_conventions::doctor_cli::ARG_FIX],
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

pub(super) fn spawn_doctor(mode: DoctorMode) -> Receiver<Result<DoctorRun, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_doctor(mode));
    });
    rx
}

fn run_doctor(mode: DoctorMode) -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    build_doctor(&root)?;
    run_doctor_binary(
        &crate::workspace::doctor_binary_path(&root),
        &root,
        mode,
        DoctorScope::Full,
        mode.full_command_args(),
    )
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
        DoctorScope::Startup,
        STARTUP_DOCTOR_CHECK_ARGS,
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
    let text = String::from_utf8_lossy(&output.stdout);
    let report =
        parse_doctor_summary(&text).ok_or_else(|| "could not read doctor summary".to_string())?;
    Ok(DoctorRun {
        report,
        lines: doctor_lines(&text, mode),
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

fn doctor_lines(text: &str, mode: DoctorMode) -> Vec<String> {
    match mode {
        DoctorMode::Check => bracket_lines(text),
        DoctorMode::Fix => fix_doctor_lines(text),
    }
}

fn fix_doctor_lines(text: &str) -> Vec<String> {
    let Some((before_after, after)) = text.rsplit_once("Doctor Check (After)") else {
        return bracket_lines(text);
    };

    let mut lines = before_after
        .lines()
        .filter(|line| line.trim_start().starts_with("[ERR]"))
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    lines.extend(bracket_lines(after));
    lines
}

fn bracket_lines(text: &str) -> Vec<String> {
    text.lines()
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

    fn span_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
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
            scope: DoctorScope::Full,
        };
        let startup_run = DoctorRun {
            report,
            lines: Vec::new(),
            scope: DoctorScope::Startup,
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
            scope: DoctorScope::Full,
        };
        let startup_warn_run = DoctorRun {
            report: warn_report,
            lines: Vec::new(),
            scope: DoctorScope::Startup,
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
                    last: Some(startup_run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: None,
                },
                Color::Green,
                "startup good · 11 startup checks · 15s ago",
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
                    last: Some(startup_warn_run),
                    last_at_ms: Some(now - 5_000),
                    manual: None,
                    error: None,
                },
                Color::Yellow,
                "startup 2 divergences · 2 warn · 0 err · just now",
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
                    last: Some(startup_run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: Some("boom".to_string()),
                },
                Color::Green,
                "startup good · 11 startup checks · 15s ago · probe failed",
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
    fn doctor_lines_fix_mode_keeps_fix_failures() {
        let text = "Doctor Check (Before)\n[WARN] a: x\nSummary: ok=0\n\nFixes attempted=1, applied=0, skipped=0, failures=1\n[ERR] a: rebuild failed\n\nDoctor Check (After)\n[WARN] a: x\nSummary: ok=0\n";
        assert_eq!(
            doctor_lines(text, DoctorMode::Fix),
            vec!["[ERR] a: rebuild failed", "[WARN] a: x"],
            "fix failures must stay visible alongside the after block"
        );
    }

    #[test]
    fn background_doctor_uses_startup_scope() {
        assert_eq!(
            STARTUP_DOCTOR_CHECK_ARGS,
            ["check", "--startup"],
            "the dashboard poller must not run full DevBuild checks in the background"
        );
        assert_eq!(DoctorMode::Check.full_command_args(), ["check"]);
        assert_eq!(DoctorMode::Fix.full_command_args(), ["fix"]);
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
}
