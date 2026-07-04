use std::collections::HashMap;
use std::io::{BufRead, BufReader, IsTerminal, Read};
use std::path::PathBuf;
#[cfg(test)]
use std::process::Command;
use std::process::{Child, ExitStatus};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::commands::emu::ResolveState;
use crate::commands::emu::{emu_scan, EnvironmentStatus, ImageCandidate};
use crate::dev_server::{
    fetch_active_worktree, fetch_plugin_health_rows, fetch_workspace_plugins, health_ok,
    probe_endpoints, toggle_dev_link, web_ok, website_url, ActiveWorktreeResponse, EndpointStatus,
    LinkToggle, PluginDaemonStatus, PluginHealthRow, WorkspacePlugin,
};
use crate::host_facade;
use crate::poller::Poller;

mod key_bindings;
use key_bindings::{
    action_for, context_action_bindings, global_action_bindings, is_feature_flags_shortcut,
    is_worktrees_shortcut, preserves_arm, unique_hints, Action, KeyHint,
};

mod log_pane;
#[cfg(test)]
use log_pane::collapse_key;
use log_pane::{clamp_offset, window_start, DevLogFile, LogPane, LogRing};

mod picker;
use picker::{default_filter_layout_width, move_picker_selection, PickerMove};

mod filters;
#[cfg(test)]
use filters::filter_panel_rows;
use filters::{
    draw_filter_panel, filter_brick_layout, filter_scope, line_matches_filters, FilterState,
    FilterStrategy, LogFilter, ViewFilters,
};

mod feature_flags;
use feature_flags::{
    draw_feature_flags_panel, feature_flag_brick_layout, toggle_feature_flag, FeatureFlagPanel,
    FeatureFlags, FEATURE_FLAGS,
};

mod worktrees_panel;
#[cfg(test)]
use worktrees_panel::worktree_panel_rows;
use worktrees_panel::{
    arm_selected_worktree, draw_worktrees_panel, move_worktree_selection, open_worktrees_panel,
    target_label, WorktreePanel,
};

mod render_util;
#[cfg(test)]
use render_util::relative_age;
use render_util::{
    accent, format_duration, list_capacity, now_unix_ms, view_content, Sign, SignBox, ITEM_GAP,
};

mod console_state;
use console_state::{load_console_state, save_console_state, ConsoleState};

mod doctor;
use doctor::{
    apply_doctor_outcome, doctor_status, draw_doctor, open_doctor, spawn_doctor,
    spawn_doctor_probe, DoctorMode, DoctorPanel, DoctorRun,
};

mod reload;
use reload::{
    poll_reload, restart_child_from_prebuilt, start_reload, trigger_rebuild, trigger_reload,
};

mod tray_handle;
pub(crate) use tray_handle::TrayHandle;

mod emu_panel;
use emu_panel::{
    act_emu, confirm_selected_candidate, drain_emu_runs, draw_emu, draw_emu_detail,
    emu_detail_ring, emu_env_count, emu_run_line, emu_status, open_emu, open_emu_detail,
    open_emu_dir, selected_candidate_mut, stop_emu_runs, EmuDetail, EmuState,
};
#[cfg(test)]
use emu_panel::{candidate_line, emu_empty_lines, is_running, keep_emu_line, live_verb};

mod stream_view;
#[cfg(test)]
use stream_view::{current_log_source, set_trace_details, DEFAULT_TRACE_LOG_FILE};
use stream_view::{
    draw_endpoints, draw_logs, draw_run_log, draw_trace, open_current_log_editor,
    open_current_log_folder, open_trace, start_trace, stop_trace, toggle_trace_details,
    toggle_trace_rate, trace_value, EndpointsState,
};

const LOG_CAP: usize = 2000;
const TICK: Duration = Duration::from_millis(150);
const RELAXED_TRACE_INTERVAL: Duration = Duration::from_millis(300);
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const EMU_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const LINKS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const DOCTOR_BASE_INTERVAL: Duration = Duration::from_secs(10);
const DOCTOR_CAP_INTERVAL: Duration = Duration::from_secs(60);
const ENDPOINTS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(5);
const HANDOFF_STOP_GRACE: Duration = Duration::from_millis(750);
const HANDOFF_STOP_INTERVAL: Duration = Duration::from_millis(50);
const SHADOW_READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHADOW_READY_INTERVAL: Duration = Duration::from_millis(100);
const PROMOTION_TIMEOUT: Duration = Duration::from_secs(10);
const PROMOTION_INTERVAL: Duration = Duration::from_millis(100);
const CRASH_TAIL: usize = 40;
const ACK_TTL: Duration = Duration::from_secs(6);
pub(super) const ORANGE: Color = Color::Rgb(255, 153, 0);
const BASE_ACCENT: Color = Color::Green;

pub(crate) enum SessionEnd {
    ChildExited(ExitStatus),
    UserQuit,
    SelfRestart { tray_pid: u32 },
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TraceRate {
    #[default]
    Relaxed,
    Realtime,
}

impl TraceRate {
    fn is_realtime(self) -> bool {
        matches!(self, Self::Realtime)
    }

    fn toggled(self) -> Self {
        match self {
            Self::Relaxed => Self::Realtime,
            Self::Realtime => Self::Relaxed,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Relaxed => "rate: relaxed",
            Self::Realtime => "rate: realtime",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TraceRenderer {
    Rust,
}

impl TraceRenderer {
    fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
        }
    }

    fn missing_hint(self) -> &'static str {
        match self {
            Self::Rust => "could not exec current qol binary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
type LinksProbeResult = Result<(Vec<WorkspacePlugin>, Option<Vec<PluginHealthRow>>), String>;
type ActiveWorktreeResult = Result<ActiveWorktreeResponse, String>;

struct Probes {
    health: Poller<HealthSnapshot>,
    active_worktree: Poller<ActiveWorktreeResult>,
    emu: Poller<EmuScanResult>,
    links: Poller<LinksProbeResult>,
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
            active_worktree: Poller::spawn(HEALTH_PROBE_INTERVAL, || {
                fetch_active_worktree().map_err(|error| format!("{error:#}"))
            }),
            emu: Poller::spawn(EMU_REFRESH_INTERVAL, || {
                emu_scan().map_err(|error| format!("{error:#}"))
            }),
            links: Poller::spawn(LINKS_REFRESH_INTERVAL, || {
                fetch_workspace_plugins()
                    .map(|plugins| (plugins, fetch_plugin_health_rows().ok().flatten()))
                    .map_err(|error| format!("{error:#}"))
            }),
            doctor: spawn_doctor_probe(),
            endpoints: None,
        }
    }
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

enum LinksState {
    Unknown,
    Live(Vec<WorkspacePlugin>),
    Unreachable,
}

enum RebuildState {
    Idle,
    Requested(Instant),
    Failed(String),
}

enum Reload {
    Idle,
    Running { child: Child, rx: Receiver<String> },
}

enum ReloadOutcome {
    Pending,
    Ready,
}

struct Dash {
    view: View,
    logs: LogPane,
    log_file: Option<DevLogFile>,
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
    plugin_cursor: usize,
    doctor: DoctorPanel,
    trace: LogPane,
    trace_unavailable: bool,
    trace_details: bool,
    trace_rate: TraceRate,
    features: FeatureFlags,
    feature_panel: FeatureFlagPanel,
    worktree_panel: WorktreePanel,
    worktree_selection: WorktreeSelection,
    startup_branch: Option<String>,
    running_branch: Option<String>,
    base_label: String,
    boot_rx: Option<Receiver<String>>,
    keys_hidden: bool,
    filters: ViewFilters,
    filter_index: usize,
    filter_layout_width: usize,
    filter_state: FilterState,
    state_dirty: bool,
    copy_count: String,
    copying: bool,
    notice: Option<(Instant, String)>,
    armed: bool,
    reload: Reload,
    pokes: Pokes,
    links: LinksState,
    plugin_health: Option<Vec<PluginHealthRow>>,
}

impl Dash {
    #[cfg(test)]
    fn new(plugin_names: Vec<String>) -> Self {
        Self::new_for_startup(plugin_names, None)
    }

    fn new_for_startup(plugin_names: Vec<String>, startup_branch: Option<String>) -> Self {
        Self {
            view: View::Dashboard,
            logs: LogPane::new(),
            log_file: None,
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
            plugin_cursor: 0,
            doctor: DoctorPanel {
                last: None,
                last_at_ms: None,
                manual: None,
                error: None,
            },
            trace: LogPane::collapsing(),
            trace_unavailable: false,
            trace_details: false,
            trace_rate: TraceRate::default(),
            features: FeatureFlags::default(),
            feature_panel: FeatureFlagPanel {
                layout_width: default_filter_layout_width(),
                ..FeatureFlagPanel::default()
            },
            worktree_panel: WorktreePanel {
                layout_width: default_filter_layout_width(),
                ..WorktreePanel::default()
            },
            worktree_selection: WorktreeSelection::Follow,
            startup_branch: startup_branch.clone(),
            running_branch: startup_branch,
            base_label: "base".to_string(),
            boot_rx: None,
            keys_hidden: false,
            filters: ViewFilters::default(),
            filter_index: 0,
            filter_layout_width: default_filter_layout_width(),
            filter_state: FilterState::Closed,
            state_dirty: false,
            copy_count: String::new(),
            copying: false,
            notice: None,
            armed: false,
            reload: Reload::Idle,
            pokes: Pokes::default(),
            links: LinksState::Unknown,
            plugin_health: None,
        }
    }

    fn start_log_file(&mut self) {
        self.log_file = DevLogFile::create();
    }

    fn push_log(&mut self, line: impl Into<String>) {
        let line = line.into();
        if let Some(log_file) = self.log_file.as_mut() {
            log_file.write_line(&line);
        }
        self.logs.push(line);
    }

    fn is_reloading(&self) -> bool {
        matches!(self.reload, Reload::Running { .. })
    }

    fn start_doctor(&mut self, mode: DoctorMode) {
        self.doctor.manual = Some((mode, spawn_doctor(mode)));
    }

    fn active_filters(&self) -> &[LogFilter] {
        self.filters.for_view(self.view)
    }

    fn mark_state_dirty(&mut self) {
        self.state_dirty = true;
    }

    fn apply_state(&mut self, state: ConsoleState) {
        self.filters = state.filters;
        self.trace_details = state.trace_details;
        self.trace_rate = state.trace_rate;
        self.keys_hidden = state.keys_hidden;
        self.features = FeatureFlags::from_ids(&state.feature_flags);
        self.state_dirty = false;
    }

    fn to_state(&self) -> ConsoleState {
        ConsoleState {
            filters: self.filters.clone(),
            trace_details: self.trace_details,
            trace_rate: self.trace_rate,
            keys_hidden: self.keys_hidden,
            feature_flags: self.features.ids(),
        }
    }

    fn close_filters(&mut self) {
        self.filter_index = 0;
        self.filter_state = FilterState::Closed;
    }

    fn open_filter_manager(&mut self) {
        self.filter_state = FilterState::Managing;
        let len = self.active_filters().len();
        self.filter_index = self.filter_index.min(len.saturating_sub(1));
    }

    fn move_filter(&mut self, direction: PickerMove) {
        let width = self.filter_layout_width;
        let active = self.active_filters();
        let layout = filter_brick_layout(active, width);
        let len = active.len();
        move_picker_selection(&mut self.filter_index, len, direction, &layout);
    }

    fn start_filter_add(&mut self) {
        self.filter_state = FilterState::Editing {
            index: None,
            draft: String::new(),
            strategy: FilterStrategy::Include,
        };
    }

    fn start_filter_edit(&mut self) {
        let Some(filter) = self.active_filters().get(self.filter_index).cloned() else {
            return;
        };
        self.filter_state = FilterState::Editing {
            index: Some(self.filter_index),
            draft: filter.text,
            strategy: filter.strategy,
        };
    }

    fn save_filter_draft(&mut self) {
        let (index, strategy, text) = match &self.filter_state {
            FilterState::Editing {
                index,
                draft,
                strategy,
            } => (*index, *strategy, draft.trim().to_string()),
            _ => return,
        };
        if text.is_empty() {
            return;
        }
        let filter = LogFilter { strategy, text };
        let view = self.view;
        let Some(filters) = self.filters.for_view_mut(view) else {
            return;
        };
        let new_index = match index {
            Some(i) => match filters.get_mut(i) {
                Some(slot) => {
                    *slot = filter;
                    i
                }
                None => return,
            },
            None => {
                filters.push(filter);
                filters.len().saturating_sub(1)
            }
        };
        self.filter_index = new_index;
        self.filter_state = FilterState::Managing;
        self.mark_state_dirty();
    }

    fn delete_selected_filter(&mut self) {
        let view = self.view;
        let Some(filters) = self.filters.for_view_mut(view) else {
            return;
        };
        if filters.is_empty() {
            return;
        }
        let index = self.filter_index.min(filters.len() - 1);
        filters.remove(index);
        let len = filters.len();
        self.filter_index = index.min(len.saturating_sub(1));
        self.mark_state_dirty();
    }

    fn trace_details_enabled(&self) -> bool {
        self.trace_details
    }

    fn trace_renderer(&self) -> TraceRenderer {
        TraceRenderer::Rust
    }

    fn toggle_feature_flags_panel(&mut self) {
        if self.feature_panel.is_active() {
            self.feature_panel.open = false;
            return;
        }
        self.filter_state = FilterState::Closed;
        self.worktree_panel.open = false;
        self.copying = false;
        self.armed = false;
        self.feature_panel.open = true;
        self.feature_panel.selected = self
            .feature_panel
            .selected
            .min(FEATURE_FLAGS.len().saturating_sub(1));
    }

    fn move_feature_flag(&mut self, direction: PickerMove) {
        let layout = feature_flag_brick_layout(self.feature_panel.layout_width);
        move_picker_selection(
            &mut self.feature_panel.selected,
            FEATURE_FLAGS.len(),
            direction,
            &layout,
        );
    }

    fn toggle_selected_feature_flag(&mut self) {
        let Some(def) = FEATURE_FLAGS.get(self.feature_panel.selected) else {
            return;
        };
        toggle_feature_flag(def.flag);
    }

    fn toggle_worktrees_panel(&mut self) {
        if self.worktree_panel.is_active() {
            self.worktree_panel.open = false;
            return;
        }
        self.filter_state = FilterState::Closed;
        self.feature_panel.open = false;
        self.copying = false;
        open_worktrees_panel(self);
    }

    fn move_worktree(&mut self, direction: PickerMove) {
        move_worktree_selection(self, direction);
    }

    fn arm_selected_worktree(&mut self) {
        arm_selected_worktree(self);
    }

    fn worktree_diverged(&self) -> bool {
        match &self.worktree_selection {
            WorktreeSelection::Follow => false,
            WorktreeSelection::Pin(target) => *target != self.running_branch,
        }
    }

    fn effective_worktree_target(&self) -> Option<&str> {
        match &self.worktree_selection {
            WorktreeSelection::Follow => self.startup_branch.as_deref(),
            WorktreeSelection::Pin(target) => target.as_deref(),
        }
    }

    fn pinned_label(&self) -> String {
        let target = match &self.worktree_selection {
            WorktreeSelection::Follow => &self.running_branch,
            WorktreeSelection::Pin(target) => target,
        };
        target_label(target.as_deref(), &self.base_label)
    }

    fn disarm(&mut self) {
        self.armed = false;
        self.worktree_selection = WorktreeSelection::Follow;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorktreeSelection {
    Follow,
    Pin(Option<String>),
}

pub(crate) fn run_session(
    child: &mut TrayHandle,
    verbose: bool,
    plugins: Vec<String>,
    lines: Receiver<String>,
    worktree_branch: Option<String>,
    boot: Option<Receiver<String>>,
) -> Result<SessionEnd> {
    if verbose || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return plain_session(child, &lines, boot);
    }
    let mut probes = Probes::spawn();
    let mut dash = Dash::new_for_startup(plugins, worktree_branch);
    dash.base_label = resolve_base_label();
    dash.apply_state(load_console_state());
    dash.start_log_file();
    dash.boot_rx = boot;
    start_trace(&mut dash);
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

fn print_crash_tail(logs: &LogRing) {
    let start = logs.len().saturating_sub(CRASH_TAIL);
    for line in logs.lines.iter().skip(start) {
        eprintln!("{line}");
    }
}

fn plain_session(
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
enum KeyOutcome {
    Quit,
    Reload,
    Handled,
}

fn handle_key(dash: &mut Dash, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
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
        Action::Quit => KeyOutcome::Quit,
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

fn tui_session(
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

fn stop_session_children(dash: &mut Dash) {
    stop_trace(dash);
    stop_emu_runs(dash);
}

fn persist_if_dirty(dash: &mut Dash) {
    if dash.state_dirty {
        save_console_state(&dash.to_state());
        dash.state_dirty = false;
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
        dash.push_log(line);
    }
}

fn edit_feature_flags(dash: &mut Dash, code: KeyCode) {
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

fn edit_worktrees(dash: &mut Dash, code: KeyCode) {
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

fn edit_filters(dash: &mut Dash, code: KeyCode) {
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
    dash.notice = Some((Instant::now(), message));
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
        .filter(|line| line_matches_filters(line, dash.active_filters()))
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
            View::Logs | View::Doctor | View::Trace | View::Endpoints | View::EmuDetail => {}
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
            dash.close_filters();
        }
        Action::ScrollUp => match dash.view {
            View::Dashboard => dash.cursor = dash.cursor.saturating_sub(1),
            View::Emu => dash.emu_cursor = dash.emu_cursor.saturating_sub(1),
            View::Plugins => dash.plugin_cursor = dash.plugin_cursor.saturating_sub(1),
            View::Logs | View::Doctor | View::Trace | View::Endpoints | View::EmuDetail => {
                dash.scroll_offset = dash.scroll_offset.saturating_add(1)
            }
        },
        Action::ScrollDown => match dash.view {
            View::Dashboard => dash.cursor = (dash.cursor + 1).min(ROWS.len() - 1),
            View::Emu => {
                let total = emu_env_count(dash) + dash.emu_candidates.len();
                dash.emu_cursor = (dash.emu_cursor + 1).min(total.saturating_sub(1));
            }
            View::Plugins => {
                let total = plugin_row_count(dash);
                dash.plugin_cursor = (dash.plugin_cursor + 1).min(total.saturating_sub(1));
            }
            View::Logs | View::Doctor | View::Trace | View::Endpoints | View::EmuDetail => {
                dash.scroll_offset = dash.scroll_offset.saturating_sub(1)
            }
        },
        Action::PageUp => dash.scroll_offset = dash.scroll_offset.saturating_add(page),
        Action::PageDown => dash.scroll_offset = dash.scroll_offset.saturating_sub(page),
        Action::Follow => dash.scroll_offset = 0,
        Action::Filter => {
            if filterable_view(dash.view) {
                dash.open_filter_manager();
            }
        }
        Action::Copy => {
            if filterable_view(dash.view) {
                dash.copying = true;
                dash.copy_count.clear();
                dash.scroll_offset = 0;
            }
        }
        Action::OpenCurrentLogFolder => open_current_log_folder(dash),
        Action::OpenCurrentLogEditor => open_current_log_editor(dash, false),
        Action::OpenCurrentLogRaw => open_current_log_editor(dash, true),
        Action::OpenEmuDir => {
            if dash.view == View::Emu {
                open_emu_dir();
            }
        }
        Action::ToggleArch => {
            if dash.view == View::Emu {
                let notice = selected_candidate_mut(dash).map(|candidate| {
                    candidate.arch = candidate.arch.toggled();
                    candidate.firmware =
                        crate::commands::emu::Firmware::for_arch(candidate.arch.arch());
                    format!("{} arch {}", candidate.id, candidate.arch.as_str())
                });
                if let Some(notice) = notice {
                    dash.notice = Some((Instant::now(), notice));
                }
            }
        }
        Action::Confirm => {
            if dash.view == View::Emu {
                confirm_selected_candidate(dash);
            }
        }
        Action::Quit | Action::Ignore => {}
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
            dash.plugin_cursor = 0;
            dash.pokes.links = true;
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

fn health_state(up: bool) -> Health {
    if up {
        Health::Up
    } else {
        Health::Down
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

fn core_log_dir() -> PathBuf {
    match crate::host_facade::os_name() {
        "macos" => dirs::home_dir()
            .map(|home| home.join("Library/Logs/qol-tray"))
            .unwrap_or_else(|| PathBuf::from("/tmp/qol-tray/logs")),
        "windows" => dirs::data_local_dir()
            .map(|dir| dir.join("qol-tray/logs"))
            .unwrap_or_else(|| PathBuf::from("C:/Temp/qol-tray/logs")),
        _ => qol_config::data_dir()
            .map(|dir| dir.join("logs"))
            .unwrap_or_else(|| PathBuf::from("/tmp/qol-tray/logs")),
    }
}

fn draw(frame: &mut Frame, dash: &mut Dash) {
    let accent = frame_accent(dash);
    render_util::set_frame_accent(accent);
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
    draw_filter_panel(frame, dash, inner, accent);
    draw_feature_flags_panel(frame, dash, inner, accent);
    draw_worktrees_panel(frame, dash, inner, accent);
    Sign {
        content: breadcrumb(dash, accent),
    }
    .render(frame, body, accent);
    draw_branch_sign(frame, dash, body, accent);
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
        View::Plugins => Some("workspace plugins · enter to link/unlink"),
        View::Emu => Some("clean-os test envs"),
        View::Endpoints => Some("local service endpoints"),
        View::Dashboard | View::EmuDetail => None,
    }
}

fn filterable_view(view: View) -> bool {
    filter_scope(view).is_some()
}

fn filters_visible(dash: &Dash) -> bool {
    filterable_view(dash.view) && !dash.active_filters().is_empty()
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
    if filters_visible(dash) {
        spans.push(" · FILTERED".fg(Color::Yellow).bold());
    }
    if dash.is_reloading() {
        spans.push(" · RELOADING".fg(Color::Red).bold());
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

const KEYS_HUD_WIDTH: u16 = 34;

fn accent_state_line(dash: &Dash) -> String {
    format!(
        "[state] accent={:?} armed={} reloading={} view={:?} selection={:?} running={:?}",
        frame_accent(dash),
        dash.armed,
        dash.is_reloading(),
        dash.view,
        dash.worktree_selection,
        dash.running_branch
    )
}

fn resolve_base_label() -> String {
    crate::workspace::repo_root()
        .ok()
        .and_then(|root| qol_dev_build::tray::resolve_git_branch(&root))
        .unwrap_or_else(|| "base".to_string())
}

fn branch_sign_line(dash: &Dash) -> Line<'static> {
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

fn draw_branch_sign(frame: &mut Frame, dash: &Dash, body: Rect, accent: Color) {
    Sign {
        content: branch_sign_line(dash),
    }
    .render_bottom(frame, body, accent);
}

fn context_keys(dash: &Dash) -> Vec<KeyHint> {
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

fn global_keys(armed: bool) -> Vec<KeyHint> {
    unique_hints(global_action_bindings(armed))
}

fn key_lines(keys: &[KeyHint]) -> Vec<Line<'static>> {
    keys.iter()
        .map(|hint| {
            Line::from(vec![
                format!(" {:<9} ", hint.key).fg(Color::White).bold(),
                format!("{} ", hint.desc).fg(Color::DarkGray),
            ])
        })
        .collect()
}

fn section_label(label: &'static str) -> Line<'static> {
    Line::from(format!(" {label}").fg(accent()).bold())
}

fn keys_rows(dash: &Dash) -> Vec<Line<'static>> {
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

fn draw_keys_hud(frame: &mut Frame, dash: &Dash, area: Rect) {
    if dash.keys_hidden {
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

fn tray_status(dash: &Dash) -> (Color, Vec<Span<'static>>) {
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

fn web_status(web: Health) -> (Color, Vec<Span<'static>>) {
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

fn dash_row(selected: bool, color: Color, label: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let caret: Span<'static> = if selected {
        "▸ ".fg(accent()).bold()
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
    } else if dash.worktree_diverged() {
        ORANGE
    } else if dash.armed {
        Color::Yellow
    } else {
        BASE_ACCENT
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

fn plugin_row_count(dash: &Dash) -> usize {
    match &dash.links {
        LinksState::Live(rows) => rows.len(),
        LinksState::Unknown | LinksState::Unreachable => 0,
    }
}

fn draw_plugins(frame: &mut Frame, dash: &mut Dash, area: Rect) {
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
        return;
    }
    if dash.plugin_cursor >= total {
        dash.plugin_cursor = total - 1;
    }
    let cursor = dash.plugin_cursor;
    let start = cursor_window_start(total, height, cursor);
    let LinksState::Live(rows) = &dash.links else {
        return;
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
}

fn cursor_window_start(total: usize, height: usize, cursor: usize) -> usize {
    if height == 0 || total <= height {
        return 0;
    }
    let max_start = total - height;
    if cursor >= height {
        (cursor + 1 - height).min(max_start)
    } else {
        0
    }
}

fn plugin_row_line(
    row: &WorkspacePlugin,
    daemon_status: Option<&PluginDaemonStatus>,
    selected: bool,
) -> Line<'static> {
    let caret: Span<'static> = if selected {
        "▸ ".fg(accent()).bold()
    } else {
        "  ".into()
    };
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

fn daemon_status_span(status: Option<&PluginDaemonStatus>) -> Option<Span<'static>> {
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

fn act_plugin(dash: &mut Dash) {
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

fn try_wait(child: &mut TrayHandle) -> Result<Option<ExitStatus>> {
    child
        .try_wait()
        .context("failed polling qol-tray dev process")
}

fn stop_child(child: &mut TrayHandle) -> Result<()> {
    terminate_child(child);
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

fn terminate_child(child: &mut TrayHandle) {
    child.signal_term();
}

pub(crate) fn spawn_forwarders(child: &mut Child) -> Receiver<String> {
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

    fn log_filter(strategy: FilterStrategy, text: &str) -> LogFilter {
        LogFilter {
            strategy,
            text: text.to_string(),
        }
    }

    fn set_active_filters(dash: &mut Dash, filters: Vec<LogFilter>) {
        let view = dash.view;
        *dash
            .filters
            .for_view_mut(view)
            .expect("active view is filterable") = filters;
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

    fn workspace_plugin(name: &str, linked: bool, needs_rebuild: bool) -> WorkspacePlugin {
        WorkspacePlugin {
            id: name.to_string(),
            name: name.to_string(),
            version: if linked { "1.0.0" } else { "" }.to_string(),
            path: format!("/ws/{name}"),
            linked,
            needs_rebuild,
            rebuild_reason: if needs_rebuild { "Source changed" } else { "" }.to_string(),
        }
    }

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
    fn cursor_window_keeps_selection_visible() {
        let cases = [
            (10, 4, 0, 0),
            (10, 4, 2, 0),
            (10, 4, 4, 1),
            (10, 4, 9, 6),
            (3, 5, 2, 0),
        ];
        for (total, height, cursor, expected) in cases {
            assert_eq!(
                cursor_window_start(total, height, cursor),
                expected,
                "total={total} height={height} cursor={cursor}"
            );
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
        use crate::commands::emu::{ArchGuess, BootMedia, Firmware, GuestArch};
        ImageCandidate {
            id: id.to_string(),
            path: std::path::PathBuf::from(format!("/a/b/{id}.qcow2")),
            display_name: id.to_string(),
            arch: ArchGuess::assumed(GuestArch::X86_64),
            firmware: Firmware::Uefi,
            media: BootMedia::Disk,
        }
    }

    fn known_emu_candidate(id: &str) -> ImageCandidate {
        use crate::commands::emu::{ArchGuess, GuestArch};
        let mut candidate = emu_candidate(id);
        candidate.arch = ArchGuess::known(GuestArch::X86_64);
        candidate
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
    fn toggle_arch_flips_selected_candidate_only() {
        use crate::commands::emu::{ArchGuess, Firmware, GuestArch};
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
            vec![
                ArchGuess::assumed(GuestArch::X86_64),
                ArchGuess::assumed(GuestArch::X86_64)
            ],
            "cursor on an env row must not mutate any candidate"
        );

        dash.emu_cursor = 3;
        apply_action(&mut dash, Action::ToggleArch, false);
        assert_eq!(
            dash.emu_candidates[0].arch,
            ArchGuess::assumed(GuestArch::X86_64),
            "untouched"
        );
        assert_eq!(
            dash.emu_candidates[1].arch,
            ArchGuess::known(GuestArch::Aarch64),
            "selected candidate becomes known"
        );
        assert_eq!(
            dash.emu_candidates[1].firmware,
            Firmware::Uefi,
            "toggle refreshes firmware for the selected arch"
        );
    }

    #[test]
    fn confirm_refuses_assumed_candidate_without_refreshing_emu_list() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        dash.emu_candidates = vec![emu_candidate("tokenless")];
        dash.emu_cursor = 1;

        apply_action(&mut dash, Action::Confirm, false);

        assert!(!dash.pokes.emu, "refusal must not refresh registered envs");
        let notice = dash.notice.as_ref().map(|(_, message)| message.as_str());
        assert_eq!(
            notice,
            Some("arch unconfirmed · press t to set arch, then a")
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
    fn diving_into_candidate_opens_its_live_log() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        dash.emu_candidates = vec![emu_candidate("linuxmint")];
        dash.emu_cursor = 1;
        dash.active_runs.insert(
            "linuxmint".to_string(),
            live_pane("  boot     linuxmint · qmp"),
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
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        dash.active_runs
            .insert("foo".to_string(), live_pane("  boot     foo · qmp"));
        assert!(is_running(&dash, "foo"));
        assert_eq!(live_verb(&dash, "foo").as_deref(), Some("boot"));
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

    fn row_bounds(rows: &[String], needle: &str) -> (usize, usize) {
        let row = rows
            .iter()
            .find(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not rendered"));
        let start = row.find(needle).expect("needle already found");
        let left = row[..start].rfind('│').expect("missing left border");
        let right = row[start..]
            .find('│')
            .map(|index| start + index)
            .expect("missing right border");
        (left, right)
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
            (View::Emu, "boot · stop"),
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
                assert!(text.contains("set arch"), "missing emu o/t/a keys");
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
        assert_eq!(rows[6], " ctrl+c    quit ");
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
        let mut dash = Dash::new_for_startup(Vec::new(), Some("feat/argv".to_string()));
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
        let mut dash = Dash::new_for_startup(Vec::new(), Some("feat/argv".to_string()));
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
    fn reloading_state_drives_red_accent_and_status() {
        let mut dash = Dash::new(Vec::new());
        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel();
        dash.reload = Reload::Running { child, rx };
        assert!(dash.is_reloading());
        assert_eq!(frame_accent(&dash), Color::Red);
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
        let mut dash = Dash::new_for_startup(Vec::new(), None);
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
            ORANGE,
            "worktree change outranks armed"
        );
        let crumb = span_text(&breadcrumb(&dash, Color::Green).spans);
        assert!(crumb.contains("WORKTREE feat/x"), "crumb: {crumb}");
        assert!(!crumb.contains("ARMED"), "single flag only: {crumb}");

        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel();
        dash.reload = Reload::Running { child, rx };
        assert_eq!(frame_accent(&dash), Color::Red);
        let crumb = span_text(&breadcrumb(&dash, Color::Green).spans);
        assert!(crumb.contains("RELOADING"), "crumb: {crumb}");
        assert!(!crumb.contains("WORKTREE"), "single flag only: {crumb}");
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
    fn console_state_round_trips_through_json() {
        let state = ConsoleState {
            filters: ViewFilters {
                logs: vec![log_filter(FilterStrategy::Exclude, "noise")],
                trace: vec![log_filter(FilterStrategy::Include, "focus")],
                emu: Vec::new(),
            },
            trace_details: true,
            trace_rate: TraceRate::Realtime,
            keys_hidden: true,
            feature_flags: Vec::new(),
        };
        let json = serde_json::to_string(&state).expect("serialize console state");
        let back: ConsoleState = serde_json::from_str(&json).expect("deserialize console state");
        assert_eq!(back.filters, state.filters);
        assert!(back.trace_details);
        assert_eq!(back.trace_rate, TraceRate::Realtime);
        assert!(back.keys_hidden);
    }

    #[test]
    fn console_state_defaults_every_missing_field() {
        let state: ConsoleState = serde_json::from_str("{}").expect("empty object deserializes");
        assert!(state.filters.logs.is_empty());
        assert!(state.filters.trace.is_empty());
        assert!(!state.trace_details);
        assert_eq!(state.trace_rate, TraceRate::Relaxed);
        assert!(!state.keys_hidden);
        assert!(state.feature_flags.is_empty());
    }

    #[test]
    fn dash_applies_saved_state_without_marking_dirty_and_exports_it_back() {
        let mut dash = Dash::new(Vec::new());
        let state = ConsoleState {
            filters: ViewFilters {
                logs: Vec::new(),
                trace: vec![log_filter(FilterStrategy::Include, "focus")],
                emu: Vec::new(),
            },
            trace_details: true,
            trace_rate: TraceRate::Realtime,
            keys_hidden: true,
            feature_flags: Vec::new(),
        };
        dash.apply_state(state);

        assert_eq!(
            dash.filters.trace,
            vec![log_filter(FilterStrategy::Include, "focus")]
        );
        assert!(dash.trace_details);
        assert_eq!(dash.trace_rate, TraceRate::Realtime);
        assert!(dash.keys_hidden);
        assert!(
            !dash.state_dirty,
            "loading saved state must not schedule a redundant save"
        );

        let exported = dash.to_state();
        assert_eq!(exported.filters.trace, dash.filters.trace);
        assert!(exported.trace_details);
        assert_eq!(exported.trace_rate, TraceRate::Realtime);
        assert!(exported.keys_hidden);
    }

    #[test]
    fn persistable_mutations_mark_state_dirty() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        edit_filters(&mut dash, KeyCode::Enter);
        for c in "focus".chars() {
            edit_filters(&mut dash, KeyCode::Char(c));
        }
        edit_filters(&mut dash, KeyCode::Enter);
        assert!(dash.state_dirty, "saving a filter should mark state dirty");

        let mut dash = Dash::new(Vec::new());
        set_trace_details(&mut dash, true);
        assert!(dash.state_dirty, "toggling trace detail should mark dirty");

        let mut dash = Dash::new(Vec::new());
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(dash.state_dirty, "toggling the keys HUD should mark dirty");
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

    fn attach_lines(lines: &[&str]) -> LogPane {
        let mut pane = LogPane::collapsing();
        let child = Command::new("true").spawn().unwrap();
        let (tx, rx) = channel::<String>();
        for line in lines {
            tx.send((*line).to_string()).unwrap();
        }
        pane.attach(child, rx);
        pane
    }

    #[test]
    fn realtime_keeps_every_distinct_line() {
        let lines: Vec<String> = (0..5)
            .map(|n| format!("[10:00:00.00{n}] AMC poll {n}"))
            .collect();
        let mut pane = attach_lines(&lines.iter().map(String::as_str).collect::<Vec<_>>());
        pane.drain_rated(|_| true, true, Instant::now(), RELAXED_TRACE_INTERVAL);
        assert_eq!(pane.ring.len(), 5, "realtime keeps every distinct line");
    }

    #[test]
    fn relaxed_keeps_one_line_per_distinct_shape() {
        let lines = [
            "[10:00:00.001] [alt-tab] SHOW_LIST: n=8",
            "[10:00:00.002] [launcher] LAUNCHER_OPEN: q=\"\"",
            "[10:00:00.003] [host] FOCUS: wid=44",
            "[10:00:00.004] [cli-sessions] RECON: panes=9",
        ];
        let mut pane = attach_lines(&lines);
        pane.drain_rated(|_| true, false, Instant::now(), RELAXED_TRACE_INTERVAL);
        assert_eq!(
            pane.ring.len(),
            4,
            "relaxed keeps one line per distinct source+tag shape"
        );
    }

    #[test]
    fn relaxed_throttles_same_shape_with_varying_payload() {
        let lines = [
            "[10:00:00.001] [alt-tab] CAPTURE: targets=2 total=34ms",
            "[10:00:00.002] [alt-tab] CAPTURE: targets=2 total=27ms",
            "[10:00:00.003] [alt-tab] CAPTURE: targets=2 total=31ms",
        ];
        let mut pane = attach_lines(&lines);
        pane.drain_rated(|_| true, false, Instant::now(), RELAXED_TRACE_INTERVAL);
        assert_eq!(
            pane.ring.len(),
            1,
            "same shape whose only difference is a changing number must throttle to one"
        );
    }

    #[test]
    fn relaxed_throttles_a_burst_of_identical_lines() {
        let spam = "[10:00:00.000] [alt-tab] PREVIEW_WARM_SKIP: reason=unknown_monitors wid=44";
        let mut pane = attach_lines(&[spam, spam, spam, spam, spam]);
        pane.drain_rated(|_| true, false, Instant::now(), RELAXED_TRACE_INTERVAL);
        assert_eq!(
            pane.ring.len(),
            1,
            "a burst of identical lines throttles to one"
        );
    }

    #[test]
    fn relaxed_preserves_distinct_action_between_spam() {
        let spam = "[12:36:10.081] [alt-tab] PREVIEW_WARM_SKIP: reason=unknown_monitors wid=44";
        let action = "[12:36:10.160] [launcher] SHOW_LIST: items=42 query=\"\"";
        let mut pane = attach_lines(&[spam, spam, action, spam, spam]);
        pane.drain_rated(|_| true, false, Instant::now(), RELAXED_TRACE_INTERVAL);
        let shown: Vec<String> = pane.ring.lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(
            shown.iter().any(|l| l.contains("SHOW_LIST")),
            "distinct launcher action must survive the rate limiter; shown={shown:?}"
        );
    }

    #[test]
    fn collapse_key_ignores_timestamp_and_ansi() {
        let a = collapse_key("\u{1b}[90m[10:00:00.001]\u{1b}[0m GHOSTWIN foo");
        let b = collapse_key("\u{1b}[90m[10:00:00.999]\u{1b}[0m GHOSTWIN foo");
        assert_eq!(a, b, "the same event at different times shares a key");
        assert_eq!(a, "GHOSTWIN foo");
        assert_ne!(
            a,
            collapse_key("[10:00:00.001] FOCUS bar"),
            "distinct events have distinct keys"
        );
    }

    #[test]
    fn collapsing_ring_folds_identical_consecutive_lines_with_a_count() {
        let mut ring = LogRing::collapsing();
        for ms in ["001", "050", "120"] {
            ring.push(format!("\u{1b}[90m[10:00:00.{ms}]\u{1b}[0m GHOSTWIN foo"));
        }
        assert_eq!(
            ring.len(),
            1,
            "events differing only by timestamp collapse to one line"
        );
        let folded = strip_ansi(&ring.lines[0]);
        assert!(folded.contains("GHOSTWIN foo"), "keeps the text: {folded}");
        assert!(
            folded.contains("(\u{d7}3)"),
            "shows the repeat count: {folded}"
        );

        ring.push("[10:00:00.200] FOCUS bar".to_string());
        assert_eq!(ring.len(), 2, "a distinct event starts a new line");

        ring.push("[10:00:00.260] FOCUS bar".to_string());
        assert_eq!(ring.len(), 2, "the new event collapses on repeat too");
        assert!(strip_ansi(&ring.lines[1]).contains("(\u{d7}2)"));
    }

    #[test]
    fn non_collapsing_ring_keeps_every_line() {
        let mut ring = LogRing::new();
        ring.push("[10:00:00.001] GHOSTWIN foo".to_string());
        ring.push("[10:00:00.050] GHOSTWIN foo".to_string());
        assert_eq!(ring.len(), 2, "the logs ring never folds repeats");
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
