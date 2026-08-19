use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph};
use ratatui::Frame;

use crate::dev_server::{PluginDaemonStatus, WorkspacePlugin};

use super::activity::draw_activity;
use super::dash::{Dash, Health, LinksState, RebuildState, Row, View};
use super::disk::{disk_status, draw_disk};
use super::doctor::{doctor_status, draw_doctor};
use super::emu_panel::{draw_emu, draw_emu_detail, emu_detail_shows_warnings, emu_status};
use super::feature_flags::draw_feature_flags_panel;
use super::filters::{draw_filter_panel, filter_scope, FilterState};
use super::key_bindings::{context_action_bindings, global_action_bindings, unique_hints, KeyHint};
use super::render_util;
use super::render_util::{
    accent, caret, cursor_window_start, format_duration, list_capacity, now_unix_ms, view_content,
    NavigationOverflow, Sign, SignBox,
};
use super::stream_view::{draw_endpoints, draw_logs, draw_trace, trace_value};
use super::worktrees_panel::{draw_worktrees_panel, target_label};
use super::{ACK_TTL, BASE_ACCENT, ORANGE};

pub(super) fn draw(frame: &mut Frame, dash: &mut Dash) {
    let accent = frame_accent(dash);
    render_util::set_frame_accent(accent);
    render_util::reset_bottom_stack();
    let [_, body, _] = Layout::vertical([
        Constraint::Length(TITLE_CAP),
        Constraint::Min(0),
        Constraint::Length(TITLE_CAP),
    ])
    .areas(frame.area());
    let block = Block::bordered()
        .border_style(Style::new().fg(accent))
        .padding(PANEL_PADDING);
    let inner = block.inner(body);
    frame.render_widget(block, body);
    let content = page_header(frame, dash.view, inner);
    let navigation = draw_view_with_navigation(frame, dash, content);
    draw_filter_panel(frame, dash, inner, accent);
    draw_feature_flags_panel(frame, dash, inner, accent);
    draw_worktrees_panel(frame, dash, inner, accent);
    draw_quit_prompt(frame, dash, inner, accent);
    Sign {
        content: breadcrumb(dash, accent),
    }
    .render(frame, body, accent);
    draw_branch_sign(frame, dash, body, navigation);
    draw_activity(frame, dash, body, accent);
    draw_keys_hud(frame, dash, inner);
}

fn draw_view_with_navigation(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let overflow = draw_view(frame, dash, area);
    if !navigation_cue_unobstructed(dash) {
        return NavigationOverflow::default();
    }
    overflow
}

fn draw_view(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    match dash.view {
        View::Dashboard => draw_dashboard(frame, dash, area),
        View::Logs => draw_logs(frame, dash, area),
        View::Doctor => draw_doctor(frame, dash, area),
        View::Disk => draw_disk(frame, dash, area),
        View::Plugins => draw_plugins(frame, dash, area),
        View::Emu => draw_emu(frame, dash, area),
        View::EmuDetail => draw_emu_detail(frame, dash, area),
        View::Trace => draw_trace(frame, dash, area),
        View::Endpoints => draw_endpoints(frame, dash, area),
    }
}

fn navigation_cue_unobstructed(dash: &Dash) -> bool {
    !dash.worktree_panel.is_active()
        && !dash.feature_panel.is_active()
        && !dash.filter_state.is_active()
        && !dash.copying
        && !dash.quit_prompt_active()
}

pub(super) fn page_header(frame: &mut Frame, view: View, inner: Rect) -> Rect {
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

pub(super) fn page_description(view: View) -> Option<&'static str> {
    match view {
        View::Logs => Some("live daemon logs"),
        View::Trace => Some("runtime trace events"),
        View::Doctor => Some("install health checks"),
        View::Disk => Some("dev disk usage · enter to rescan"),
        View::Plugins => Some("workspace plugins · enter to link/unlink"),
        View::Emu => Some("isolated guest development · enter runs qol dev"),
        View::Endpoints => Some("local service endpoints"),
        View::Dashboard | View::EmuDetail => None,
    }
}

pub(super) fn filterable_view(view: View) -> bool {
    filter_scope(view).is_some()
}

pub(super) fn filters_visible(dash: &Dash) -> bool {
    filterable_view(dash.view)
        && !emu_detail_shows_warnings(dash)
        && !dash.active_filters().is_empty()
}

pub(super) fn breadcrumb(dash: &Dash, accent: Color) -> Line<'static> {
    let trail: Vec<String> = match dash.view {
        View::Dashboard => Vec::new(),
        View::Logs => vec!["logs".to_string()],
        View::Trace => vec!["trace".to_string()],
        View::Doctor => vec!["doctor".to_string()],
        View::Disk => vec!["disk".to_string()],
        View::Plugins => vec!["plugins".to_string()],
        View::Emu => vec!["sandboxes".to_string()],
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
    if filters_visible(dash) {
        spans.push(" · FILTERED".fg(Color::Yellow).bold());
    }
    if dash.quit_prompt_active() {
        spans.push(" · QUIT?".fg(Color::Red).bold());
    } else if dash.is_reloading() {
        let tone = if dash.worktree_diverged() {
            ORANGE
        } else {
            Color::Red
        };
        spans.push(" · RELOADING".fg(tone).bold());
    } else if dash.worktree_diverged() {
        spans.push(
            format!(" · WORKTREE {}", dash.pinned_label())
                .fg(ORANGE)
                .bold(),
        );
    } else if dash.armed {
        spans.push(" · ARMED".fg(Color::Yellow).bold());
    }
    Line::from(spans)
}

pub(super) const KEYS_HUD_WIDTH: u16 = 34;

pub(super) fn accent_state_line(dash: &Dash) -> String {
    format!(
        "[state] accent={:?} armed={} reloading={} quit={} view={:?} selection={:?} running={:?}",
        frame_accent(dash),
        dash.armed,
        dash.is_reloading(),
        dash.quit_prompt_active(),
        dash.view,
        dash.worktree_selection,
        dash.running_branch
    )
}

pub(super) fn resolve_base_label() -> String {
    crate::workspace::repo_root()
        .ok()
        .and_then(|root| qol_dev_build::tray::resolve_git_branch(&root))
        .unwrap_or_else(|| "base".to_string())
}

pub(super) fn branch_sign_line(dash: &Dash) -> Line<'static> {
    let running = target_label(dash.running_branch.as_deref(), &dash.base_label);
    if !dash.worktree_diverged() {
        return Line::from(running.fg(accent()).bold());
    }
    Line::from(vec![
        running.fg(accent()),
        " → ".fg(ORANGE).bold(),
        dash.pinned_label().fg(ORANGE).bold(),
    ])
}

pub(super) fn draw_branch_sign(
    frame: &mut Frame,
    dash: &Dash,
    body: Rect,
    navigation: NavigationOverflow,
) {
    Sign {
        content: branch_sign_line(dash),
    }
    .render_bottom(frame, body, sign_accent(dash), navigation);
}

pub(super) fn quit_prompt_rows() -> Vec<Line<'static>> {
    vec![Line::from(vec![
        " press ".fg(Color::White),
        "ctrl+q".fg(Color::Red).bold(),
        " again to quit".fg(Color::White),
    ])]
}

pub(super) fn draw_quit_prompt(frame: &mut Frame, dash: &Dash, area: Rect, accent: Color) {
    if !dash.quit_prompt_active() {
        return;
    }
    render_util::render_bottom_panel(frame, area, "quit", quit_prompt_rows(), accent);
}

pub(super) fn context_keys(dash: &Dash) -> Vec<KeyHint> {
    if dash.copying {
        return vec![
            KeyHint {
                key: "digits",
                desc: "line count",
            },
            KeyHint {
                key: "enter",
                desc: "copy",
            },
            KeyHint {
                key: "esc",
                desc: "cancel",
            },
        ];
    }
    if dash.feature_panel.is_active() {
        return vec![
            KeyHint {
                key: "←↑↓→",
                desc: "select flag",
            },
            KeyHint {
                key: "space",
                desc: "toggle",
            },
            KeyHint {
                key: "enter",
                desc: "toggle",
            },
            KeyHint {
                key: "esc",
                desc: "close",
            },
        ];
    }
    if dash.worktree_panel.is_active() {
        return vec![
            KeyHint {
                key: "←↑↓→",
                desc: "select worktree",
            },
            KeyHint {
                key: "enter",
                desc: "arm target",
            },
            KeyHint {
                key: "esc",
                desc: "close",
            },
        ];
    }
    match &dash.filter_state {
        FilterState::Managing => {
            return vec![
                KeyHint {
                    key: "←↑↓→",
                    desc: "select filter",
                },
                KeyHint {
                    key: "enter",
                    desc: "add",
                },
                KeyHint {
                    key: "e",
                    desc: "edit",
                },
                KeyHint {
                    key: "d",
                    desc: "delete",
                },
                KeyHint {
                    key: "esc",
                    desc: "close",
                },
            ];
        }
        FilterState::Editing { .. } => {
            return vec![
                KeyHint {
                    key: "type",
                    desc: "filter text",
                },
                KeyHint {
                    key: "↑/↓",
                    desc: "strategy + / -",
                },
                KeyHint {
                    key: "enter",
                    desc: "save",
                },
                KeyHint {
                    key: "esc",
                    desc: "cancel",
                },
            ];
        }
        FilterState::Closed => {}
    }
    unique_hints(context_action_bindings(dash))
}

pub(super) fn global_keys(armed: bool) -> Vec<KeyHint> {
    unique_hints(global_action_bindings(armed))
}

pub(super) fn key_lines(keys: &[KeyHint]) -> Vec<Line<'static>> {
    keys.iter()
        .map(|hint| {
            Line::from(vec![
                format!(" {:<9} ", hint.key).fg(Color::White).bold(),
                format!("{} ", hint.desc).fg(Color::DarkGray),
            ])
        })
        .collect()
}

pub(super) fn section_label(label: &'static str) -> Line<'static> {
    Line::from(format!(" {label}").fg(accent()).bold())
}

pub(super) fn keys_rows(dash: &Dash) -> Vec<Line<'static>> {
    let mut rows = vec![section_label("global")];
    rows.push(Line::from(""));
    rows.extend(key_lines(&global_keys(dash.armed)));
    rows.push(Line::from(""));
    rows.push(Line::from(""));
    rows.push(section_label("context"));
    rows.push(Line::from(""));
    rows.extend(key_lines(&context_keys(dash)));
    rows
}

pub(super) fn draw_keys_hud(frame: &mut Frame, dash: &Dash, area: Rect) {
    if dash.keys_hidden || dash.is_reloading() {
        return;
    }
    let rows = keys_rows(dash);
    let height = (rows.len() as u16 + SignBox::CHROME_ROWS).min(area.height);
    if height == 0 {
        return;
    };
    let rect = Rect {
        x: area.x + area.width.saturating_sub(KEYS_HUD_WIDTH),
        y: area.y,
        width: KEYS_HUD_WIDTH.min(area.width),
        height,
    };
    frame.render_widget(Clear, rect);
    SignBox {
        title: "keys · ctrl+k",
        rows,
    }
    .render(frame, rect, frame_accent(dash));
}

pub(super) fn draw_dashboard(frame: &mut Frame, dash: &Dash, area: Rect) -> NavigationOverflow {
    let (tray_color, tray_value) = tray_status(dash);
    let (web_color, web_value) = web_status(dash.web);
    let (plugins_color, plugins_value) =
        plugins_status(&dash.plugin_reload, dash.plugin_names.len(), &dash.links);
    let (emu_color, emu_value) = emu_status(&dash.emu);
    let (doctor_color, doctor_value) = doctor_status(&dash.doctor, now_unix_ms());
    let (disk_color, disk_value) = disk_status(&dash.disk, now_unix_ms());

    let rows = vec![
        dash_row(dash.cursor == 0, tray_color, Row::Tray, tray_value),
        dash_row(dash.cursor == 1, web_color, Row::Web, web_value),
        dash_row(dash.cursor == 2, plugins_color, Row::Plugins, plugins_value),
        dash_row(dash.cursor == 3, emu_color, Row::Emu, emu_value),
        dash_row(dash.cursor == 4, doctor_color, Row::Doctor, doctor_value),
        dash_row(dash.cursor == 5, disk_color, Row::Disk, disk_value),
        dash_row(
            dash.cursor == 6,
            accent(),
            Row::Logs,
            vec![format!("{} buffered", dash.logs.len()).fg(Color::DarkGray)],
        ),
        dash_row(dash.cursor == 7, accent(), Row::Trace, trace_value(dash)),
    ];

    let total = rows.len();
    let height = list_capacity(area.height);
    let start = cursor_window_start(total, height, dash.cursor);
    let visible = rows.into_iter().skip(start).take(height).collect();
    view_content(frame, area, visible);
    NavigationOverflow::from_window(start, height, total)
}

pub(super) fn tray_status(dash: &Dash) -> (Color, Vec<Span<'static>>) {
    let (text, color) = match dash.health {
        Health::Checking => ("starting", Color::Yellow),
        Health::Up => ("running", accent()),
        Health::Down => ("down", Color::Red),
    };
    let mut value = vec![
        text.fg(color).bold(),
        format!(" · up {}", format_duration(dash.started.elapsed())).fg(Color::DarkGray),
    ];
    if dash.health == Health::Up {
        value.push(" · api ✓".fg(accent()));
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

pub(super) fn web_status(web: Health) -> (Color, Vec<Span<'static>>) {
    match web {
        Health::Checking => (Color::Yellow, vec!["checking".fg(Color::Yellow)]),
        Health::Up => (
            accent(),
            vec![
                "up".fg(accent()).bold(),
                format!(" · localhost:{}", qol_conventions::DEFAULT_PORT).fg(Color::DarkGray),
            ],
        ),
        Health::Down => (Color::Red, vec!["down".fg(Color::Red).bold()]),
    }
}

pub(super) fn dash_row(
    selected: bool,
    color: Color,
    row: Row,
    value: Vec<Span<'static>>,
) -> Line<'static> {
    let label = row.label();
    let caret = caret(selected);
    let label_span = if selected {
        format!(" {label:<DASH_LABEL_WIDTH$} ")
            .fg(Color::White)
            .bold()
    } else {
        format!(" {label:<DASH_LABEL_WIDTH$} ").fg(Color::DarkGray)
    };
    let mut spans: Vec<Span<'static>> = vec![caret, "●".fg(color).bold(), label_span];
    spans.extend(value);
    Line::from(spans)
}

const DASH_LABEL_WIDTH: usize = "sandboxes".len();

pub(super) fn frame_accent(dash: &Dash) -> Color {
    if dash.quit_prompt_active() {
        Color::Red
    } else if dash.is_reloading() {
        if dash.worktree_diverged() {
            ORANGE
        } else {
            Color::Red
        }
    } else if dash.armed || dash.is_busy() {
        Color::Yellow
    } else {
        BASE_ACCENT
    }
}

pub(super) fn sign_accent(dash: &Dash) -> Color {
    if dash.worktree_diverged() && !dash.quit_prompt_active() {
        ORANGE
    } else {
        frame_accent(dash)
    }
}

pub(super) const PANEL_PADDING: Padding = Padding {
    left: 1,
    right: 1,
    top: 2,
    bottom: 1,
};

pub(super) const TITLE_CAP: u16 = 1;

pub(super) fn plugins_status(
    state: &RebuildState,
    boot_count: usize,
    links: &LinksState,
) -> (Color, Vec<Span<'static>>) {
    let (live_color, mut value) = match links {
        LinksState::Live(plugins) => {
            let linked = plugins.iter().filter(|plugin| plugin.linked).count();
            let stale = plugins
                .iter()
                .filter(|plugin| plugin.linked && plugin.needs_rebuild)
                .count();
            if stale > 0 {
                (
                    Color::Yellow,
                    vec![
                        format!("{linked} linked").fg(accent()),
                        format!(" · {stale} stale").fg(Color::Yellow).bold(),
                    ],
                )
            } else {
                (accent(), vec![format!("{linked} linked").fg(accent())])
            }
        }
        LinksState::Unknown => (
            accent(),
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

pub(super) fn plugin_row_count(dash: &Dash) -> usize {
    match &dash.links {
        LinksState::Live(rows) => rows.len(),
        LinksState::Unknown | LinksState::Unreachable => 0,
    }
}

pub(super) fn draw_plugins(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let height = list_capacity(area.height);
    dash.log_height = height;
    let total = plugin_row_count(dash);
    if total == 0 {
        let message = match &dash.links {
            LinksState::Unreachable => "  api down",
            LinksState::Unknown => "  loading plugins…",
            LinksState::Live(_) => "  no workspace plugins found",
        };
        view_content(frame, area, vec![Line::from(message.fg(Color::DarkGray))]);
        return NavigationOverflow::default();
    }
    if dash.plugin_cursor >= total {
        dash.plugin_cursor = total - 1;
    }
    let cursor = dash.plugin_cursor;
    let start = cursor_window_start(total, height, cursor);
    let LinksState::Live(rows) = &dash.links else {
        return NavigationOverflow::default();
    };
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, row)| {
            let status = dash.plugin_health.as_deref().and_then(|health_rows| {
                health_rows
                    .iter()
                    .find(|health| health.plugin_id == row.id)
                    .map(|health| &health.status)
            });
            plugin_row_line(row, status, index == cursor)
        })
        .collect();
    view_content(frame, area, lines);
    NavigationOverflow::from_window(start, height, total)
}

pub(super) fn plugin_row_line(
    row: &WorkspacePlugin,
    daemon_status: Option<&PluginDaemonStatus>,
    selected: bool,
) -> Line<'static> {
    let caret = caret(selected);
    let (dot, status) = if !row.linked {
        (
            "○".fg(Color::DarkGray).bold(),
            " · linkable".fg(Color::DarkGray),
        )
    } else if row.needs_rebuild {
        ("●".fg(Color::Yellow).bold(), " · stale".fg(Color::Yellow))
    } else {
        ("●".fg(accent()).bold(), " · linked".fg(Color::DarkGray))
    };
    let name = format!(" {}", row.name);
    let name_span = if selected {
        name.fg(Color::White).bold()
    } else {
        name.fg(Color::White)
    };
    let mut spans = vec![caret, dot, name_span];
    if !row.version.is_empty() {
        spans.push(format!(" v{}", row.version).fg(Color::DarkGray));
    }
    spans.push(status);
    if row.linked && row.needs_rebuild && !row.rebuild_reason.is_empty() {
        spans.push(" · ".fg(Color::Yellow));
        spans.push(row.rebuild_reason.clone().fg(Color::DarkGray));
    }
    if let Some(daemon_span) = daemon_status_span(daemon_status) {
        spans.push(daemon_span);
    }
    Line::from(spans)
}

pub(super) fn daemon_status_span(status: Option<&PluginDaemonStatus>) -> Option<Span<'static>> {
    match status? {
        PluginDaemonStatus::NotExpected => None,
        PluginDaemonStatus::AutostartBlocked => Some(" · idle (on-demand)".fg(Color::DarkGray)),
        PluginDaemonStatus::OnDemand { pid: _ } => Some(" · running (on-demand)".fg(accent())),
        PluginDaemonStatus::Stable { pid: _ } => Some(" · running".fg(accent())),
        PluginDaemonStatus::Probation {
            pid: _,
            consecutive_failures: _,
        } => Some(" · starting".fg(Color::Yellow)),
        PluginDaemonStatus::Down {
            consecutive_failures: _,
            suppressed: true,
        } => Some(" · crash-looped".fg(Color::Red).bold()),
        PluginDaemonStatus::Down {
            consecutive_failures: _,
            suppressed: false,
        } => Some(" · dead".fg(Color::Red)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::dash::ROWS;
    use crate::dev_console::testkit::*;
    use crate::dev_console::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::mpsc::channel;
    use std::time::Instant;

    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    use crate::dev_console::emu_panel::EmuDetail;
    use crate::dev_console::filters::{filter_panel_rows, FilterStrategy};
    use crate::dev_console::key_bindings::{action_for, Action};
    use crate::dev_console::log_pane::DevLogFile;

    use crate::dev_console::stream_view::{current_log_source, DEFAULT_TRACE_LOG_FILE};
    use crate::dev_console::worktrees_panel::worktree_panel_rows;

    #[test]
    fn plugins_status_counts_only_linked_among_workspace_plugins() {
        let fresh = workspace_plugin("foo", true, false);
        let stale = workspace_plugin("bar", true, true);
        let linkable = workspace_plugin("baz", false, false);
        let cases = [
            (
                LinksState::Live(vec![fresh.clone(), stale.clone(), linkable.clone()]),
                Color::Yellow,
                "2 linked · 1 stale",
            ),
            (
                LinksState::Live(vec![fresh.clone(), linkable.clone()]),
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
    fn dashboard_values_share_one_column_for_every_row_label() {
        let starts = ROWS.map(|row| {
            let line = dash_row(false, Color::Green, row, vec!["VALUE".fg(Color::Green)]);
            span_text(&line.spans)
                .chars()
                .position(|character| character == 'V')
                .unwrap()
        });

        assert!(starts.iter().all(|start| *start == starts[0]));
        assert_eq!(
            ROWS.iter().map(|row| row.label().len()).max(),
            Some(DASH_LABEL_WIDTH)
        );
    }

    #[test]
    fn plugin_row_line_marks_linked_and_linkable_states() {
        let cases = [
            (workspace_plugin("a", true, false), "● a v1.0.0 · linked"),
            (workspace_plugin("b", false, false), "○ b · linkable"),
        ];
        for (row, expected) in cases {
            let text = span_text(&plugin_row_line(&row, None, false).spans);
            assert!(
                text.contains(expected),
                "row line must show {expected:?}, got {text:?}"
            );
        }
    }

    #[test]
    fn plugin_row_line_renders_daemon_status_dimension() {
        let row = workspace_plugin("a", true, false);
        let cases = [
            (None, ""),
            (Some(PluginDaemonStatus::NotExpected), ""),
            (
                Some(PluginDaemonStatus::AutostartBlocked),
                "idle (on-demand)",
            ),
            (
                Some(PluginDaemonStatus::OnDemand { pid: 1 }),
                "running (on-demand)",
            ),
            (Some(PluginDaemonStatus::Stable { pid: 1 }), "running"),
            (
                Some(PluginDaemonStatus::Probation {
                    pid: 1,
                    consecutive_failures: 0,
                }),
                "starting",
            ),
            (
                Some(PluginDaemonStatus::Down {
                    consecutive_failures: 1,
                    suppressed: false,
                }),
                "dead",
            ),
            (
                Some(PluginDaemonStatus::Down {
                    consecutive_failures: 5,
                    suppressed: true,
                }),
                "crash-looped",
            ),
        ];
        for (status, expected) in cases {
            let text = span_text(&plugin_row_line(&row, status.as_ref(), false).spans);
            if expected.is_empty() {
                assert!(
                    text.ends_with("· linked"),
                    "no daemon suffix for {status:?}, got {text:?}"
                );
            } else {
                assert!(
                    text.contains(expected),
                    "{status:?} renders {expected}, got: {text}"
                );
            }
        }
    }

    #[test]
    fn plugin_row_line_carets_the_selected_row() {
        let row = workspace_plugin("a", true, false);
        let selected = span_text(&plugin_row_line(&row, None, true).spans);
        let unselected = span_text(&plugin_row_line(&row, None, false).spans);
        assert!(
            selected.starts_with("▸"),
            "selected row gets a caret: {selected:?}"
        );
        assert!(
            !unselected.starts_with("▸"),
            "unselected row has no caret: {unselected:?}"
        );
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
    fn every_view_shows_its_page_as_a_breadcrumb_on_the_qol_dev_sign() {
        let cases = [
            (View::Endpoints, "qol dev · endpoints"),
            (View::Plugins, "qol dev · plugins"),
            (View::Doctor, "qol dev · doctor"),
            (View::Emu, "qol dev · sandboxes"),
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
    fn filterable_log_views_mark_saved_filters_in_the_breadcrumb() {
        for view in [View::Logs, View::Trace, View::EmuDetail] {
            let mut dash = Dash::new(Vec::new());
            dash.view = view;
            set_active_filters(
                &mut dash,
                vec![log_filter(FilterStrategy::Include, "shortcut")],
            );
            if view == View::EmuDetail {
                dash.emu_detail = Some(EmuDetail {
                    id: "foo".to_string(),
                    info: Vec::new(),
                    warnings: Vec::new(),
                    replay: None,
                });
            }
            let text = span_text(&breadcrumb(&dash, Color::Green).spans);
            assert!(
                text.contains("FILTERED"),
                "filterable view should mark active filters: {text}"
            );
        }
    }

    #[test]
    fn non_filtering_views_do_not_mark_filters_in_the_breadcrumb() {
        let mut logs = Dash::new(Vec::new());
        logs.view = View::Logs;
        assert!(!span_text(&breadcrumb(&logs, Color::Green).spans).contains("FILTERED"));

        let mut dashboard = Dash::new(Vec::new());
        dashboard.filters.logs = vec![log_filter(FilterStrategy::Include, "shortcut")];
        assert!(!span_text(&breadcrumb(&dashboard, Color::Green).spans).contains("FILTERED"));
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
            (View::Emu, "run qol dev · stop"),
        ];
        for (view, expected) in cases {
            let mut dash = Dash::new(Vec::new());
            dash.view = view;
            let text = render_text(&mut dash);
            assert!(text.contains(expected), "missing {expected:?}");
            assert!(text.contains("ctrl+r"), "missing globals");
            assert!(text.contains("rebuild tray+plugins"), "missing globals");
            assert!(text.contains("worktrees"), "missing worktree picker key");
            assert!(
                !text.contains("reload qol dev"),
                "reload shown while unarmed"
            );
            assert!(!text.contains("armed ctrl+r"), "stale armed label rendered");
            assert!(!text.contains("ctrl+u"), "stale reload shortcut rendered");
            assert!(text.contains("keys · ctrl+k"), "missing keys badge");
            assert!(text.contains("global"), "missing global section");
            assert!(text.contains("context"), "missing context section");
            assert!(
                text.find("global") < text.find("context"),
                "global section should render before context"
            );
            assert!(
                !text.contains("context · k"),
                "stale context title rendered"
            );
            if matches!(view, View::Emu) {
                assert!(text.contains("verify image"), "missing verified image key");
                assert!(
                    !text.contains("set arch"),
                    "stale architecture key rendered"
                );
            }
        }
    }

    #[test]
    fn keys_hud_swaps_ctrl_r_action_when_armed() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        let text = render_text(&mut dash);
        assert!(text.contains("ctrl+r"), "missing ctrl+r key");
        assert!(
            text.contains("reload qol dev"),
            "missing armed reload action"
        );
        assert!(
            !text.contains("rebuild tray+plugins"),
            "unarmed rebuild action rendered"
        );
        assert!(text.contains("keys · ctrl+k"), "missing keys badge");
        assert!(text.contains("global"), "missing global section");
        assert!(text.contains("context"), "missing context section");
        assert!(!text.contains("armed ctrl+r"), "stale armed label rendered");
        assert!(!text.contains("ctrl+u"), "stale reload shortcut rendered");
    }

    #[test]
    fn keys_rows_space_sections() {
        let dash = Dash::new(Vec::new());
        let rows: Vec<String> = keys_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        assert_eq!(rows[0], " global");
        assert_eq!(rows[1], "");
        assert_eq!(rows[2], " ctrl+r    rebuild tray+plugins ");
        assert_eq!(rows[3], " ctrl+k    keys ");
        assert_eq!(rows[4], " ctrl+w    worktrees ");
        assert_eq!(rows[5], " ctrl+f    feature flags ");
        assert_eq!(rows[6], " ctrl+q    quit (press twice) ");
        assert_eq!(rows[7], "");
        assert_eq!(rows[8], "");
        assert_eq!(rows[9], " context");
        assert_eq!(rows[10], "");
        assert_eq!(rows[11], " ↑/↓       move ");
    }

    #[test]
    fn trace_keys_include_detail_toggle_not_doctor() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;
        let rows: Vec<String> = keys_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        let text = rows.join("\n");
        assert!(
            text.contains(" d         details "),
            "missing trace detail key"
        );
        assert!(
            text.contains(" o         open folder "),
            "missing open folder key"
        );
        assert!(
            text.contains(" e         open in editor "),
            "missing editor open key"
        );
        assert!(
            text.contains(" r         open raw "),
            "missing raw open key"
        );
        assert!(
            text.contains(" space     arm: reload "),
            "missing reload arm key"
        );
        assert!(
            !text.contains("arm: raw"),
            "trace context must not show the legacy raw arm key"
        );
        assert!(
            !text.contains("refresh checks"),
            "trace context must not show doctor binding"
        );
    }

    #[test]
    fn logs_and_trace_render_source_metadata() {
        let mut trace = Dash::new(Vec::new());
        trace.view = View::Trace;
        let trace_text = render_text(&mut trace);
        assert!(
            trace_text.contains(DEFAULT_TRACE_LOG_FILE),
            "trace pane should show trace file path"
        );
        assert!(
            !trace_text.contains("open -t"),
            "trace pane should not expose macOS opener fallback"
        );

        let mut logs = Dash::new(Vec::new());
        logs.view = View::Logs;
        logs.log_file = Some(DevLogFile::path_only(PathBuf::from(
            "/tmp/qol-dev-test.log",
        )));
        let source = current_log_source(&logs).expect("logs source");
        assert_eq!(source.stream_note, "qol dev stdout/stderr tee");
        let logs_text = render_text(&mut logs);
        assert!(
            logs_text.contains("qol-dev-test.log"),
            "logs pane should show the current session log file"
        );
        assert!(
            !logs_text.contains("open -t"),
            "logs pane should not expose macOS opener fallback"
        );
    }

    #[test]
    fn keys_box_width_stays_fixed_when_armed() {
        let mut unarmed = Dash::new(Vec::new());
        let unarmed_rows = render_rows(&mut unarmed);
        let mut armed = Dash::new(Vec::new());
        armed.armed = true;
        let armed_rows = render_rows(&mut armed);

        assert_eq!(
            row_bounds(&unarmed_rows, "rebuild tray+plugins"),
            row_bounds(&armed_rows, "reload qol dev")
        );
    }

    #[test]
    fn keys_hud_and_panel_follow_filter_state() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        dash.filter_layout_width = 24;
        set_active_filters(
            &mut dash,
            vec![
                log_filter(FilterStrategy::Include, "shortcut"),
                log_filter(FilterStrategy::Exclude, "success"),
                log_filter(FilterStrategy::Include, "trace"),
            ],
        );

        let text = render_text(&mut dash);
        assert!(text.contains("select filter"), "manager keys missing");
        assert!(text.contains("delete"), "delete key missing");
        dash.filter_layout_width = 24;
        let rows: Vec<String> = filter_panel_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        assert_eq!(rows[0], "[+ shortcut]  - success ");
        assert_eq!(rows[1], " + trace ");

        dash.start_filter_add();
        let text = render_text(&mut dash);
        assert!(
            text.contains("strategy + / -"),
            "editing strategy key missing"
        );
        assert!(text.contains("save"), "editing save key missing");
        let rows: Vec<String> = filter_panel_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        assert_eq!(rows, vec![" add + _"]);
    }

    #[test]
    fn worktree_panel_empty_scan_renders_no_worktrees() {
        let mut dash = Dash::new(Vec::new());
        dash.worktree_panel.open = true;
        dash.worktree_panel.layout_width = 24;
        dash.worktree_panel.targets = vec![worktrees_panel::WorktreeTarget {
            branch: None,
            id: "base".to_string(),
        }];

        let rows: Vec<String> = worktree_panel_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();

        assert!(rows.iter().any(|row| row.contains("base")));
        assert!(rows.iter().any(|row| row.contains("no worktrees")));
    }

    #[test]
    fn reloading_state_drives_red_accent_and_status() {
        let mut dash = Dash::new(Vec::new());
        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel();
        dash.reload = Reload::Running {
            child,
            rx,
            activity: ReloadProgress::new(),
        };
        assert!(dash.is_reloading());
        assert_eq!(frame_accent(&dash), Color::Red);
        if let Reload::Running { mut child, .. } = dash.reload {
            let _ = child.wait();
        }
    }

    #[test]
    fn reload_activity_sign_right_aligns_on_the_bottom_border() {
        let mut dash = Dash::new(Vec::new());
        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel();
        let mut activity = ReloadProgress::new();
        assert!(activity.observe(&format!(
            "{}build\tqol-tray dev",
            crate::commands::dev::DEV_RELOAD_PROGRESS_PREFIX
        )));
        dash.reload = Reload::Running {
            child,
            rx,
            activity,
        };

        let rows = render_rows_at(&mut dash, 110, 28);
        let border = &rows[rows.len() - 2];
        let activity_idx = border
            .find("┤ reload")
            .expect("reload sign title missing from the bottom border");
        let branch_idx = border.find("┤ base ├").expect("worktree sign rendered");
        assert!(
            border.contains("build · qol-tray dev"),
            "reload phase and detail must sit inside the sign"
        );
        assert!(
            activity_idx > branch_idx,
            "reload sign must right-align after the centered worktree sign: {border}"
        );
        assert!(
            rows.iter().all(|row| !row.contains("keys · ctrl+k")),
            "the keys HUD must not cover the reload sign"
        );
        if let Reload::Running { mut child, .. } = dash.reload {
            let _ = child.wait();
        }
    }

    #[test]
    fn branch_sign_shows_running_and_diverged_target() {
        let mut dash = Dash::new(Vec::new());
        dash.base_label = "main".to_string();
        let cases: [(Option<&str>, Option<&str>, &str); 3] = [
            (None, None, "main"),
            (Some("feat/x"), None, "main → feat/x"),
            (Some("feat/x"), Some("feat/x"), "feat/x"),
        ];
        for (target, running, expected) in cases {
            dash.worktree_selection = match target {
                Some(branch) => WorktreeSelection::Pin(Some(branch.to_string())),
                None => WorktreeSelection::Follow,
            };
            dash.running_branch = running.map(str::to_string);
            let text = span_text(&branch_sign_line(&dash).spans);
            assert_eq!(text, expected, "target: {target:?} running: {running:?}");
        }
    }

    #[test]
    fn branch_sign_straddles_bottom_border() {
        let mut dash = Dash::new(Vec::new());
        let rows = render_rows(&mut dash);
        let border = &rows[rows.len() - 2];
        assert!(
            border.contains("┤ base ├"),
            "bottom sign missing from the border row: {border}"
        );
        assert!(
            border.contains('└') && border.contains('┘'),
            "frame corners must share the border row: {border}"
        );
        let cap = rows.last().expect("render produced rows");
        assert!(
            cap.contains('╰') && cap.contains('╯'),
            "sign undercurve must not be cut off: {cap}"
        );
    }

    #[test]
    fn navigation_cues_only_render_for_clipped_content() {
        let mut dash = Dash::new(Vec::new());
        let rows = render_rows(&mut dash);
        assert!(
            rows.iter()
                .all(|row| !row.contains("│ ^ │") && !row.contains("│ v │")),
            "fully visible dashboard must not render navigation cues"
        );

        let rows = render_rows_at(&mut dash, 110, 14);
        let border = &rows[rows.len() - 2];
        let down = border
            .find("│ v │")
            .expect("clipped dashboard must render a down cue");
        let sign = border
            .find("┤ base ├")
            .expect("worktree sign missing from bottom border");
        assert!(
            rows[rows.len() - 3].contains("┌───┐") && rows[rows.len() - 1].contains("└───┘"),
            "down cue must be a square-cornered box containing v"
        );
        assert!(down < sign, "down cue must sit left of the worktree sign");
        for label in ["tray", "web", "plugins", "sandboxes"] {
            assert!(
                rows.iter().any(|row| row.contains(label)),
                "down cue pushed {label} out of the viewport"
            );
        }
        assert!(
            rows.iter().all(|row| !row.contains("│ ^ │")),
            "top cue rendered with no content above"
        );

        dash.cursor = ROWS.len() - 1;
        let rows = render_rows_at(&mut dash, 110, 14);
        let border = &rows[rows.len() - 2];
        let up = border
            .find("│ ^ │")
            .expect("scrolled dashboard must render an up cue");
        let sign = border
            .find("┤ base ├")
            .expect("worktree sign missing from bottom border");
        assert!(
            rows[rows.len() - 3].contains("┌───┐") && rows[rows.len() - 1].contains("└───┘"),
            "up cue must be a square-cornered box containing ^"
        );
        assert!(up > sign, "up cue must sit right of the worktree sign");
        for label in ["doctor", "disk", "logs", "trace"] {
            assert!(
                rows.iter().any(|row| row.contains(label)),
                "up cue pushed {label} out of the viewport"
            );
        }
        assert!(
            rows.iter().all(|row| !row.contains("│ v │")),
            "down cue remained when the viewport reached the final row"
        );

        dash.cursor = 4;
        let rows = render_rows_at(&mut dash, 110, 14);
        let border = &rows[rows.len() - 2];
        let down = border.find("│ v │").expect("middle viewport needs down");
        let sign = border.find("┤ base ├").expect("worktree sign missing");
        let up = border.find("│ ^ │").expect("middle viewport needs up");
        assert!(down < sign && sign < up, "cue order: {border}");
    }

    #[test]
    fn viewport_detects_content_above_and_below() {
        let cases = [
            (0, 7, 7, NavigationOverflow::default()),
            (
                0,
                4,
                7,
                NavigationOverflow {
                    above: false,
                    below: true,
                },
            ),
            (
                2,
                3,
                7,
                NavigationOverflow {
                    above: true,
                    below: true,
                },
            ),
            (
                3,
                4,
                7,
                NavigationOverflow {
                    above: true,
                    below: false,
                },
            ),
        ];
        for (start, height, total, expected) in cases {
            assert_eq!(
                NavigationOverflow::from_window(start, height, total),
                expected,
                "start={start} height={height} total={total}"
            );
        }
    }

    #[test]
    fn accent_source_tints_normally_green_ui() {
        render_util::set_frame_accent(Color::Red);
        let label = section_label("global");
        assert_eq!(
            label.spans[0].style.fg,
            Some(Color::Red),
            "section labels must follow the frame accent"
        );
        let sign = branch_sign_line(&Dash::new(Vec::new()));
        assert_eq!(
            sign.spans[0].style.fg,
            Some(Color::Red),
            "branch sign must follow the frame accent"
        );
        render_util::set_frame_accent(Color::Green);
    }

    #[test]
    fn quit_prompt_outranks_every_other_accent_state() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        dash.worktree_selection = WorktreeSelection::Pin(Some("feat/x".to_string()));
        dash.quit_prompt = Some(Instant::now());
        assert_eq!(frame_accent(&dash), Color::Red);
    }

    #[test]
    fn frame_accent_does_not_latch_the_previous_frames_accent() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        render_util::set_frame_accent(frame_accent(&dash));
        dash.disarm();
        render_util::set_frame_accent(frame_accent(&dash));
        assert_eq!(
            frame_accent(&dash),
            Color::Green,
            "frame accent must derive from state only, never from the previous frame"
        );
        render_util::set_frame_accent(Color::Green);
    }

    #[test]
    fn accent_states_are_exclusive_reloading_worktree_armed() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        assert_eq!(frame_accent(&dash), Color::Yellow);
        let crumb = span_text(&breadcrumb(&dash, Color::Green).spans);
        assert!(crumb.contains("ARMED"), "crumb: {crumb}");

        dash.worktree_selection = WorktreeSelection::Pin(Some("feat/x".to_string()));
        dash.running_branch = None;
        assert!(dash.worktree_diverged());
        assert_eq!(
            frame_accent(&dash),
            Color::Yellow,
            "an armed worktree leaves the frame on the armed colour"
        );
        assert_eq!(
            sign_accent(&dash),
            ORANGE,
            "the pending worktree shows on the sign box alone"
        );
        let crumb = span_text(&breadcrumb(&dash, Color::Green).spans);
        assert!(crumb.contains("WORKTREE feat/x"), "crumb: {crumb}");
        assert!(!crumb.contains("ARMED"), "single flag only: {crumb}");

        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel();
        dash.reload = Reload::Running {
            child,
            rx,
            activity: ReloadProgress::new(),
        };
        assert_eq!(
            frame_accent(&dash),
            ORANGE,
            "reloading into a pending worktree holds the whole frame orange"
        );
        let crumb = span_text(&breadcrumb(&dash, Color::Green).spans);
        assert!(crumb.contains("RELOADING"), "crumb: {crumb}");
        assert!(!crumb.contains("WORKTREE"), "single flag only: {crumb}");

        dash.worktree_selection = WorktreeSelection::Follow;
        assert_eq!(
            frame_accent(&dash),
            Color::Red,
            "a reload with no pending worktree stays red"
        );
        if let Reload::Running { mut child, .. } = dash.reload {
            let _ = child.wait();
        }
    }

    #[test]
    fn shell_uses_last_terminal_row_after_footer_removal() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        let rows = render_rows(&mut dash);
        let border = &rows[rows.len() - 2];
        assert!(
            border.contains('└') && border.contains('┘'),
            "main panel should own the row above the sign cap: {border}"
        );
        for row in rows.iter().rev().take(2) {
            assert!(
                !row.contains("filter") && !row.contains("enter"),
                "footer text leaked onto the bottom rows: {row}"
            );
        }
    }

    #[test]
    fn trace_view_legend_exposes_the_rate_toggle() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;
        let hints = unique_hints(context_action_bindings(&dash));
        let rate = hints
            .iter()
            .find(|h| h.key == "s")
            .expect("trace legend must surface the rate toggle on 's'");
        assert!(
            rate.desc.contains("relaxed"),
            "relaxed is the default, shown in the legend: {}",
            rate.desc
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('s'), KeyModifiers::NONE),
            Action::ToggleTraceRate,
            "'s' in the trace view toggles the reporting rate"
        );
    }
}
