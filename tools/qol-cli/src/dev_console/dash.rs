use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{mpsc::Receiver, Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::commands::emu::ImageCandidate;
use crate::commands::{dev_env, emu};
use crate::dev_server::{
    fetch_active_worktree, fetch_plugin_health_rows, fetch_workspace_plugins, health_ok, web_ok,
    ActiveWorktreeResponse, EndpointStatus, PluginHealthRow, WorkspacePlugin,
};
use crate::poller::Poller;

use super::activity::Activity;
use super::console_state::ConsoleState;
use super::disk::DiskPanel;
use super::doctor::{spawn_doctor, spawn_doctor_probe, DoctorMode, DoctorPanel, DoctorRun};
use super::emu_panel::{ActiveSandboxRun, EmuDetail, EmuState};
use super::feature_flags::{
    feature_flag_brick_layout, toggle_feature_flag, FeatureFlagPanel, FeatureFlags, FEATURE_FLAGS,
};
use super::filters::{filter_brick_layout, FilterState, FilterStrategy, LogFilter, ViewFilters};
use super::log_pane::{DevLogFile, LogPane};
use super::picker::{default_filter_layout_width, move_picker_selection, PickerMove};
use super::render_util::now_unix_ms;
use super::stream_view::EndpointsState;
use super::worktrees_panel::{
    arm_selected_worktree, move_worktree_selection, open_worktrees_panel, target_label,
    WorktreePanel,
};
use super::{
    EMU_REFRESH_INTERVAL, HEALTH_PROBE_INTERVAL, LINKS_REFRESH_INTERVAL, QUIT_CONFIRM_WINDOW,
};

#[derive(Debug, PartialEq, Eq, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TraceRate {
    #[default]
    Relaxed,
    Realtime,
}

impl TraceRate {
    pub(super) fn is_realtime(self) -> bool {
        matches!(self, Self::Realtime)
    }

    pub(super) fn toggled(self) -> Self {
        match self {
            Self::Relaxed => Self::Realtime,
            Self::Realtime => Self::Relaxed,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Relaxed => "rate: relaxed",
            Self::Realtime => "rate: realtime",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum TraceRenderer {
    Rust,
}

impl TraceRenderer {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
        }
    }

    pub(super) fn missing_hint(self) -> &'static str {
        match self {
            Self::Rust => "could not exec current qol binary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Dashboard,
    Logs,
    Doctor,
    Disk,
    Plugins,
    Emu,
    EmuDetail,
    Trace,
    Endpoints,
}

#[derive(Clone, Copy)]
pub(super) enum Row {
    Tray,
    Web,
    Plugins,
    Emu,
    Doctor,
    Disk,
    Logs,
    Trace,
}

impl Row {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Tray => "tray",
            Self::Web => "web",
            Self::Plugins => "plugins",
            Self::Emu => "sandboxes",
            Self::Doctor => "doctor",
            Self::Disk => "disk",
            Self::Logs => "logs",
            Self::Trace => "trace",
        }
    }
}

pub(super) const ROWS: [Row; 8] = [
    Row::Tray,
    Row::Web,
    Row::Plugins,
    Row::Emu,
    Row::Doctor,
    Row::Disk,
    Row::Logs,
    Row::Trace,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Health {
    Checking,
    Up,
    Down,
}

pub(super) struct HealthSnapshot {
    pub(super) api: bool,
    pub(super) web: bool,
}

pub(super) type EmuScanResult = Result<(qol_dev_env::Inventory, Vec<ImageCandidate>), String>;
pub(super) type LinksProbeResult =
    Result<(Vec<WorkspacePlugin>, Option<Vec<PluginHealthRow>>), String>;
pub(super) type ActiveWorktreeResult = Result<ActiveWorktreeResponse, String>;

#[derive(Clone)]
struct ProbeWorktree(Arc<Mutex<PathBuf>>);

impl ProbeWorktree {
    fn new(root: PathBuf) -> Self {
        Self(Arc::new(Mutex::new(root)))
    }

    fn current(&self) -> PathBuf {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn replace(&self, root: &Path) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = root.to_path_buf();
    }
}

pub(super) struct Probes {
    pub(super) health: Poller<HealthSnapshot>,
    pub(super) active_worktree: Poller<ActiveWorktreeResult>,
    pub(super) emu: Poller<EmuScanResult>,
    pub(super) links: Poller<LinksProbeResult>,
    pub(super) doctor: Poller<Result<DoctorRun, String>>,
    pub(super) endpoints: Option<Poller<Vec<EndpointStatus>>>,
    emu_worktree: ProbeWorktree,
}

impl Probes {
    pub(super) fn spawn(running_worktree: PathBuf) -> Self {
        let emu_worktree = ProbeWorktree::new(running_worktree);
        let emu_probe_worktree = emu_worktree.clone();
        Self {
            health: Poller::spawn(HEALTH_PROBE_INTERVAL, || HealthSnapshot {
                api: health_ok(),
                web: web_ok(),
            }),
            active_worktree: Poller::spawn(HEALTH_PROBE_INTERVAL, || {
                fetch_active_worktree().map_err(|error| format!("{error:#}"))
            }),
            emu: Poller::spawn(EMU_REFRESH_INTERVAL, move || {
                let root = emu_probe_worktree.current();
                let mut inventory =
                    dev_env::snapshot_in(&root).map_err(|error| format!("{error:#}"))?;
                let candidates = match emu::discover_all() {
                    Ok(discovered) => discovered.candidates,
                    Err(error) => {
                        inventory.issues.push(qol_dev_env::InventoryIssue {
                            path: emu::emu_config_path().unwrap_or_default(),
                            message: format!("unregistered image discovery failed: {error:#}"),
                        });
                        Vec::new()
                    }
                };
                let candidates = without_registered_candidates(&inventory, candidates);
                Ok((inventory, candidates))
            }),
            links: Poller::spawn(LINKS_REFRESH_INTERVAL, || {
                fetch_workspace_plugins()
                    .map(|plugins| (plugins, fetch_plugin_health_rows().ok().flatten()))
                    .map_err(|error| format!("{error:#}"))
            }),
            doctor: spawn_doctor_probe(),
            endpoints: None,
            emu_worktree,
        }
    }
}

fn without_registered_candidates(
    inventory: &qol_dev_env::Inventory,
    candidates: Vec<ImageCandidate>,
) -> Vec<ImageCandidate> {
    let registered = inventory
        .environments
        .iter()
        .filter(|environment| environment.resolved.state == qol_dev_env::ResolutionState::Ready)
        .filter_map(|environment| environment.resolved.image_path.as_deref())
        .map(canonical_or_self)
        .collect::<HashSet<_>>();
    candidates
        .into_iter()
        .filter(|candidate| !registered.contains(&canonical_or_self(&candidate.path)))
        .collect()
}

fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[derive(Default)]
pub(super) struct Pokes {
    pub(super) emu: bool,
    pub(super) links: bool,
    pub(super) doctor: bool,
}

pub(super) fn flush_pokes(dash: &mut Dash, probes: &Probes) {
    if std::mem::take(&mut dash.pokes.emu) {
        sync_emu_probe_worktree(dash, &probes.emu_worktree);
        probes.emu.poke();
    }
    if std::mem::take(&mut dash.pokes.links) {
        probes.links.poke();
    }
    if std::mem::take(&mut dash.pokes.doctor) {
        probes.doctor.poke();
    }
}

fn sync_emu_probe_worktree(dash: &Dash, probe_worktree: &ProbeWorktree) {
    probe_worktree.replace(&dash.running_worktree);
}

pub(super) enum LinksState {
    Unknown,
    Live(Vec<WorkspacePlugin>),
    Unreachable,
}

pub(super) enum RebuildState {
    Idle,
    Requested(Instant),
    Failed(String),
}

pub(super) struct ReloadProgress {
    pub(super) started: Instant,
    pub(super) phase: String,
    pub(super) detail: String,
}

impl ReloadProgress {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            phase: "prepare".to_string(),
            detail: "dev artifacts".to_string(),
        }
    }

    pub(super) fn activity(&self) -> Activity {
        Activity {
            title: "reload",
            phase: self.phase.clone(),
            detail: self.detail.clone(),
            elapsed: self.started.elapsed(),
        }
    }

    pub(super) fn observe(&mut self, line: &str) -> bool {
        let Some(payload) = line.strip_prefix(crate::commands::dev::DEV_RELOAD_PROGRESS_PREFIX)
        else {
            return false;
        };
        let Some((phase, detail)) = payload.split_once('\t') else {
            return false;
        };
        if phase.is_empty() || detail.is_empty() {
            return false;
        }
        self.phase = phase.to_string();
        self.detail = detail.to_string();
        true
    }
}

pub(super) enum Reload {
    Idle,
    Running {
        child: Child,
        rx: Receiver<String>,
        activity: ReloadProgress,
    },
}

pub(super) enum ReloadOutcome {
    Pending,
    Ready,
}

pub(super) struct Dash {
    pub(super) view: View,
    pub(super) logs: LogPane,
    pub(super) log_file: Option<DevLogFile>,
    pub(super) scroll_offset: usize,
    pub(super) health: Health,
    pub(super) web: Health,
    pub(super) endpoints: EndpointsState,
    pub(super) started: Instant,
    pub(super) rebuild: RebuildState,
    pub(super) plugin_reload: RebuildState,
    pub(super) plugin_names: Vec<String>,
    pub(super) emu: EmuState,
    pub(super) active_runs: HashMap<String, ActiveSandboxRun>,
    pub(super) emu_detail: Option<EmuDetail>,
    pub(super) emu_cursor: usize,
    pub(super) sandbox_flow_lanes: u32,
    pub(super) emu_candidates: Vec<ImageCandidate>,
    pub(super) log_height: usize,
    pub(super) cursor: usize,
    pub(super) plugin_cursor: usize,
    pub(super) doctor_cursor: usize,
    pub(super) doctor_detail_open: bool,
    pub(super) doctor: DoctorPanel,
    pub(super) disk: DiskPanel,
    pub(super) disk_scan_pending: bool,
    pub(super) trace: LogPane,
    pub(super) trace_unavailable: bool,
    pub(super) trace_details: bool,
    pub(super) trace_rate: TraceRate,
    pub(super) features: FeatureFlags,
    pub(super) feature_panel: FeatureFlagPanel,
    pub(super) worktree_panel: WorktreePanel,
    pub(super) worktree_selection: WorktreeSelection,
    pub(super) startup_branch: Option<String>,
    pub(super) running_branch: Option<String>,
    pub(super) running_worktree: PathBuf,
    pub(super) base_label: String,
    pub(super) boot_rx: Option<Receiver<String>>,
    pub(super) keys_hidden: bool,
    pub(super) filters: ViewFilters,
    pub(super) filter_index: usize,
    pub(super) filter_layout_width: usize,
    pub(super) filter_state: FilterState,
    pub(super) state_dirty: bool,
    pub(super) copy_count: String,
    pub(super) copying: bool,
    pub(super) notice: Option<(Instant, String)>,
    pub(super) quit_prompt: Option<Instant>,
    pub(super) armed: bool,
    pub(super) reload: Reload,
    pub(super) pokes: Pokes,
    pub(super) links: LinksState,
    pub(super) plugin_health: Option<Vec<PluginHealthRow>>,
}

impl Dash {
    #[cfg(test)]
    pub(super) fn new(plugin_names: Vec<String>) -> Self {
        Self::new_for_startup(plugin_names, None, PathBuf::from("/qol/base"))
    }

    pub(super) fn new_for_startup(
        plugin_names: Vec<String>,
        startup_branch: Option<String>,
        running_worktree: PathBuf,
    ) -> Self {
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
            sandbox_flow_lanes: 1,
            emu_candidates: Vec::new(),
            log_height: 0,
            cursor: 0,
            plugin_cursor: 0,
            doctor_cursor: 0,
            doctor_detail_open: false,
            doctor: DoctorPanel {
                last: None,
                last_at_ms: None,
                manual: None,
                error: None,
            },
            disk: DiskPanel::new(),
            disk_scan_pending: false,
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
            running_worktree,
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
            quit_prompt: None,
            armed: false,
            reload: Reload::Idle,
            pokes: Pokes::default(),
            links: LinksState::Unknown,
            plugin_health: None,
        }
    }

    pub(super) fn start_log_file(&mut self) {
        self.log_file = DevLogFile::create();
    }

    pub(super) fn push_log(&mut self, line: impl Into<String>) {
        let line = line.into();
        if let Some(log_file) = self.log_file.as_mut() {
            log_file.write_line(&line);
        }
        self.logs.push(line);
    }

    pub(super) fn is_reloading(&self) -> bool {
        matches!(self.reload, Reload::Running { .. })
    }

    pub(super) fn is_busy(&self) -> bool {
        self.activity().is_some()
    }

    pub(super) fn activity(&self) -> Option<Activity> {
        match &self.reload {
            Reload::Running { activity, .. } => Some(activity.activity()),
            Reload::Idle => self
                .doctor
                .manual
                .as_ref()
                .map(|manual| manual.activity(now_unix_ms()))
                .or_else(|| {
                    self.disk
                        .scan
                        .as_ref()
                        .map(|scan| scan.activity(now_unix_ms()))
                })
                .or_else(|| {
                    self.disk
                        .verify
                        .as_ref()
                        .map(|verify| verify.activity(now_unix_ms()))
                }),
        }
    }

    pub(super) fn quit_prompt_active(&self) -> bool {
        self.quit_prompt
            .is_some_and(|since| since.elapsed() < QUIT_CONFIRM_WINDOW)
    }

    pub(super) fn start_doctor(&mut self, mode: DoctorMode) {
        self.doctor.manual = Some(spawn_doctor(mode));
    }

    pub(super) fn active_filters(&self) -> &[LogFilter] {
        self.filters.for_view(self.view)
    }

    pub(super) fn mark_state_dirty(&mut self) {
        self.state_dirty = true;
    }

    pub(super) fn adopt_running_worktree(&mut self, running_worktree: PathBuf) {
        self.running_worktree = running_worktree;
        self.pokes.emu = true;
    }

    pub(super) fn decrease_sandbox_flow_lanes(&mut self) {
        self.sandbox_flow_lanes = self.sandbox_flow_lanes.saturating_sub(1).max(1);
    }

    pub(super) fn increase_sandbox_flow_lanes(&mut self) {
        self.sandbox_flow_lanes = self
            .sandbox_flow_lanes
            .saturating_add(1)
            .min(qol_dev_env::resources::MAX_CONCURRENT_LANES);
    }

    pub(super) fn apply_state(&mut self, state: ConsoleState) {
        self.filters = state.filters;
        self.trace_details = state.trace_details;
        self.trace_rate = state.trace_rate;
        self.keys_hidden = state.keys_hidden;
        self.features = FeatureFlags::from_ids(&state.feature_flags);
        self.state_dirty = false;
    }

    pub(super) fn to_state(&self) -> ConsoleState {
        ConsoleState {
            filters: self.filters.clone(),
            trace_details: self.trace_details,
            trace_rate: self.trace_rate,
            keys_hidden: self.keys_hidden,
            feature_flags: self.features.ids(),
        }
    }

    pub(super) fn close_filters(&mut self) {
        self.filter_index = 0;
        self.filter_state = FilterState::Closed;
    }

    pub(super) fn open_filter_manager(&mut self) {
        self.filter_state = FilterState::Managing;
        let len = self.active_filters().len();
        self.filter_index = self.filter_index.min(len.saturating_sub(1));
    }

    pub(super) fn move_filter(&mut self, direction: PickerMove) {
        let width = self.filter_layout_width;
        let active = self.active_filters();
        let layout = filter_brick_layout(active, width);
        let len = active.len();
        move_picker_selection(&mut self.filter_index, len, direction, &layout);
    }

    pub(super) fn start_filter_add(&mut self) {
        self.filter_state = FilterState::Editing {
            index: None,
            draft: String::new(),
            strategy: FilterStrategy::Include,
        };
    }

    pub(super) fn start_filter_edit(&mut self) {
        let Some(filter) = self.active_filters().get(self.filter_index).cloned() else {
            return;
        };
        self.filter_state = FilterState::Editing {
            index: Some(self.filter_index),
            draft: filter.text,
            strategy: filter.strategy,
        };
    }

    pub(super) fn save_filter_draft(&mut self) {
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

    pub(super) fn delete_selected_filter(&mut self) {
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

    pub(super) fn trace_details_enabled(&self) -> bool {
        self.trace_details
    }

    pub(super) fn trace_renderer(&self) -> TraceRenderer {
        TraceRenderer::Rust
    }

    pub(super) fn toggle_feature_flags_panel(&mut self) {
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

    pub(super) fn move_feature_flag(&mut self, direction: PickerMove) {
        let layout = feature_flag_brick_layout(self.feature_panel.layout_width);
        move_picker_selection(
            &mut self.feature_panel.selected,
            FEATURE_FLAGS.len(),
            direction,
            &layout,
        );
    }

    pub(super) fn toggle_selected_feature_flag(&mut self) {
        let Some(def) = FEATURE_FLAGS.get(self.feature_panel.selected) else {
            return;
        };
        toggle_feature_flag(def.flag);
    }

    pub(super) fn toggle_worktrees_panel(&mut self) {
        if self.worktree_panel.is_active() {
            self.worktree_panel.open = false;
            return;
        }
        self.filter_state = FilterState::Closed;
        self.feature_panel.open = false;
        self.copying = false;
        open_worktrees_panel(self);
    }

    pub(super) fn move_worktree(&mut self, direction: PickerMove) {
        move_worktree_selection(self, direction);
    }

    pub(super) fn arm_selected_worktree(&mut self) {
        arm_selected_worktree(self);
    }

    pub(super) fn worktree_diverged(&self) -> bool {
        match &self.worktree_selection {
            WorktreeSelection::Follow => false,
            WorktreeSelection::Pin(target) => *target != self.running_branch,
        }
    }

    pub(super) fn effective_worktree_target(&self) -> Option<&str> {
        match &self.worktree_selection {
            WorktreeSelection::Follow => self.startup_branch.as_deref(),
            WorktreeSelection::Pin(target) => target.as_deref(),
        }
    }

    pub(super) fn pinned_label(&self) -> String {
        let target = match &self.worktree_selection {
            WorktreeSelection::Follow => &self.running_branch,
            WorktreeSelection::Pin(target) => target,
        };
        target_label(target.as_deref(), &self.base_label)
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
        self.worktree_selection = WorktreeSelection::Follow;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorktreeSelection {
    Follow,
    Pin(Option<String>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::testkit::*;
    use crate::dev_console::*;

    use std::time::Instant;

    use ratatui::crossterm::event::KeyCode;

    use crate::dev_console::filters::FilterStrategy;
    use crate::dev_console::key_bindings::Action;

    use crate::dev_console::session::{apply_action, edit_filters};
    use crate::dev_console::stream_view::set_trace_details;

    #[test]
    fn emu_probe_worktree_switches_from_base_to_named_checkout() {
        let root = ProbeWorktree::new(PathBuf::from("/qol/base"));
        assert_eq!(root.current(), Path::new("/qol/base"));

        let mut dash = Dash::new(Vec::new());
        dash.running_worktree = PathBuf::from("/qol/worktrees/shot-speed");
        sync_emu_probe_worktree(&dash, &root);

        assert_eq!(root.current(), Path::new("/qol/worktrees/shot-speed"));
    }

    #[test]
    fn adopting_running_worktree_schedules_its_inventory_refresh() {
        let mut dash = Dash::new(Vec::new());
        assert!(!dash.pokes.emu);

        dash.adopt_running_worktree(PathBuf::from("/qol/worktrees/shot-speed"));

        assert_eq!(
            dash.running_worktree,
            Path::new("/qol/worktrees/shot-speed")
        );
        assert!(dash.pokes.emu);
    }

    #[test]
    fn typed_inventory_images_are_not_repeated_as_legacy_candidates() {
        let inventory = emu_inventory(vec![emu_env(
            "managed",
            crate::commands::emu::ResolveState::Ready,
        )]);
        let candidates = vec![emu_candidate("managed"), emu_candidate("fresh")];

        let candidates = without_registered_candidates(&inventory, candidates);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect::<Vec<_>>(),
            vec!["fresh"]
        );
    }

    #[test]
    fn missing_unverified_images_remain_available_for_verified_import() {
        let inventory = emu_inventory(vec![emu_env(
            "managed",
            crate::commands::emu::ResolveState::Missing,
        )]);
        let candidates = vec![emu_candidate("managed")];

        let candidates = without_registered_candidates(&inventory, candidates);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "managed");
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
}
