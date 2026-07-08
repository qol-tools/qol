use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::commands::emu::{
    emu_config_path, emu_dir, newest_run_detail, EnvironmentStatus, ImageCandidate, LastRun,
    ResolveState, RunDetail,
};

use super::render_util::{
    accent, list_window, now_unix_ms, relative_age, spaced_height, view_content,
};
use super::{
    copy_highlight, draw_run_log, frame_accent, spawn_forwarders, Dash, LogPane, LogRing, View,
    ITEM_GAP,
};

pub(super) enum EmuState {
    Probing,
    Done(Vec<EnvironmentStatus>),
    Failed(String),
}

pub(super) struct EmuDetail {
    pub(super) id: String,
    pub(super) info: Vec<Line<'static>>,
    pub(super) replay: Option<LogPane>,
}

pub(super) fn open_emu(dash: &mut Dash) {
    dash.view = View::Emu;
    dash.scroll_offset = 0;
    dash.pokes.emu = true;
}

pub(super) fn emu_env_count(dash: &Dash) -> usize {
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

pub(super) fn selected_candidate_mut(dash: &mut Dash) -> Option<&mut ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get_mut(index))
}

fn selected_candidate(dash: &Dash) -> Option<&ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get(index))
}

pub(super) fn is_running(dash: &Dash, id: &str) -> bool {
    dash.active_runs.get(id).is_some_and(LogPane::is_live)
}

pub(super) fn act_emu(dash: &mut Dash, modified: bool) {
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

pub(super) fn emu_run_line(verb: &str, detail: &str) -> String {
    format!("  {verb:<9}{detail}")
}

pub(super) fn keep_emu_line(line: &str) -> bool {
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

pub(super) fn open_emu_dir() {
    let Some(dir) = emu_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    crate::host_facade::open_path(&dir);
}

pub(super) fn confirm_selected_candidate(dash: &mut Dash) {
    let Some(emu_toml) = emu_config_path() else {
        return;
    };
    let Some(candidate) = selected_candidate(dash).cloned() else {
        return;
    };
    let qemu_img = crate::commands::emu::find_on_path("qemu-img");
    match crate::commands::emu::register_image(&emu_toml, &candidate, qemu_img.as_deref()) {
        Ok(id) => {
            dash.notice = Some((Instant::now(), format!("registered {id}")));
            dash.pokes.emu = true;
        }
        Err(error) => {
            dash.notice = Some((Instant::now(), candidate_register_error(&error.to_string())));
        }
    }
}

fn candidate_register_error(message: &str) -> String {
    if message == "arch unconfirmed" {
        return "arch unconfirmed · press t to set arch, then a".to_string();
    }
    message.to_string()
}

pub(super) fn drain_emu_runs(dash: &mut Dash) {
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

pub(super) fn stop_emu_runs(dash: &mut Dash) {
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

pub(super) fn open_emu_detail(dash: &mut Dash) {
    if let Some(status) = selected_emu_status(dash).cloned() {
        let detail = newest_run_detail(&status.id);
        let info = emu_info_lines(&status, detail.as_ref());
        set_emu_detail(dash, status.id, info, detail);
        return;
    }
    let Some(candidate) = selected_candidate(dash).cloned() else {
        return;
    };
    let detail = newest_run_detail(&candidate.id);
    let info = candidate_info_lines(&candidate, detail.as_ref());
    set_emu_detail(dash, candidate.id, info, detail);
}

fn set_emu_detail(
    dash: &mut Dash,
    id: String,
    info: Vec<Line<'static>>,
    detail: Option<RunDetail>,
) {
    let replay = if dash.active_runs.contains_key(&id) {
        None
    } else {
        detail.as_ref().map(|d| LogPane::replay(&d.run_log()))
    };
    dash.emu_detail = Some(EmuDetail { id, info, replay });
    dash.view = View::EmuDetail;
    dash.scroll_offset = 0;
    dash.close_filters();
}

pub(super) fn emu_detail_ring(dash: &Dash) -> Option<&LogRing> {
    let detail = dash.emu_detail.as_ref()?;
    if let Some(pane) = dash.active_runs.get(&detail.id) {
        return Some(&pane.ring);
    }
    detail.replay.as_ref().map(|pane| &pane.ring)
}

pub(super) fn live_verb(dash: &Dash, id: &str) -> Option<String> {
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
        ResolveState::Ready => accent(),
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
            lines.push(Line::from(
                "  press t to set arch, then a to add".fg(Color::DarkGray),
            ));
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
            accent(),
            vec![
                format!("{} envs · {ready} ready", statuses.len())
                    .fg(accent())
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

pub(super) fn emu_empty_lines(config: &str) -> Vec<Line<'static>> {
    vec![
        Line::from("  no emus found".fg(Color::DarkGray)),
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
    let caret: Span<'static> = if selected {
        "▸ ".fg(accent()).bold()
    } else {
        "  ".into()
    };
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

pub(super) fn draw_emu(frame: &mut Frame, dash: &mut Dash, area: Rect) {
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
                    "▸ ".fg(accent()).bold()
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
            live_verb(dash, &candidate.id),
        ));
    }
    let total = lines.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();
    view_content(frame, area, visible);
}

pub(super) fn draw_emu_detail(frame: &mut Frame, dash: &mut Dash, area: Rect) {
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
            &dash.filters.emu,
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
            &dash.filters.emu,
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
        "pass" => accent(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::testkit::*;

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
    fn emu_empty_lines_list_config_path() {
        let lines = emu_empty_lines("~/.config/qol-tray/emu.toml");
        assert_eq!(lines.len(), 2, "lines: {lines:?}");
    }
}
