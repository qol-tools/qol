use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::dev_server::{probe_endpoints, toggle_dev_link, website_url, LinkToggle};
use crate::host_facade;
use crate::poller::Poller;

use super::console_state::{load_console_state, save_console_state};
use super::dash::{
    flush_pokes, Dash, Health, HealthSnapshot, LinksState, Probes, ReloadOutcome, Row, View, ROWS,
};
use super::disk::{
    apply_disk_outcome, disk_view_lines, open_disk, start_disk_cleanup, start_disk_scan,
};
use super::doctor::{
    apply_doctor_outcome, doctor_detail_text, doctor_scroll_len, open_doctor, spawn_doctor_probe,
    toggle_doctor_detail, DoctorMode,
};
use super::draw::{accent_state_line, draw, filterable_view, plugin_row_count, resolve_base_label};
use super::emu_panel::{
    act_emu, drain_emu_runs, emu_detail_ring, emu_detail_scroll_len, emu_env_count, open_emu,
    open_emu_detail, open_emu_dir, repair_sandbox_cleanup, run_selected_flow, stop_emu_runs,
    verify_selected_image, EmuState,
};
use super::filters::{line_matches_filters, FilterState};
use super::key_bindings::{
    action_for, is_feature_flags_shortcut, is_quit_shortcut, is_worktrees_shortcut, preserves_arm,
    Action,
};
use super::log_pane::{clamp_offset, LogRing};
use super::picker::PickerMove;
use super::reload::{
    poll_reload, restart_child_from_prebuilt, start_reload, trigger_rebuild, trigger_reload,
};
use super::stream_view::{
    open_current_log_editor, open_current_log_folder, open_trace, start_trace, stop_trace,
    toggle_trace_details, toggle_trace_rate, EndpointsState,
};
use super::tray_handle::TrayHandle;
use super::tray_handle::{stop_child, try_wait};
use super::{CRASH_TAIL, ENDPOINTS_REFRESH_INTERVAL, RELAXED_TRACE_INTERVAL, TICK};

pub(crate) enum SessionEnd {
    ChildExited(ExitStatus),
    UserQuit,
    SelfRestart { tray_pid: u32 },
}

pub(crate) fn run_session(
    child: &mut TrayHandle,
    verbose: bool,
    plugins: Vec<String>,
    lines: Receiver<String>,
    worktree_branch: Option<String>,
    running_worktree: PathBuf,
    boot: Option<Receiver<String>>,
) -> Result<SessionEnd> {
    if verbose || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return plain_session(child, &lines, boot);
    }
    let mut probes = Probes::spawn(running_worktree.clone());
    let mut dash = Dash::new_for_startup(plugins, worktree_branch, running_worktree);
    dash.base_label = resolve_base_label();
    dash.apply_state(load_console_state());
    dash.start_log_file();
    dash.boot_rx = boot;
    start_trace(&mut dash);
    start_disk_scan(&mut dash);
    let mut terminal = ratatui::init();
    let mut lines = lines;
    let result = tui_session(&mut terminal, child, &mut lines, &mut probes, &mut dash);
    ratatui::restore();
    if let Ok(SessionEnd::ChildExited(status)) = &result {
        if !status.success() {
            print_crash_tail(&dash.logs.ring);
        }
    }
    result
}

pub(super) fn print_crash_tail(logs: &LogRing) {
    let start = logs.len().saturating_sub(CRASH_TAIL);
    for line in logs.lines.iter().skip(start) {
        eprintln!("{line}");
    }
}

pub(super) fn plain_session(
    child: &mut TrayHandle,
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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum KeyOutcome {
    Quit,
    Reload,
    Handled,
}

pub(super) fn handle_key(dash: &mut Dash, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
    if dash.quit_prompt.is_some() && !is_quit_shortcut(code, mods) {
        dash.quit_prompt = None;
    }
    if is_feature_flags_shortcut(code, mods) {
        dash.toggle_feature_flags_panel();
        return KeyOutcome::Handled;
    }
    if is_worktrees_shortcut(code, mods) {
        dash.toggle_worktrees_panel();
        return KeyOutcome::Handled;
    }
    if dash.worktree_panel.is_active() {
        edit_worktrees(dash, code);
        return KeyOutcome::Handled;
    }
    if dash.feature_panel.is_active() {
        edit_feature_flags(dash, code);
        return KeyOutcome::Handled;
    }
    if dash.filter_state.is_active() {
        edit_filters(dash, code);
        return KeyOutcome::Handled;
    }
    if dash.copying {
        edit_copy(dash, code);
        return KeyOutcome::Handled;
    }
    if dash.armed && code == KeyCode::Esc {
        dash.disarm();
        return KeyOutcome::Handled;
    }
    let modified = dash.armed;
    let action = action_for(dash, code, mods);
    match action {
        Action::Quit if dash.quit_prompt_active() => KeyOutcome::Quit,
        Action::Quit => {
            if modified {
                dash.disarm();
            }
            dash.quit_prompt = Some(Instant::now());
            KeyOutcome::Handled
        }
        Action::Rebuild if modified => {
            dash.armed = false;
            KeyOutcome::Reload
        }
        action => {
            apply_action(dash, action, modified);
            if modified && !preserves_arm(action) {
                dash.disarm();
            }
            KeyOutcome::Handled
        }
    }
}

pub(super) fn tui_session(
    terminal: &mut DefaultTerminal,
    child: &mut TrayHandle,
    lines: &mut Receiver<String>,
    probes: &mut Probes,
    dash: &mut Dash,
) -> Result<SessionEnd> {
    let mut last_state = String::new();
    loop {
        while let Ok(line) = lines.try_recv() {
            dash.push_log(line);
        }
        if dash.quit_prompt.is_some() && !dash.quit_prompt_active() {
            dash.quit_prompt = None;
        }
        let state = accent_state_line(dash);
        if state != last_state {
            dash.push_log(state.clone());
            last_state = state;
        }
        if let Some(snapshot) = probes.health.latest() {
            apply_health(dash, snapshot);
        }
        if let Some(Ok(active)) = probes.active_worktree.latest() {
            dash.running_branch = active.branch;
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
            match outcome {
                Ok((links, health)) => {
                    if !dash.is_reloading() {
                        dash.plugin_health = health;
                    }
                    dash.links = LinksState::Live(links);
                }
                Err(_) => {
                    dash.plugin_health = None;
                    dash.links = LinksState::Unreachable;
                }
            }
        }
        let manual_outcome = dash
            .doctor
            .manual
            .as_ref()
            .and_then(|manual| manual.rx.try_recv().ok());
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
        let disk_outcome = dash
            .disk
            .scan
            .as_ref()
            .and_then(|scan| scan.rx.try_recv().ok());
        if let Some(outcome) = disk_outcome {
            dash.disk.scan = None;
            apply_disk_outcome(dash, outcome);
        }
        dash.trace.drain_rated(
            |_| true,
            dash.trace_rate.is_realtime(),
            Instant::now(),
            RELAXED_TRACE_INTERVAL,
        );
        drain_boot(dash);
        drain_emu_runs(dash);
        if let ReloadOutcome::Ready = poll_reload(dash) {
            match restart_child_from_prebuilt(child, lines, dash) {
                Ok(()) => {
                    stop_session_children(dash);
                    return Ok(SessionEnd::SelfRestart {
                        tray_pid: child.id(),
                    });
                }
                Err(error) => {
                    dash.push_log(format!("[qol dev] handoff failed: {error:#}"));
                    dash.notice = Some((Instant::now(), "handoff failed".to_string()));
                    dash.plugin_health = None;
                }
            }
        }
        if let Some(status) = try_wait(child)? {
            while let Ok(line) = lines.try_recv() {
                dash.push_log(line);
            }
            stop_session_children(dash);
            return Ok(SessionEnd::ChildExited(status));
        }
        flush_pokes(dash, probes);
        terminal.draw(|frame| draw(frame, dash))?;
        if let Some((code, mods)) = poll_key()? {
            match handle_key(dash, code, mods) {
                KeyOutcome::Quit => {
                    persist_if_dirty(dash);
                    stop_session_children(dash);
                    stop_child(child)?;
                    return Ok(SessionEnd::UserQuit);
                }
                KeyOutcome::Reload => start_reload(dash),
                KeyOutcome::Handled => {}
            }
        }
        persist_if_dirty(dash);
    }
}

pub(super) fn stop_session_children(dash: &mut Dash) {
    stop_trace(dash);
    stop_emu_runs(dash);
}

pub(super) fn persist_if_dirty(dash: &mut Dash) {
    if dash.state_dirty {
        save_console_state(&dash.to_state());
        dash.state_dirty = false;
    }
}

pub(super) fn drain_boot(dash: &mut Dash) {
    let mut received = Vec::new();
    if let Some(rx) = dash.boot_rx.as_ref() {
        while let Ok(line) = rx.try_recv() {
            received.push(line);
        }
    }
    for line in received {
        dash.push_log(line);
    }
}

pub(super) fn edit_feature_flags(dash: &mut Dash, code: KeyCode) {
    match code {
        KeyCode::Left => dash.move_feature_flag(PickerMove::Left),
        KeyCode::Right => dash.move_feature_flag(PickerMove::Right),
        KeyCode::Up => dash.move_feature_flag(PickerMove::Up),
        KeyCode::Down => dash.move_feature_flag(PickerMove::Down),
        KeyCode::Enter | KeyCode::Char(' ') => dash.toggle_selected_feature_flag(),
        KeyCode::Esc => dash.feature_panel.open = false,
        _ => {}
    }
}

pub(super) fn edit_worktrees(dash: &mut Dash, code: KeyCode) {
    match code {
        KeyCode::Left => dash.move_worktree(PickerMove::Left),
        KeyCode::Right => dash.move_worktree(PickerMove::Right),
        KeyCode::Up => dash.move_worktree(PickerMove::Up),
        KeyCode::Down => dash.move_worktree(PickerMove::Down),
        KeyCode::Enter => dash.arm_selected_worktree(),
        KeyCode::Esc => dash.worktree_panel.open = false,
        _ => {}
    }
}

pub(super) fn edit_filters(dash: &mut Dash, code: KeyCode) {
    if let FilterState::Editing {
        draft, strategy, ..
    } = &mut dash.filter_state
    {
        match code {
            KeyCode::Char(c) => draft.push(c),
            KeyCode::Backspace => {
                draft.pop();
            }
            KeyCode::Up | KeyCode::Down => *strategy = (*strategy).cycle(),
            KeyCode::Enter => dash.save_filter_draft(),
            KeyCode::Esc => dash.filter_state = FilterState::Managing,
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Left => dash.move_filter(PickerMove::Left),
        KeyCode::Right => dash.move_filter(PickerMove::Right),
        KeyCode::Up => dash.move_filter(PickerMove::Up),
        KeyCode::Down => dash.move_filter(PickerMove::Down),
        KeyCode::Enter => dash.start_filter_add(),
        KeyCode::Char('e') | KeyCode::Char('E') => dash.start_filter_edit(),
        KeyCode::Char('d') | KeyCode::Char('D') => dash.delete_selected_filter(),
        KeyCode::Esc => dash.filter_state = FilterState::Closed,
        _ => {}
    }
}

pub(super) fn edit_copy(dash: &mut Dash, code: KeyCode) {
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

pub(super) fn finish_copy(dash: &mut Dash) {
    dash.copying = false;
    let count = dash.copy_count.parse::<usize>().ok().filter(|&n| n > 0);
    dash.copy_count.clear();
    let Some(count) = count else {
        return;
    };
    let text = newest_lines(dash, count);
    copy_text_to_clipboard(dash, &text);
}

fn copy_text_to_clipboard(dash: &mut Dash, text: &str) {
    let message = match host_facade::copy_to_clipboard(text) {
        Ok(()) => format!("copied {} lines to clipboard", text.lines().count()),
        Err(error) => format!("copy failed: {error}"),
    };
    dash.notice = Some((Instant::now(), message));
}

pub(super) fn newest_lines(dash: &Dash, count: usize) -> String {
    let ring = match dash.view {
        View::Trace => Some(&dash.trace.ring),
        View::EmuDetail => emu_detail_ring(dash),
        View::Dashboard
        | View::Logs
        | View::Doctor
        | View::Disk
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
        .filter(|line| line_matches_filters(line, dash.active_filters()))
        .collect();
    let start = filtered.len().saturating_sub(count);
    filtered[start..]
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

enum CopyStrategy {
    RingBuffer,
    Instant(String),
}

fn copy_strategy(dash: &Dash) -> Option<CopyStrategy> {
    match dash.view {
        View::Doctor => {
            doctor_detail_text(&dash.doctor, dash.doctor_cursor).map(CopyStrategy::Instant)
        }
        _ if filterable_view(dash.view) => Some(CopyStrategy::RingBuffer),
        _ => None,
    }
}

enum ScrollDirection {
    Up,
    Down,
}

fn apply_scroll(dash: &mut Dash, dir: ScrollDirection) {
    match dash.view {
        View::Disk | View::Endpoints => move_top_anchored(&mut dash.scroll_offset, dir),
        _ => match scroll_cursor_and_total(dash) {
            Some((cursor, total)) => move_cursor(cursor, total, dir),
            None => move_stream(&mut dash.scroll_offset, dir),
        },
    }
}

fn move_top_anchored(offset: &mut usize, dir: ScrollDirection) {
    match dir {
        ScrollDirection::Up => *offset = offset.saturating_sub(1),
        ScrollDirection::Down => *offset = offset.saturating_add(1),
    }
}

fn scroll_cursor_and_total(dash: &mut Dash) -> Option<(&mut usize, usize)> {
    match dash.view {
        View::Dashboard => Some((&mut dash.cursor, ROWS.len())),
        View::Emu => {
            let total = emu_env_count(dash) + dash.emu_candidates.len();
            Some((&mut dash.emu_cursor, total))
        }
        View::Plugins => {
            let total = plugin_row_count(dash);
            Some((&mut dash.plugin_cursor, total))
        }
        View::Doctor => {
            let total = doctor_scroll_len(&dash.doctor);
            Some((&mut dash.doctor_cursor, total))
        }
        _ => None,
    }
}

fn move_cursor(cursor: &mut usize, total: usize, dir: ScrollDirection) {
    match dir {
        ScrollDirection::Up => *cursor = cursor.saturating_sub(1),
        ScrollDirection::Down => *cursor = (*cursor + 1).min(total.saturating_sub(1)),
    }
}

fn move_stream(offset: &mut usize, dir: ScrollDirection) {
    match dir {
        ScrollDirection::Up => *offset = offset.saturating_add(1),
        ScrollDirection::Down => *offset = offset.saturating_sub(1),
    }
}

pub(super) fn strip_ansi(raw: &str) -> String {
    use ansi_to_tui::IntoText;
    let Ok(text) = raw.into_text() else {
        return raw.to_string();
    };
    text.lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
        .collect()
}

pub(super) fn copy_highlight(dash: &Dash) -> Option<usize> {
    if !dash.copying {
        return None;
    }
    dash.copy_count.parse::<usize>().ok().filter(|&n| n > 0)
}

pub(super) fn poll_key() -> Result<Option<(KeyCode, KeyModifiers)>> {
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

pub(super) fn apply_action(dash: &mut Dash, action: Action, modified: bool) {
    let page = dash.log_height.max(1);
    match action {
        Action::ToggleKeys => {
            dash.keys_hidden = !dash.keys_hidden;
            dash.mark_state_dirty();
        }
        Action::ToggleArm => {
            if dash.armed {
                dash.disarm();
            } else {
                dash.armed = true;
            }
        }
        Action::FeatureFlags => dash.toggle_feature_flags_panel(),
        Action::Worktrees => dash.toggle_worktrees_panel(),
        Action::Rebuild => {
            trigger_rebuild(dash);
            trigger_reload(dash);
        }
        Action::Doctor => open_doctor(dash),
        Action::ToggleTraceDetails => toggle_trace_details(dash),
        Action::ToggleTraceRate => toggle_trace_rate(dash),
        Action::Activate => match dash.view {
            View::Dashboard => act_row(dash, modified),
            View::Emu => act_emu(dash, modified),
            View::Plugins => act_plugin(dash),
            View::Disk => {
                if modified {
                    start_disk_cleanup(dash);
                } else {
                    start_disk_scan(dash);
                }
            }
            View::Doctor => toggle_doctor_detail(dash),
            View::Logs | View::Trace | View::Endpoints | View::EmuDetail => {}
        },
        Action::Dive => match dash.view {
            View::Dashboard => dive_row(dash),
            View::Emu => open_emu_detail(dash),
            View::Logs
            | View::Doctor
            | View::Disk
            | View::Plugins
            | View::Trace
            | View::Endpoints
            | View::EmuDetail => {}
        },
        Action::Back => {
            dash.doctor_detail_open = false;
            dash.view = if dash.view == View::EmuDetail {
                View::Emu
            } else {
                View::Dashboard
            };
            dash.emu_detail = None;
            dash.scroll_offset = 0;
            dash.close_filters();
        }
        Action::ScrollUp => apply_scroll(dash, ScrollDirection::Up),
        Action::ScrollDown => apply_scroll(dash, ScrollDirection::Down),
        Action::PageUp => dash.scroll_offset = dash.scroll_offset.saturating_add(page),
        Action::PageDown => dash.scroll_offset = dash.scroll_offset.saturating_sub(page),
        Action::Follow => dash.scroll_offset = 0,
        Action::Filter => {
            if filterable_view(dash.view) {
                dash.open_filter_manager();
            }
        }
        Action::Copy => match copy_strategy(dash) {
            Some(CopyStrategy::Instant(text)) => copy_text_to_clipboard(dash, &text),
            Some(CopyStrategy::RingBuffer) => {
                dash.copying = true;
                dash.copy_count.clear();
                dash.scroll_offset = 0;
            }
            None => {}
        },
        Action::OpenCurrentLogFolder => open_current_log_folder(dash),
        Action::OpenCurrentLogEditor => open_current_log_editor(dash, false),
        Action::OpenCurrentLogRaw => open_current_log_editor(dash, true),
        Action::OpenEmuDir => {
            if dash.view == View::Emu {
                open_emu_dir(dash);
            }
        }
        Action::RunSandboxFlow => {
            if dash.view == View::Emu {
                run_selected_flow(dash);
            }
        }
        Action::DecreaseSandboxFlowLanes => {
            if dash.view == View::Emu {
                dash.decrease_sandbox_flow_lanes();
            }
        }
        Action::IncreaseSandboxFlowLanes => {
            if dash.view == View::Emu {
                dash.increase_sandbox_flow_lanes();
            }
        }
        Action::VerifySandboxImage => {
            if dash.view == View::Emu {
                verify_selected_image(dash);
            }
        }
        Action::RepairSandboxCleanup => {
            if dash.view == View::Emu {
                repair_sandbox_cleanup(dash);
            }
        }
        Action::Quit | Action::Ignore => {}
    }
    let len = if dash.view == View::Trace {
        dash.trace.len()
    } else if dash.view == View::EmuDetail {
        emu_detail_scroll_len(dash)
    } else if dash.view == View::Disk {
        disk_view_lines(&dash.disk).len()
    } else if dash.view == View::Endpoints {
        match &dash.endpoints {
            EndpointsState::Done(items) => items.len(),
            EndpointsState::Probing => 0,
        }
    } else {
        dash.logs.len()
    };
    dash.scroll_offset = clamp_offset(len, dash.log_height, dash.scroll_offset);
}

pub(super) fn act_row(dash: &mut Dash, modified: bool) {
    match ROWS[dash.cursor] {
        Row::Tray => {
            if !modified {
                trigger_rebuild(dash);
            }
        }
        Row::Web => {
            if !modified {
                host_facade::open_url(&website_url());
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
        Row::Disk => {
            if modified {
                start_disk_cleanup(dash);
            } else {
                start_disk_scan(dash);
            }
        }
        Row::Logs | Row::Trace => {}
    }
}

pub(super) fn dive_row(dash: &mut Dash) {
    match ROWS[dash.cursor] {
        Row::Tray => {}
        Row::Web => open_endpoints(dash),
        Row::Plugins => {
            dash.view = View::Plugins;
            dash.scroll_offset = 0;
            dash.plugin_cursor = 0;
            dash.pokes.links = true;
        }
        Row::Emu => open_emu(dash),
        Row::Doctor => open_doctor(dash),
        Row::Disk => open_disk(dash),
        Row::Logs => {
            dash.view = View::Logs;
            dash.scroll_offset = 0;
        }
        Row::Trace => open_trace(dash),
    }
}

pub(super) fn open_endpoints(dash: &mut Dash) {
    dash.view = View::Endpoints;
    dash.scroll_offset = 0;
}

pub(super) fn health_state(up: bool) -> Health {
    if up {
        Health::Up
    } else {
        Health::Down
    }
}

pub(super) fn apply_health(dash: &mut Dash, snapshot: HealthSnapshot) {
    let was_up = dash.health == Health::Up;
    dash.health = health_state(snapshot.api);
    dash.web = health_state(snapshot.web);
    if !was_up && dash.health == Health::Up {
        dash.pokes.links = true;
        dash.pokes.doctor = true;
    }
}

pub(super) fn core_log_dir() -> PathBuf {
    crate::host_facade::core_log_dir()
}

pub(super) fn act_plugin(dash: &mut Dash) {
    let selected = match &dash.links {
        LinksState::Live(rows) => rows.get(dash.plugin_cursor).cloned(),
        LinksState::Unknown | LinksState::Unreachable => None,
    };
    let Some(plugin) = selected else {
        return;
    };
    let message = match toggle_dev_link(&plugin) {
        Ok(LinkToggle::Linked) => format!("linked {}", plugin.name),
        Ok(LinkToggle::Unlinked) => format!("unlinked {}", plugin.name),
        Err(error) => format!("link failed · {error:#}"),
    };
    dash.notice = Some((Instant::now(), message));
    dash.pokes.links = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::testkit::*;
    use crate::dev_console::*;

    use std::time::Instant;

    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    use crate::commands::emu::ResolveState;
    use crate::dev_console::emu_panel::{is_running, live_verb, ActiveSandboxRun};
    use crate::dev_console::filters::FilterStrategy;
    use crate::dev_console::key_bindings::Action;

    use crate::dev_console::draw::breadcrumb;
    use crate::dev_console::session::{apply_action, edit_filters, handle_key, KeyOutcome};

    #[test]
    fn diving_into_emu_requests_an_emu_poke() {
        let mut dash = Dash::new(Vec::new());
        dash.cursor = 3;
        apply_action(&mut dash, Action::Dive, false);
        assert!(dash.pokes.emu, "emu dive marks the emu probe dirty");
        assert!(matches!(dash.view, View::Emu), "dive opened the emu view");
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
    fn emu_row_opens_emu_view() {
        let mut dash = Dash::new(Vec::new());
        dash.cursor = 3;
        apply_action(&mut dash, Action::Activate, false);
        assert!(matches!(dash.view, View::Emu));
    }

    #[test]
    fn emu_cursor_moves_and_clamps_without_scrolling() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]));
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

    #[test]
    fn emu_cursor_extends_into_candidate_rows_and_clamps() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]));
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
    fn verification_refuses_candidate_without_an_exact_missing_environment() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![emu_env("foo", ResolveState::Ready)]));
        dash.emu_candidates = vec![emu_candidate("tokenless")];
        dash.emu_cursor = 1;

        apply_action(&mut dash, Action::VerifySandboxImage, false);

        assert!(!dash.pokes.emu, "refusal must not refresh registered envs");
        let notice = dash.notice.as_ref().map(|(_, message)| message.as_str());
        assert_eq!(
            notice,
            Some("no missing environment exactly expects /a/b/tokenless.qcow2")
        );
    }

    #[test]
    fn image_import_start_failure_is_visible_in_notice_and_environment_log() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![emu_env(
            "linux/mint",
            ResolveState::Missing,
        )]));
        dash.emu_candidates = vec![emu_candidate("mint")];

        apply_action(&mut dash, Action::VerifySandboxImage, false);

        let notice = dash.notice.as_ref().map(|(_, message)| message.as_str());
        assert!(notice.is_some_and(|message| {
            message.starts_with("image verification failed to start:")
                && message.ends_with("· → opens error log")
        }));
        let run = dash.active_runs.get("linux/mint").unwrap();
        assert!(!is_running(&dash, "linux/mint"));
        assert!(run
            .pane
            .ring
            .lines
            .back()
            .is_some_and(|line| line.contains("error")));
    }

    #[test]
    fn act_emu_refuses_envs_that_are_not_ready() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![emu_env("foo", ResolveState::Missing)]));
        act_emu(&mut dash, false);
        assert!(
            dash.active_runs.is_empty(),
            "a not-ready emu does not start a run"
        );
    }

    #[test]
    fn sandbox_flow_action_requires_a_manifest_selected_workflow() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![emu_env("foo", ResolveState::Ready)]));

        apply_action(&mut dash, Action::RunSandboxFlow, false);

        assert!(dash.active_runs.is_empty());
        assert_eq!(
            dash.notice.as_ref().map(|(_, notice)| notice.as_str()),
            Some("foo has no manifest-selected default workflow")
        );
    }

    #[test]
    fn sandbox_flow_lane_actions_start_at_one_and_clamp_to_resource_limit() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        assert_eq!(dash.sandbox_flow_lanes, 1);

        apply_action(&mut dash, Action::DecreaseSandboxFlowLanes, false);
        assert_eq!(dash.sandbox_flow_lanes, 1);

        for _ in 0..=qol_dev_env::resources::MAX_CONCURRENT_LANES {
            apply_action(&mut dash, Action::IncreaseSandboxFlowLanes, false);
        }
        assert_eq!(
            dash.sandbox_flow_lanes,
            qol_dev_env::resources::MAX_CONCURRENT_LANES
        );

        apply_action(&mut dash, Action::IncreaseSandboxFlowLanes, false);
        assert_eq!(
            dash.sandbox_flow_lanes,
            qol_dev_env::resources::MAX_CONCURRENT_LANES
        );
        apply_action(&mut dash, Action::DecreaseSandboxFlowLanes, false);
        assert_eq!(
            dash.sandbox_flow_lanes,
            qol_dev_env::resources::MAX_CONCURRENT_LANES - 1
        );
    }

    #[test]
    fn diving_into_an_emu_opens_its_detail() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![emu_env("foo", ResolveState::Ready)]));
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
    fn diving_into_candidate_opens_its_live_log() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![emu_env("foo", ResolveState::Ready)]));
        dash.emu_candidates = vec![emu_candidate("linuxmint")];
        dash.emu_cursor = 1;
        dash.active_runs.insert(
            "linuxmint".to_string(),
            ActiveSandboxRun::candidate(live_pane("  boot     linuxmint · qmp")),
        );

        apply_action(&mut dash, Action::Dive, false);

        assert!(matches!(dash.view, View::EmuDetail));
        assert_eq!(
            dash.emu_detail.as_ref().map(|detail| detail.id.as_str()),
            Some("linuxmint")
        );
        assert_eq!(
            emu_detail_ring(&dash)
                .and_then(|ring| ring.lines.back())
                .map(String::as_str),
            Some("  boot     linuxmint · qmp")
        );
    }

    #[test]
    fn live_run_state_exposes_running_detail() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(emu_inventory(vec![emu_env("foo", ResolveState::Ready)]));
        dash.active_runs.insert(
            "foo".to_string(),
            ActiveSandboxRun::environment(live_pane("  boot     foo · qmp"), "foo-batch"),
        );
        assert!(is_running(&dash, "foo"));
        assert_eq!(live_verb(&dash, "foo").as_deref(), Some("boot"));
    }

    #[test]
    fn armed_ctrl_r_requests_reload_from_dashboard() {
        let mut dash = Dash::new(Vec::new());
        assert!(
            dash.view == View::Dashboard,
            "dashboard is the landing view"
        );

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char(' '), KeyModifiers::NONE),
            KeyOutcome::Handled
        );
        assert!(dash.armed, "space arms in the dashboard");

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char('r'), KeyModifiers::CONTROL),
            KeyOutcome::Reload,
            "armed ctrl+r reloads instead of rebuilding"
        );
        assert!(!dash.armed, "reload consumes the armed state");
    }

    #[test]
    fn armed_ctrl_r_requests_reload_from_trace() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char(' '), KeyModifiers::NONE),
            KeyOutcome::Handled
        );
        assert!(dash.armed, "space arms in the trace view");

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char('r'), KeyModifiers::CONTROL),
            KeyOutcome::Reload,
            "armed ctrl+r reloads from the trace view too"
        );
        assert!(!dash.armed, "reload consumes the armed state");
    }

    #[test]
    fn esc_disarms_without_quitting() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        assert_eq!(
            handle_key(&mut dash, KeyCode::Esc, KeyModifiers::NONE),
            KeyOutcome::Handled
        );
        assert!(!dash.armed, "esc clears the armed state");
    }

    #[test]
    fn feature_flags_panel_reuses_picker_controls() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        dash.toggle_feature_flags_panel();

        assert!(dash.feature_panel.is_active());
        assert!(
            matches!(dash.filter_state, FilterState::Closed),
            "feature flags supersede filter modal"
        );
        let text = render_text(&mut dash);
        assert!(text.contains("feature flags"), "missing feature panel");
        assert!(text.contains("select flag"), "missing feature keys");
        assert!(text.contains("no feature flags"), "missing empty state");
        assert_eq!(dash.trace_renderer(), TraceRenderer::Rust);
        assert!(
            !dash.trace_details_enabled(),
            "details are not a feature flag"
        );

        edit_feature_flags(&mut dash, KeyCode::Enter);
        assert_eq!(dash.trace_renderer(), TraceRenderer::Rust);
        assert!(
            !dash.trace_details_enabled(),
            "renderer flag must not toggle details"
        );
        edit_feature_flags(&mut dash, KeyCode::Char(' '));
        assert_eq!(dash.trace_renderer(), TraceRenderer::Rust);
        edit_feature_flags(&mut dash, KeyCode::Esc);
        assert!(!dash.feature_panel.is_active(), "esc closes feature panel");
    }

    #[test]
    fn worktree_panel_arms_base_without_building() {
        let mut dash = Dash::new_for_startup(
            Vec::new(),
            Some("feat/argv".to_string()),
            PathBuf::from("/qol/feat-argv"),
        );
        dash.worktree_panel.open = true;
        dash.worktree_panel.targets = vec![worktrees_panel::WorktreeTarget {
            branch: None,
            id: "base".to_string(),
        }];
        dash.worktree_panel.selected = 0;

        edit_worktrees(&mut dash, KeyCode::Enter);

        assert_eq!(dash.worktree_selection, WorktreeSelection::Pin(None));
        assert!(dash.armed, "enter arms the selected target");
        assert!(!dash.worktree_panel.is_active(), "enter closes the panel");
    }

    #[test]
    fn worktree_panel_closes_without_changing_target() {
        let mut dash = Dash::new_for_startup(
            Vec::new(),
            Some("feat/argv".to_string()),
            PathBuf::from("/qol/feat-argv"),
        );
        dash.worktree_panel.open = true;
        dash.worktree_panel.targets = vec![worktrees_panel::WorktreeTarget {
            branch: None,
            id: "base".to_string(),
        }];

        edit_worktrees(&mut dash, KeyCode::Esc);

        assert_eq!(dash.worktree_selection, WorktreeSelection::Follow);
        assert_eq!(dash.effective_worktree_target(), Some("feat/argv"));
        assert!(!dash.worktree_panel.is_active());
    }

    #[test]
    fn filter_manager_arrows_follow_brick_rows() {
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

        edit_filters(&mut dash, KeyCode::Right);
        assert_eq!(dash.filter_index, 1, "right selects next brick");
        edit_filters(&mut dash, KeyCode::Down);
        assert_eq!(dash.filter_index, 2, "down selects nearest brick below");
        edit_filters(&mut dash, KeyCode::Up);
        assert_eq!(dash.filter_index, 0, "up selects nearest brick above");
        edit_filters(&mut dash, KeyCode::Left);
        assert_eq!(dash.filter_index, 2, "left wraps to previous row tail");
    }

    #[test]
    fn toggle_keys_hides_and_restores_the_hud() {
        let mut dash = Dash::new(Vec::new());
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(dash.keys_hidden);
        let text = render_text(&mut dash);
        assert!(!text.contains("rebuild tray+plugins"), "hud still rendered");
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(!dash.keys_hidden);
        let text = render_text(&mut dash);
        assert!(
            text.contains("rebuild tray+plugins"),
            "hud did not come back"
        );
    }

    #[test]
    fn disarming_cancels_pending_worktree_switch() {
        let mut dash = Dash::new(Vec::new());
        dash.worktree_panel.open = true;
        dash.worktree_panel.targets = vec![worktrees_panel::WorktreeTarget {
            branch: Some("feat/x".to_string()),
            id: "feat/x".to_string(),
        }];
        edit_worktrees(&mut dash, KeyCode::Enter);
        assert!(dash.armed && dash.worktree_diverged());
        assert_eq!(frame_accent(&dash), ORANGE);

        handle_key(&mut dash, KeyCode::Char(' '), KeyModifiers::NONE);

        assert!(!dash.armed, "space toggles the arm off");
        assert!(
            !dash.worktree_diverged(),
            "disarm cancels the pending switch"
        );
        assert_eq!(dash.worktree_selection, WorktreeSelection::Follow);
        assert_eq!(frame_accent(&dash), Color::Green);
    }

    #[test]
    fn plain_arm_disarm_stays_green_when_running_branch_updates() {
        let mut dash = Dash::new_for_startup(Vec::new(), None, PathBuf::from("/qol/base"));
        handle_key(&mut dash, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(frame_accent(&dash), Color::Yellow);

        dash.running_branch = Some("feat/x".to_string());
        handle_key(&mut dash, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(frame_accent(&dash), Color::Green);

        dash.running_branch = None;
        assert_eq!(
            frame_accent(&dash),
            Color::Green,
            "without an explicit selection the accent must follow the running branch, never diverge"
        );
    }

    #[test]
    fn armed_reload_keeps_the_pending_worktree_target() {
        let mut dash = Dash::new(Vec::new());
        dash.worktree_panel.open = true;
        dash.worktree_panel.targets = vec![worktrees_panel::WorktreeTarget {
            branch: Some("feat/x".to_string()),
            id: "feat/x".to_string(),
        }];
        edit_worktrees(&mut dash, KeyCode::Enter);

        let outcome = handle_key(&mut dash, KeyCode::Char('r'), KeyModifiers::CONTROL);

        assert_eq!(outcome, KeyOutcome::Reload);
        assert_eq!(
            dash.worktree_selection,
            WorktreeSelection::Pin(Some("feat/x".to_string())),
            "the reload must still consume the armed target"
        );
    }

    #[test]
    fn ctrl_q_quits_only_on_second_press_within_the_window() {
        let mut dash = Dash::new(Vec::new());
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(
            handle_key(&mut dash, KeyCode::Char('q'), ctrl),
            KeyOutcome::Handled,
            "first press must open the confirmation, not quit"
        );
        assert!(dash.quit_prompt_active());
        assert_eq!(frame_accent(&dash), Color::Red, "confirm window turns red");
        let line = breadcrumb(&dash, frame_accent(&dash));
        assert!(
            line.spans.iter().any(|span| span.content.contains("QUIT?")),
            "breadcrumb must flag the pending quit"
        );
        assert_eq!(
            handle_key(&mut dash, KeyCode::Char('q'), ctrl),
            KeyOutcome::Quit,
            "second press inside the window quits"
        );
    }

    #[test]
    fn any_other_key_dismisses_the_quit_prompt() {
        let mut dash = Dash::new(Vec::new());
        handle_key(&mut dash, KeyCode::Char('q'), KeyModifiers::CONTROL);
        handle_key(&mut dash, KeyCode::Down, KeyModifiers::NONE);
        assert!(!dash.quit_prompt_active());
        assert_eq!(frame_accent(&dash), Color::Green);
    }

    #[test]
    fn quit_prompt_expires_and_a_late_press_reopens_instead_of_quitting() {
        let mut dash = Dash::new(Vec::new());
        dash.quit_prompt = Instant::now().checked_sub(QUIT_CONFIRM_WINDOW);
        assert!(!dash.quit_prompt_active(), "window must close after 3s");
        assert_eq!(frame_accent(&dash), Color::Green);
        assert_eq!(
            handle_key(&mut dash, KeyCode::Char('q'), KeyModifiers::CONTROL),
            KeyOutcome::Handled,
            "a press after expiry restarts the confirmation"
        );
    }

    #[test]
    fn edit_filters_adds_cycles_edits_and_deletes() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        edit_filters(&mut dash, KeyCode::Enter);
        for c in "focus".chars() {
            edit_filters(&mut dash, KeyCode::Char(c));
        }
        edit_filters(&mut dash, KeyCode::Backspace);
        edit_filters(&mut dash, KeyCode::Down);
        edit_filters(&mut dash, KeyCode::Enter);
        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Exclude, "focu")]
        );
        assert!(matches!(dash.filter_state, FilterState::Managing));

        edit_filters(&mut dash, KeyCode::Char('e'));
        edit_filters(&mut dash, KeyCode::Backspace);
        edit_filters(&mut dash, KeyCode::Up);
        edit_filters(&mut dash, KeyCode::Enter);
        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Include, "foc")]
        );

        edit_filters(&mut dash, KeyCode::Char('d'));
        assert!(
            dash.active_filters().is_empty(),
            "d deletes the selected filter"
        );
        edit_filters(&mut dash, KeyCode::Esc);
        assert!(matches!(dash.filter_state, FilterState::Closed));
    }

    #[test]
    fn filters_are_per_view_and_survive_navigation() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        set_active_filters(
            &mut dash,
            vec![log_filter(FilterStrategy::Exclude, "GHOSTWIN")],
        );
        dash.view = View::Trace;
        set_active_filters(
            &mut dash,
            vec![log_filter(FilterStrategy::Include, "focus")],
        );

        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Exclude, "GHOSTWIN")],
            "logs view keeps its own filter set"
        );
        assert_eq!(
            dash.filters.trace,
            vec![log_filter(FilterStrategy::Include, "focus")],
            "trace view keeps a separate filter set"
        );

        dash.view = View::Logs;
        apply_action(&mut dash, Action::Back, false);
        assert_eq!(dash.view, View::Dashboard);
        dash.cursor = ROWS
            .iter()
            .position(|row| matches!(row, Row::Logs))
            .unwrap();
        apply_action(&mut dash, Action::Dive, false);
        assert_eq!(dash.view, View::Logs);
        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Exclude, "GHOSTWIN")],
            "navigating away and back must not wipe per-view filters"
        );
        assert_eq!(
            dash.filters.trace,
            vec![log_filter(FilterStrategy::Include, "focus")],
            "other views keep their filters through navigation"
        );
    }

    #[test]
    fn rate_toggle_flips_relaxed_and_realtime_and_persists() {
        let mut dash = Dash::new(Vec::new());
        assert_eq!(dash.trace_rate, TraceRate::Relaxed, "relaxed by default");
        apply_action(&mut dash, Action::ToggleTraceRate, false);
        assert_eq!(dash.trace_rate, TraceRate::Realtime);
        assert!(dash.state_dirty, "rate change must persist");
        apply_action(&mut dash, Action::ToggleTraceRate, false);
        assert_eq!(dash.trace_rate, TraceRate::Relaxed);
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
        set_active_filters(&mut dash, vec![log_filter(FilterStrategy::Include, "a")]);
        assert_eq!(
            newest_lines(&dash, 2),
            "gamma\ndelta",
            "filter keeps only matching lines before taking the tail"
        );
        set_active_filters(&mut dash, vec![log_filter(FilterStrategy::Include, "beta")]);
        assert_eq!(
            newest_lines(&dash, 5),
            "beta",
            "ansi codes are stripped from the copied text"
        );
    }
}
