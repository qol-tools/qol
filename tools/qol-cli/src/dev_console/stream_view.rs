use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::dev_server::EndpointStatus;

use super::filters::{line_matches_filters, LogFilter};
use super::log_pane::{clamp_offset, dev_log_dir, window_start, LogRing};
use super::render_util::{
    accent, list_status, styled_line, view_content, NavigationOverflow, SignBox,
};
use super::{copy_highlight, spawn_forwarders, Dash, TraceRenderer, View};

pub(super) enum EndpointsState {
    Probing,
    Done(Vec<EndpointStatus>),
}

pub(super) fn start_trace(dash: &mut Dash) {
    if dash.trace.is_live() {
        return;
    }
    let renderer = dash.trace_renderer();
    match spawn_trace(renderer, dash.trace_details_enabled()) {
        Some((child, rx)) => {
            dash.trace.attach(child, rx);
            dash.trace_unavailable = false;
        }
        None => {
            if !dash.trace_unavailable {
                dash.trace.push(format!(
                    "[qol dev] could not start {} tracer ({})",
                    renderer.name(),
                    renderer.missing_hint()
                ));
            }
            dash.trace_unavailable = true;
        }
    }
}

pub(super) fn open_trace(dash: &mut Dash) {
    dash.view = View::Trace;
    dash.scroll_offset = 0;
    start_trace(dash);
}

fn trace_args(details: bool) -> Vec<&'static str> {
    let mut args = vec!["--no-header"];
    if details {
        args.push("--details");
    } else {
        args.push("--no-ghosts");
        args.push("--no-opacity");
    }
    args
}

fn spawn_trace(renderer: TraceRenderer, details: bool) -> Option<(Child, Receiver<String>)> {
    let root = crate::workspace::repo_root().ok()?;
    let mut cmd = trace_command(renderer, &root)?;
    cmd.args(trace_args(details));
    let mut child = cmd
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn trace_command(renderer: TraceRenderer, root: &Path) -> Option<Command> {
    let _ = root;
    Some(match renderer {
        TraceRenderer::Rust => {
            let mut cmd = Command::new(std::env::current_exe().ok()?);
            cmd.arg("trace-rs");
            cmd
        }
    })
}

pub(super) fn stop_trace(dash: &mut Dash) {
    dash.trace.stop();
}

pub(super) fn toggle_trace_details(dash: &mut Dash) {
    set_trace_details(dash, !dash.trace_details_enabled());
}

pub(super) fn toggle_trace_rate(dash: &mut Dash) {
    dash.trace_rate = dash.trace_rate.toggled();
    dash.mark_state_dirty();
    dash.notice = Some((Instant::now(), format!("trace {}", dash.trace_rate.label())));
}

pub(super) fn set_trace_details(dash: &mut Dash, enabled: bool) {
    if dash.trace_details_enabled() == enabled {
        return;
    }
    dash.trace_details = enabled;
    dash.mark_state_dirty();
    if !dash.trace.is_live() {
        return;
    }
    stop_trace(dash);
    start_trace(dash);
}

pub(super) const DEFAULT_TRACE_LOG_FILE: &str = qol_conventions::TRACE_LOG_PATH;

pub(super) struct LogSourceInfo {
    pub(super) kind: &'static str,
    file: Option<PathBuf>,
    folder: PathBuf,
    pub(super) stream_note: &'static str,
}

pub(super) fn current_log_source(dash: &Dash) -> Option<LogSourceInfo> {
    match dash.view {
        View::Trace => {
            let path = trace_log_file();
            Some(LogSourceInfo {
                kind: "trace",
                folder: path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(std::env::temp_dir),
                file: Some(path),
                stream_note: "raw probe file",
            })
        }
        View::Logs => {
            let file = dash.log_file.as_ref().map(|log_file| log_file.path.clone());
            let folder = file
                .as_ref()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(dev_log_dir);
            Some(LogSourceInfo {
                kind: "log",
                file,
                folder,
                stream_note: "qol dev stdout/stderr tee",
            })
        }
        _ => None,
    }
}

fn trace_log_file() -> PathBuf {
    std::env::var_os("QOL_TRACE_LOG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TRACE_LOG_FILE))
}

pub(super) fn open_current_log_folder(dash: &mut Dash) {
    let Some(source) = current_log_source(dash) else {
        return;
    };
    let message = match crate::host_facade::open_path(&source.folder) {
        Ok(outcome) if outcome.desktop_opened() => {
            format!("opened {} folder {}", source.kind, source.folder.display())
        }
        Ok(_) => {
            format!(
                "could not open {} folder {} · no desktop session",
                source.kind,
                source.folder.display()
            )
        }
        Err(error) => format!(
            "could not open {} folder {} · {error:#}",
            source.kind,
            source.folder.display()
        ),
    };
    dash.notice = Some((Instant::now(), message));
}

pub(super) fn open_current_log_editor(dash: &mut Dash, raw: bool) {
    let Some(source) = current_log_source(dash) else {
        return;
    };
    let Some(ref file) = source.file else {
        dash.notice = Some((
            Instant::now(),
            format!(
                "no persisted {} file yet in {}",
                source.kind,
                source.folder.display()
            ),
        ));
        return;
    };
    if !file.exists() {
        dash.notice = Some((Instant::now(), format!("{} does not exist", file.display())));
        return;
    }
    let target = match editor_target_for_source(dash, &source, file, raw) {
        Ok(target) => target,
        Err(error) => {
            dash.notice = Some((Instant::now(), error));
            return;
        }
    };
    let message = if open_with_os_default(&target) {
        format!("opened {}", target.display())
    } else {
        format!("could not open {}", target.display())
    };
    dash.notice = Some((Instant::now(), message));
}

fn editor_target_for_source(
    dash: &Dash,
    source: &LogSourceInfo,
    file: &Path,
    raw: bool,
) -> Result<PathBuf, String> {
    if source.kind != "trace" || raw {
        return Ok(file.to_path_buf());
    }
    render_pretty_trace_snapshot(file, dash.trace_renderer())
}

fn render_pretty_trace_snapshot(file: &Path, renderer: TraceRenderer) -> Result<PathBuf, String> {
    let root = crate::workspace::repo_root().map_err(|error| error.to_string())?;
    let pretty = pretty_trace_file(file);
    let mut cmd = trace_command(renderer, &root)
        .ok_or_else(|| format!("could not create {} trace renderer", renderer.name()))?;
    cmd.arg("--replay")
        .arg("--details")
        .env("QOL_TRACE_LOG_FILE", file);
    let output = cmd
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not render pretty trace: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(format!("pretty trace failed{suffix}"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    fs::write(&pretty, strip_ansi_codes(&text))
        .map_err(|error| format!("could not write {}: {error}", pretty.display()))?;
    Ok(pretty)
}

fn pretty_trace_file(file: &Path) -> PathBuf {
    let folder = file.parent().unwrap_or_else(|| Path::new("."));
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("trace");
    folder.join(format!("{stem}.pretty.log"))
}

fn strip_ansi_codes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn open_with_os_default(path: &Path) -> bool {
    crate::host_facade::open_text_file(path)
}

pub(super) fn draw_logs(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let highlight = copy_highlight(dash);
    let header = log_source_header(dash);
    let body_height = area.height.saturating_sub(header.len() as u16) as usize;
    let (rows, _, overflow) = stream_rows(
        &dash.logs.ring,
        &dash.filters.logs,
        &mut dash.scroll_offset,
        &mut dash.log_height,
        body_height,
        highlight,
        area.width as usize,
    );
    frame.render_widget(Paragraph::new(join_header_rows(header, rows)), area);
    overflow
}

pub(super) fn draw_trace(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let header = log_source_header(dash);
    let body_height = area.height.saturating_sub(header.len() as u16) as usize;
    let highlight = copy_highlight(dash);
    let (rows, overflow) = if dash.trace.ring.lines.is_empty() && dash.active_filters().is_empty() {
        dash.log_height = body_height;
        (
            vec![Line::from("  waiting for trace events")],
            NavigationOverflow::default(),
        )
    } else {
        let (rows, _, overflow) = stream_rows(
            &dash.trace.ring,
            &dash.filters.trace,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            body_height,
            highlight,
            area.width as usize,
        );
        (rows, overflow)
    };
    frame.render_widget(Paragraph::new(join_header_rows(header, rows)), area);
    overflow
}

fn join_header_rows<'a>(header: Vec<Line<'static>>, body: Vec<Line<'a>>) -> Vec<Line<'a>> {
    header.into_iter().chain(body).collect()
}

fn log_source_header(dash: &Dash) -> Vec<Line<'static>> {
    let Some(source) = current_log_source(dash) else {
        return Vec::new();
    };
    let file = source
        .file
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "(none yet)".to_string());
    let mut meta = vec![
        "  folder ".fg(Color::DarkGray),
        source.folder.display().to_string().fg(Color::White),
    ];
    meta.push(" · ".fg(Color::DarkGray));
    meta.push(source.stream_note.fg(Color::DarkGray));

    vec![
        Line::from(vec![
            format!("  {} file ", source.kind).fg(Color::DarkGray),
            file.fg(Color::White),
        ]),
        Line::from(meta),
        Line::from(""),
    ]
}

#[allow(clippy::too_many_arguments)]
fn stream_rows<'a>(
    ring: &'a LogRing,
    filters: &[LogFilter],
    scroll_offset: &mut usize,
    log_height: &mut usize,
    height: usize,
    highlight_tail: Option<usize>,
    inner_width: usize,
) -> (Vec<Line<'a>>, usize, NavigationOverflow) {
    *log_height = height;
    let filtered: Vec<&'a String> = ring
        .lines
        .iter()
        .filter(|line| line_matches_filters(line, filters))
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
    (
        rows,
        total,
        NavigationOverflow::from_window(start, height, total),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_run_log(
    frame: &mut Frame,
    area: Rect,
    ring: &LogRing,
    filters: &[LogFilter],
    scroll_offset: &mut usize,
    log_height: &mut usize,
    accent: Color,
    highlight_tail: Option<usize>,
) -> NavigationOverflow {
    let height = SignBox::capacity(area.height);
    let inner_width = area.width.saturating_sub(2) as usize;
    let (rows, total, overflow) = stream_rows(
        ring,
        filters,
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
    overflow
}

fn highlight_bar(line: Line<'_>, inner_width: usize) -> Line<'_> {
    let pad = inner_width.saturating_sub(line.width());
    let mut spans = line.spans;
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    Line::from(spans).style(Style::new().bg(Color::Rgb(38, 44, 74)))
}

pub(super) fn trace_value(dash: &Dash) -> Vec<Span<'static>> {
    if dash.trace.is_live() {
        return vec![format!("{} lines", dash.trace.len()).fg(Color::DarkGray)];
    }
    if dash.trace_unavailable {
        return vec!["tracer unavailable".fg(Color::DarkGray)];
    }
    vec!["idle · → open".fg(Color::DarkGray)]
}

pub(super) fn draw_endpoints(frame: &mut Frame, dash: &Dash, area: Rect) -> NavigationOverflow {
    let lines: Vec<Line> = match &dash.endpoints {
        EndpointsState::Probing => vec![Line::from("  probing endpoints".fg(Color::DarkGray))],
        EndpointsState::Done(items) => items.iter().map(endpoint_line).collect(),
    };
    view_content(frame, area, lines);
    NavigationOverflow::default()
}

fn endpoint_line(status: &EndpointStatus) -> Line<'static> {
    let (symbol, color) = if status.ok {
        ("✓", accent())
    } else {
        ("✗", Color::Red)
    };
    Line::from(vec![
        format!("  {symbol} ").fg(color).bold(),
        format!("{:<8}", status.label).fg(Color::White),
        format!("  {}", status.url).fg(Color::DarkGray),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_args_relax_by_default_and_expand_when_detailed() {
        assert_eq!(
            trace_args(false),
            ["--no-header", "--no-ghosts", "--no-opacity"],
            "the console trace defaults to a relaxed, suppressed firehose"
        );
        assert_eq!(
            trace_args(true),
            ["--no-header", "--details"],
            "toggling detail opts into the full expanded trace"
        );
    }

    #[test]
    fn trace_pretty_snapshot_helpers_keep_utf8_and_strip_ansi() {
        assert_eq!(
            pretty_trace_file(Path::new(DEFAULT_TRACE_LOG_FILE)),
            PathBuf::from("/tmp/qol-altmon.pretty.log")
        );
        assert_eq!(
            strip_ansi_codes("\x1b[2m[08:00]\x1b[0m ┌── \x1b[1;32mok\x1b[0m\n"),
            "[08:00] ┌── ok\n"
        );
    }
}
