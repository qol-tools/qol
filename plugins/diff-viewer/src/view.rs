use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, ease_out_quint, point, px, rgb, rgba, Animation, AnimationExt as _, AnyElement, App,
    BoxShadow, ClickEvent, Context, CursorStyle, Div, ElementId, Entity, FocusHandle, Focusable,
    HighlightStyle, KeyDownEvent, ScrollDelta, ScrollWheelEvent, SharedString, StyledText,
    TextStyle, WhiteSpace, Window,
};
use qol_cache::TtlCache;
use qol_diff::constructs::{self, Construct};
use qol_diff::lexer::Lang;
use qol_diff::token_path::{token_edit_path, TokenEdit, TokenFate};
use qol_diff::transition::{NewLineFate, OldLineFate, TransitionPlan};
use qol_diff::waveform::waveform;
use qol_diff::{DiffError, FileDiff, HeatLevel, LineChange, LineKind, TokenKind};
use qol_gpui::placement::Corner;
use qol_gpui::scroll_list::ScrollList;
use qol_gpui::surface::{DragGestureState, DRAG_THRESHOLD_PX};
use qol_gpui::window_chrome::PanelChrome;
use qol_gpui::WindowBar;

use crate::field::{self, AshSpec, ConeSpec, SceneState};
use crate::overview::{HunkMarker, OverviewView};
use crate::pipeline::{self, commit_range, Facts, GitRequest, GitResult};
use crate::scrubber::{arrow_delta, Commit as ScrubCommit, ScrubberView};
use crate::surface::{self, CodeSurface, LineStyle};
use crate::terrain::{TerrainDeath, TerrainGeo, TerrainMark, TerrainSpec};
use crate::wave::{self, WaveMorph};

pub const WINDOW_WIDTH: f32 = 960.0;
pub const WINDOW_HEIGHT: f32 = 640.0;
const LIST_WIDTH: f32 = 340.0;
const ROW_HEIGHT: f32 = 26.0;
pub const LINE_HEIGHT: f32 = 18.0;
const FONT_SIZE: f32 = 13.0;
const GLYPH_WIDTH: f32 = 12.0;
const PIXELS_PER_NOTCH: f32 = 50.0;
const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const DIFF_TTL: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const HEAT_TICK: Duration = Duration::from_secs(1);
pub const TRANSITION_MS: Duration = Duration::from_millis(450);
const MORPH_FADE: f32 = 0.4;
const GHOST_DRIFT_PX: f32 = 8.0;
const SLIDE_PX: f32 = 10.0;
const EVAP_LIFT_PX: f32 = 10.0;
const EVAP_SHRINK: f32 = 0.15;
const FLASH_SPAN: f32 = 0.4;
const PULSE_PERIOD_S: f32 = 2.5;
const PULSE_AMPLITUDE: f32 = 0.08;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub added: Option<u64>,
    pub deleted: Option<u64>,
    pub added_label: Option<String>,
    pub deleted_label: Option<String>,
}

impl FileRow {
    pub fn new(path: String, added: Option<u64>, deleted: Option<u64>) -> Self {
        Self {
            added_label: added.map(|count| format!("+{count}")),
            deleted_label: deleted.map(|count| format!("-{count}")),
            path,
            added,
            deleted,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileListState {
    pub files: Vec<FileRow>,
    pub list: ScrollList,
}
impl FileListState {
    pub fn new(max_visible: usize) -> Self {
        Self {
            files: Vec::new(),
            list: ScrollList::new(max_visible),
        }
    }

    pub fn set_files(&mut self, files: Vec<FileRow>) {
        let selected = self.selected_path().map(str::to_owned);
        self.files = files;
        self.list.sync(self.files.len());
        match selected
            .as_deref()
            .and_then(|path| self.files.iter().position(|file| file.path == path))
        {
            Some(index) => {
                self.list.selected = index;
                self.list.sync(self.files.len());
            }
            None if selected.is_some() => self.list.selected = 0,
            None => {}
        }
    }

    pub fn selected_path(&self) -> Option<&str> {
        self.files
            .get(self.list.selected)
            .map(|file| file.path.as_str())
    }

    pub fn move_up(&mut self) {
        self.list.move_up();
    }

    pub fn move_down(&mut self) {
        self.list.move_down(self.files.len());
    }

    pub fn page(&mut self, delta: isize) {
        if self.files.is_empty() {
            return;
        }
        let window = self.list.max_visible as isize;
        let target = self.list.selected as isize + delta * window;
        self.list.selected = target.clamp(0, self.files.len() as isize - 1) as usize;
        self.list.sync(self.files.len());
    }

    pub fn visible_range(&self) -> std::ops::Range<usize> {
        self.list.visible_range(self.files.len())
    }
}

#[derive(Debug, Clone, PartialEq)]
enum DiffPane {
    Empty,
    Error(DiffError),
    Ready(Arc<FileDiff>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Field,
    #[allow(dead_code)]
    Unified,
    #[allow(dead_code)]
    Split,
    Wave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinePair {
    old: Option<usize>,
    new: Option<usize>,
}

struct TransitionState {
    seq: u64,
    started: Instant,
    old_lines: Vec<LineChange>,
    old_pair_of: Vec<Option<usize>>,
    old_layout: Layout,
    old_scroll: usize,
    old_gutter_width: usize,
    plan: TransitionPlan,
    token_paths: Vec<Vec<TokenEdit>>,
    old_constructs: Vec<Construct>,
    old_arms: Vec<usize>,
}

struct TokenRowContext<'a> {
    seq: u64,
    line_index: usize,
    dimmed: bool,
    age: Option<Duration>,
    line: &'a LineChange,
}

pub struct DiffView {
    repo: Option<PathBuf>,
    files: FileListState,
    pane: DiffPane,
    surface: CodeSurface,
    lines: Vec<LineChange>,
    files_collapsed: bool,
    layout: Layout,
    pairs: Vec<LinePair>,
    hunk_pair_starts: Vec<usize>,
    hunk_line_starts: Vec<usize>,
    facts_error: Option<String>,
    rendered_once: bool,
    last_fit: usize,
    generation: Arc<AtomicU64>,
    git_tx: mpsc::Sender<GitRequest>,
    results: Option<mpsc::Receiver<GitResult>>,
    diff_cache: TtlCache<(String, String), Arc<FileDiff>>,
    in_flight: Option<(String, String)>,
    line_text: Vec<SharedString>,
    line_highlights: Vec<Vec<(Range<usize>, HighlightStyle)>>,
    unified_gutters: Vec<String>,
    side_gutters: Vec<(String, String)>,
    last_markers_diff: Option<Arc<FileDiff>>,
    last_markers_layout: Layout,
    wave_folded: bool,
    field_folded: bool,
    field_fit: usize,
    field_epoch: Instant,
    pending_cone: Option<(usize, usize)>,
    cone: Option<ConeSpec>,
    font_family: SharedString,
    focus_handle: FocusHandle,
    scrubber_view: Entity<ScrubberView>,
    overview_view: Entity<OverviewView>,
    jump_rx: Option<mpsc::Receiver<f32>>,
    scrub_commits: Vec<ScrubCommit>,
    last_scrub_selected: Option<usize>,
    requested: Option<(String, String)>,
    pending_center: Option<usize>,
    scrub_anchor: usize,
    last_facts: Option<Facts>,
    heat_stamps: HashMap<String, Instant>,
    heat_stamp: Option<Instant>,
    heat_fingerprint: Vec<LineStyle>,
    transition: Option<TransitionState>,
    transition_seq: u64,
    constructs: Vec<Construct>,
    construct_arms: Vec<usize>,
    select_rx: Option<mpsc::Receiver<usize>>,
    chrome: PanelChrome,
    drag_gesture: Rc<RefCell<DragGestureState>>,
}

impl DiffView {
    pub fn new(
        repo: Option<PathBuf>,
        window_title: String,
        git_tx: mpsc::Sender<GitRequest>,
        generation: Arc<AtomicU64>,
        results: mpsc::Receiver<GitResult>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (jump_tx, jump_rx) = mpsc::channel::<f32>();
        let (select_tx, select_rx) = mpsc::channel::<usize>();
        let scrubber_view = cx.new(|cx| {
            ScrubberView::new(
                cx,
                None,
                Some(Box::new(move |index, _cx| {
                    let _ = select_tx.send(index);
                })),
            )
        });
        let overview_view = cx.new(|_cx| {
            let tx = jump_tx.clone();
            OverviewView::new(move |ratio, _app| {
                let _ = tx.send(ratio);
            })
        });
        let mut view = Self {
            drag_gesture: Rc::new(RefCell::new(DragGestureState::new(DRAG_THRESHOLD_PX))),
            repo,
            files: FileListState::new(list_max_visible()),
            pane: DiffPane::Empty,
            surface: CodeSurface::new(),
            lines: Vec::new(),
            files_collapsed: false,
            layout: Layout::Field,
            pairs: Vec::new(),
            hunk_pair_starts: Vec::new(),
            hunk_line_starts: Vec::new(),
            facts_error: None,
            rendered_once: false,
            last_fit: 0,
            generation,
            git_tx,
            results: Some(results),
            diff_cache: TtlCache::new(DIFF_TTL),
            in_flight: None,
            line_text: Vec::new(),
            line_highlights: Vec::new(),
            unified_gutters: Vec::new(),
            side_gutters: Vec::new(),
            last_markers_diff: None,
            last_markers_layout: Layout::Field,
            wave_folded: false,
            field_folded: false,
            field_fit: 1,
            field_epoch: Instant::now(),
            pending_cone: None,
            cone: None,
            font_family: pick_monospace(cx),
            focus_handle: cx.focus_handle(),
            scrubber_view,
            overview_view,
            jump_rx: Some(jump_rx),
            scrub_commits: Vec::new(),
            last_scrub_selected: None,
            requested: None,
            pending_center: None,
            scrub_anchor: 0,
            last_facts: None,
            heat_stamps: HashMap::new(),
            heat_stamp: None,
            heat_fingerprint: Vec::new(),
            transition: None,
            transition_seq: 0,
            constructs: Vec::new(),
            construct_arms: Vec::new(),
            select_rx: Some(select_rx),
            chrome: PanelChrome::new(window_title, Corner::TopLeft),
        };
        if view.repo.is_none() {
            view.facts_error =
                Some("no repository: set QOL_DIFF_REPO or launch from a git worktree".to_string());
            return view;
        }
        view.spawn_loops(cx);
        pipeline::send_boot(&view.git_tx, &view.generation);
        view
    }

    fn spawn_loops(&mut self, cx: &mut Context<Self>) {
        self.spawn_result_poll(cx);
        self.spawn_backstop(cx);
        self.spawn_heat_tick(cx);
    }

    fn spawn_result_poll(&mut self, cx: &mut Context<Self>) {
        let rx = self.results.take().expect("result poll spawned once");
        let jump_rx = self.jump_rx.take().expect("jump poll spawned once");
        let select_rx = self.select_rx.take().expect("select poll spawned once");
        let this = cx.weak_entity();
        let generation = self.generation.clone();
        cx.spawn(async move |_view, cx| loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let mut results = Vec::new();
            while let Ok(result) = rx.try_recv() {
                results.push(result);
            }
            let mut jumps = Vec::new();
            while let Ok(ratio) = jump_rx.try_recv() {
                jumps.push(ratio);
            }
            let mut selects = Vec::new();
            while let Ok(index) = select_rx.try_recv() {
                selects.push(index);
            }
            if results.is_empty() && jumps.is_empty() && selects.is_empty() {
                continue;
            }
            let current = generation.load(Ordering::SeqCst);
            let _ = this.update(cx, |view, cx| {
                let mut changed = view.apply_results(results, current, cx);
                if let Some(index) = selects.last().copied() {
                    changed |= view.on_scrub_selected(index);
                }
                if let Some(ratio) = jumps.last().copied() {
                    view.apply_jump(ratio);
                    changed = true;
                }
                if view.poll_scrubber(cx) {
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn spawn_backstop(&self, cx: &mut Context<Self>) {
        let git_tx = self.git_tx.clone();
        let generation = self.generation.clone();
        cx.spawn(async move |_view, cx| loop {
            cx.background_executor().timer(REFRESH_INTERVAL).await;
            pipeline::send_refresh(&git_tx, &generation);
        })
        .detach();
    }

    fn spawn_heat_tick(&self, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        cx.spawn(async move |_view, cx| loop {
            cx.background_executor().timer(HEAT_TICK).await;
            let _ = this.update(cx, |view, cx| {
                if view.tick_heat() {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn tick_heat(&mut self) -> bool {
        let Some(age) = self.heat_age() else {
            if self.heat_fingerprint.is_empty() {
                return false;
            }
            self.heat_fingerprint.clear();
            return true;
        };
        let fingerprint: Vec<LineStyle> = self
            .lines
            .iter()
            .map(|line| {
                let fresh = surface::style_from_line(line);
                LineStyle {
                    background_heat: qol_diff::decayed_heat(fresh.background_heat, age),
                    dimmed: fresh.dimmed,
                }
            })
            .collect();
        if fingerprint != self.heat_fingerprint {
            self.heat_fingerprint = fingerprint;
            qol_runtime::probe!("DIFF_VIEWER", "heat tick age={}s", age.as_secs());
            return true;
        }
        self.heat_fingerprint
            .iter()
            .any(|style| style.background_heat != HeatLevel::Cool)
    }

    fn heat_age(&self) -> Option<Duration> {
        let stamp = self.heat_stamp?;
        if !matches!(self.pane, DiffPane::Ready(_)) {
            return None;
        }
        Some(stamp.elapsed())
    }

    fn apply_results(
        &mut self,
        results: Vec<GitResult>,
        current: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut changed = false;
        for result in results {
            match result {
                GitResult::Facts { generation, facts } if generation == current => {
                    changed |= self.apply_facts(facts, cx);
                }
                GitResult::FactsFailed {
                    generation,
                    message,
                } if generation == current => {
                    self.facts_error = Some(message);
                    changed = true;
                }
                GitResult::Diff {
                    generation: _,
                    path,
                    range,
                    diff,
                    touched_at,
                } if self.matches_request(&path, &range) => {
                    let diff = diff.map(Arc::new);
                    if let Ok(diff) = &diff {
                        self.diff_cache
                            .insert((path.clone(), range.clone()), Arc::clone(diff));
                    }
                    if range == pipeline::DEFAULT_RANGE {
                        let stamp = touched_at.unwrap_or_else(Instant::now);
                        self.heat_stamps.insert(path.clone(), stamp);
                        self.heat_stamp = Some(stamp);
                    } else {
                        self.heat_stamp = None;
                    }
                    changed |= self.apply_diff(diff, &range);
                    self.in_flight = None;
                }
                GitResult::History {
                    generation,
                    commits,
                    magnitudes,
                } if generation == current => {
                    let magnitudes_by_sha: HashMap<&str, u64> = magnitudes
                        .iter()
                        .map(|entry| (entry.sha.as_str(), entry.magnitude()))
                        .collect();
                    let commits: Vec<ScrubCommit> = commits
                        .into_iter()
                        .map(|entry| {
                            let magnitude = magnitudes_by_sha
                                .get(entry.sha.as_str())
                                .copied()
                                .unwrap_or(0);
                            ScrubCommit::with_magnitude(entry.sha, entry.subject, magnitude)
                        })
                        .collect();
                    if commits != self.scrub_commits {
                        self.diff_cache.clear();
                    }
                    self.scrub_commits = commits;
                    self.pending_cone = None;
                    self.cone = None;
                    let commits = self.scrub_commits.clone();
                    self.scrubber_view.update(cx, |view, cx| {
                        view.set_commits(commits, cx);
                    });
                    self.last_scrub_selected = None;
                    changed = true;
                }
                GitResult::Facts { .. }
                | GitResult::FactsFailed { .. }
                | GitResult::Diff { .. }
                | GitResult::History { .. } => {}
            }
        }
        changed
    }

    fn matches_request(&self, path: &str, range: &str) -> bool {
        self.requested
            .as_ref()
            .is_some_and(|(wanted_path, wanted_range)| wanted_path == path && wanted_range == range)
    }

    fn request_diff(&mut self, path: String, range: String) {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|(in_path, in_range)| *in_path == path && *in_range == range)
        {
            return;
        }
        self.in_flight = Some((path.clone(), range.clone()));
        self.requested = Some((path.clone(), range.clone()));
        let _ = self.git_tx.send(GitRequest::SelectFile {
            generation: self.generation.load(Ordering::SeqCst),
            path,
            range,
        });
    }

    fn request_diff_cached(&mut self, path: String, range: String) {
        if let Some(diff) = self.diff_cache.get(&(path.clone(), range.clone())).cloned() {
            self.heat_stamp = if range == pipeline::DEFAULT_RANGE {
                self.heat_stamps.get(&path).copied()
            } else {
                None
            };
            self.apply_diff(Ok(diff), &range);
            return;
        }
        self.request_diff(path, range);
    }

    fn apply_jump(&mut self, ratio: f32) {
        let total = self.row_count().max(1);
        let target = (ratio * total as f32) as usize;
        let max = self.row_count().saturating_sub(self.last_fit.max(1));
        self.surface.set_scroll_offset(target.min(max));
    }

    fn poll_scrubber(&mut self, cx: &mut Context<Self>) -> bool {
        if self.scrub_commits.is_empty() {
            return false;
        }
        let selected = self.scrubber_view.read(cx).state().selected();
        if self.last_scrub_selected == Some(selected) {
            return false;
        }
        self.arm_scrub_trail(selected);
        true
    }

    fn on_scrub_selected(&mut self, index: usize) -> bool {
        if self.last_scrub_selected == Some(index) {
            return false;
        }
        self.arm_scrub_trail(index);
        true
    }

    fn arm_scrub_trail(&mut self, index: usize) {
        let old = self.last_scrub_selected;
        self.last_scrub_selected = Some(index);
        self.pending_center = Some(index);
        self.scrub_anchor = self.surface.scroll_offset();
        if let Some(old) = old {
            self.pending_cone = Some((old, index));
        }
        self.request_scrub_diff(index);
    }

    fn request_scrub_diff(&mut self, index: usize) {
        let Some(path) = self.files.selected_path() else {
            return;
        };
        self.request_diff_cached(path.to_owned(), commit_range(index));
    }

    fn apply_facts(&mut self, facts: Facts, cx: &mut Context<Self>) -> bool {
        qol_runtime::probe!("DIFF_VIEWER", "facts numstat={}", facts.numstat.len());
        self.facts_error = None;
        let facts_equal = self
            .last_facts
            .as_ref()
            .is_some_and(|last| last.numstat == facts.numstat);
        let mut changed = false;
        if !facts_equal {
            changed = true;
            self.last_facts = Some(facts.clone());
            let files = facts
                .numstat
                .iter()
                .map(|entry| FileRow::new(entry.path.clone(), entry.added, entry.deleted))
                .collect();
            self.files.set_files(files);
            if self.files.files.is_empty() {
                self.pane = DiffPane::Empty;
                return true;
            }
        }
        let Some(path) = self.files.selected_path().map(str::to_owned) else {
            return changed;
        };
        if facts
            .changed
            .iter()
            .any(|changed_path| *changed_path == path)
        {
            self.heat_stamps.insert(path.clone(), Instant::now());
        }
        let is_worktree_range = self
            .requested
            .as_ref()
            .is_none_or(|(_, range)| range == pipeline::DEFAULT_RANGE);
        let refetch = is_worktree_range
            && (facts
                .changed
                .iter()
                .any(|changed_path| *changed_path == path)
                || (facts.changed.is_empty() && !facts_equal));
        if refetch {
            if self.files.files.iter().any(|file| file.path == path) {
                let range = self
                    .requested
                    .as_ref()
                    .map(|(_, range)| range.clone())
                    .unwrap_or_else(|| pipeline::DEFAULT_RANGE.to_string());
                self.request_diff(path, range);
            } else {
                self.select_current_file(cx);
            }
            changed = true;
        }
        changed
    }

    fn apply_diff(&mut self, diff: Result<Arc<FileDiff>, DiffError>, range: &str) -> bool {
        let centered = if let Some(index) = self.pending_center {
            if range == commit_range(index) {
                self.pending_center = None;
                self.center_ribbon_on(diff.as_ref().ok().map(Arc::as_ref), self.scrub_anchor)
            } else {
                false
            }
        } else {
            false
        };
        match diff {
            Ok(diff) if diff.is_empty() => {
                if matches!(self.pane, DiffPane::Empty) {
                    return centered;
                }
                self.begin_transition(&[]);
                self.lines.clear();
                self.constructs.clear();
                self.construct_arms.clear();
                self.line_text.clear();
                self.line_highlights.clear();
                self.unified_gutters.clear();
                self.side_gutters.clear();
                self.surface.set_lines(&[]);
                self.pane = DiffPane::Empty;
            }
            Ok(diff) => {
                if let DiffPane::Ready(previous) = &self.pane {
                    if Arc::ptr_eq(previous, &diff) || **previous == *diff {
                        return centered;
                    }
                }
                let lines = flatten(&diff);
                self.begin_transition(&lines);
                let (pairs, hunk_pair_starts, hunk_line_starts) = build_pairs(&diff.hunks);
                let gutter_width = self.surface.gutter_width();
                self.line_text = lines
                    .iter()
                    .map(|line| SharedString::from(line.text.clone()))
                    .collect();
                self.line_highlights = lines
                    .iter()
                    .map(|line| highlight_ranges(line, None))
                    .collect();
                self.unified_gutters = lines
                    .iter()
                    .map(|line| unified_gutter(line, gutter_width))
                    .collect();
                self.side_gutters = lines
                    .iter()
                    .map(|line| side_gutter_labels(line, gutter_width))
                    .collect();
                self.surface.set_lines(&lines);
                self.lines = lines;
                let lang = self.file_lang();
                self.constructs = constructs::detect_constructs(&self.lines, lang);
                self.construct_arms = self
                    .constructs
                    .iter()
                    .map(|construct| constructs::branch_arms(&self.lines, construct, lang))
                    .collect();
                self.pairs = pairs;
                self.hunk_pair_starts = hunk_pair_starts;
                self.hunk_line_starts = hunk_line_starts;
                self.pane = DiffPane::Ready(diff);
            }
            Err(error) => {
                self.transition = None;
                self.constructs.clear();
                self.construct_arms.clear();
                self.pane = DiffPane::Error(error);
            }
        }
        true
    }

    fn center_ribbon_on(&mut self, diff: Option<&FileDiff>, anchor: usize) -> bool {
        let Some(diff) = diff else {
            return false;
        };
        let total: usize = diff.hunks.iter().map(|hunk| hunk.lines.len()).sum();
        let fit = match self.layout {
            Layout::Field if !self.field_folded => self.field_fit.max(1),
            _ => self.last_fit.max(1),
        };
        let target = ribbon_center_target(first_changed_line(diff), total, fit, anchor);
        if target != self.surface.scroll_offset() {
            self.surface.set_scroll_offset(target);
            return true;
        }
        false
    }

    fn begin_transition(&mut self, new_lines: &[LineChange]) {
        self.transition = None;
        if matches!(self.pane, DiffPane::Error(_)) {
            return;
        }
        self.cone = self.pending_cone.take().map(|(from, to)| ConeSpec {
            seq: self.transition_seq + 1,
            from,
            to,
        });
        let old_lines = self.lines.clone();
        let plan = TransitionPlan::between(&old_lines, new_lines);
        let old_pair_of = old_pair_rows(&self.pairs, old_lines.len());
        let token_paths = new_lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let old_text = match plan.new.get(index) {
                    Some(NewLineFate::CarriedFrom(old_index))
                    | Some(NewLineFate::MorphedFrom(old_index)) => {
                        old_lines[*old_index].text.as_str()
                    }
                    _ => "",
                };
                token_edit_path(old_text, &line.text)
            })
            .collect();
        let lang = self.file_lang();
        let old_constructs = constructs::detect_constructs(&old_lines, lang);
        let old_arms = old_constructs
            .iter()
            .map(|construct| constructs::branch_arms(&old_lines, construct, lang))
            .collect();
        self.transition_seq += 1;
        self.transition = Some(TransitionState {
            seq: self.transition_seq,
            started: Instant::now(),
            old_lines,
            old_pair_of,
            old_layout: self.layout,
            old_scroll: self.surface.scroll_offset(),
            old_gutter_width: self.surface.gutter_width(),
            plan,
            token_paths,
            old_constructs,
            old_arms,
        });
    }

    fn hide_panel(&mut self, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn select_current_file(&mut self, cx: &mut Context<Self>) {
        self.pending_cone = None;
        let Some(path) = self.files.selected_path() else {
            return;
        };
        let index = self.scrubber_view.read(cx).state().selected();
        self.pending_center = Some(index);
        self.scrub_anchor = self.surface.scroll_offset();
        self.request_diff(path.to_owned(), commit_range(index));
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "escape" | "esc" => {
                self.hide_panel(cx);
                return;
            }
            "left" | "right" => {
                if self.scrub_commits.is_empty() {
                    return;
                }
                if self
                    .scrubber_view
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    return;
                }
                let delta: isize = arrow_delta(key);
                self.scrubber_view.update(cx, |view, cx| {
                    view.step_selection(delta, cx);
                });
            }
            "j" => self.scroll_lines(1),
            "k" => self.scroll_lines(-1),
            "down" => self.files.move_down(),
            "up" => self.files.move_up(),
            "enter" | "return" => self.select_current_file(cx),
            "f" => self.files_collapsed = !self.files_collapsed,
            "t" => {
                self.field_folded = !self.field_folded;
                let lines = self.lines.clone();
                self.begin_transition(&lines);
            }
            _ => return,
        }
        cx.notify();
    }

    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let notches = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y.to_f64() as f32 / PIXELS_PER_NOTCH,
        };
        if notches == 0.0 {
            return;
        }
        self.scroll_lines(-(notches.round() as isize));
        cx.notify();
    }

    fn scroll_lines(&mut self, delta: isize) {
        if self.layout == Layout::Wave && !self.wave_folded {
            let max = self.row_count().saturating_sub(1);
            let target = self.surface.scroll_offset() as isize + delta;
            self.surface
                .set_scroll_offset(target.clamp(0, max as isize) as usize);
            return;
        }
        let fit = match self.layout {
            Layout::Field if !self.field_folded => self.field_fit.max(1),
            _ => self.last_fit.max(1),
        };
        let max = self.row_count().saturating_sub(fit);
        let target = self.surface.scroll_offset() as isize + delta;
        self.surface
            .set_scroll_offset(target.clamp(0, max as isize) as usize);
    }

    fn row_count(&self) -> usize {
        match self.layout {
            Layout::Unified => self.lines.len(),
            Layout::Split => self.pairs.len(),
            Layout::Wave => self.lines.len(),
            Layout::Field => self.lines.len(),
        }
    }

    fn render_file_list(&self, cx: &Context<Self>) -> AnyElement {
        let rows: Vec<AnyElement> = self
            .files
            .visible_range()
            .map(|index| self.file_row(index, cx))
            .collect();
        let mut list = div()
            .id("file-list")
            .flex_none()
            .w(px(LIST_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(surface::LIST_BG))
            .border_r_1()
            .border_color(rgb(surface::BORDER))
            .overflow_hidden();
        if let Some(error) = &self.facts_error {
            list = list.child(center_message(error, surface::ERROR_TEXT));
        } else if self.files.files.is_empty() {
            list = list.child(center_message("No changed files", surface::TEXT_MUTED));
        } else {
            list = list.children(rows);
        }
        list.into_any_element()
    }

    fn file_row(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let file = &self.files.files[index];
        let selected = index == self.files.list.selected;
        let mut row = div()
            .id(("file-row", index))
            .cursor(CursorStyle::PointingHand)
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .text_size(px(12.0))
            .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.files.list.selected = index;
                view.files.list.sync(view.files.files.len());
                view.select_current_file(cx);
                cx.notify();
            }));
        if selected {
            row = row.bg(rgb(surface::LIST_SELECTED_BG));
        }
        row.child(
            div()
                .flex_1()
                .truncate()
                .text_color(rgb(if selected {
                    surface::TEXT_SELECTED
                } else {
                    surface::TEXT_PRIMARY
                }))
                .child(file.path.clone()),
        )
        .child(count_cell(file.added_label.as_deref(), surface::TEXT_ADDED))
        .child(count_cell(
            file.deleted_label.as_deref(),
            surface::TEXT_REMOVED,
        ))
        .into_any_element()
    }

    fn render_pane(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pane = div()
            .id("diff-pane")
            .flex_1()
            .h_full()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(Self::on_scroll));
        if let Some(error) = &self.facts_error {
            return pane
                .bg(rgb(surface::CANVAS_BG))
                .child(center_message(error, surface::ERROR_TEXT))
                .into_any_element();
        }
        match &self.pane {
            DiffPane::Empty => {
                let message = match &self.repo {
                    Some(repo) => format!("No changes in {}", repo.display()),
                    None => "No changes".to_string(),
                };
                let text_style = self.base_text_style(window);
                let old_layout = self
                    .transition
                    .as_ref()
                    .map(|transition| transition.old_layout);
                if matches!(self.layout, Layout::Field) && !self.field_folded {
                    let scene = self.field_scene(window, cx, Vec::new());
                    let message = div()
                        .absolute()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_2()
                        .text_color(rgb(surface::TEXT_MUTED))
                        .child(message);
                    return pane
                        .bg(field::background())
                        .child(scene)
                        .child(message)
                        .into_any_element();
                }
                let ghosts = match old_layout {
                    Some(Layout::Split) => self.split_ghost_layer(&text_style, None),
                    _ => div()
                        .absolute()
                        .size_full()
                        .top(px(0.0))
                        .children(self.unified_ghosts(&text_style, None))
                        .into_any_element(),
                };
                pane.bg(rgb(surface::CANVAS_BG))
                    .child(center_message(&message, surface::TEXT_MUTED))
                    .child(ghosts)
                    .into_any_element()
            }
            DiffPane::Error(error) => pane
                .bg(rgb(surface::CANVAS_BG))
                .child(center_message(&error.to_string(), surface::ERROR_TEXT))
                .into_any_element(),
            DiffPane::Ready(_) => {
                let text_style = self.base_text_style(window);
                let age = self.heat_age();
                match self.layout {
                    Layout::Split => pane
                        .bg(rgb(surface::CANVAS_BG))
                        .child(self.split_columns(&text_style, age))
                        .into_any_element(),
                    Layout::Unified => pane
                        .bg(rgb(surface::CANVAS_BG))
                        .children(self.code_rows(&text_style, age))
                        .children(self.unified_ghosts(&text_style, age))
                        .into_any_element(),
                    Layout::Wave if self.wave_folded => pane
                        .bg(rgb(surface::CANVAS_BG))
                        .children(self.code_rows(&text_style, age))
                        .children(self.unified_ghosts(&text_style, age))
                        .into_any_element(),
                    Layout::Wave => pane
                        .bg(rgb(surface::CANVAS_BG))
                        .child(self.wave_pane(age))
                        .into_any_element(),
                    Layout::Field if self.field_folded => pane
                        .bg(field::background())
                        .children(self.code_rows(&text_style, age))
                        .children(self.unified_ghosts(&text_style, age))
                        .into_any_element(),
                    Layout::Field => pane
                        .bg(field::background())
                        .child(self.field_pane(window, cx))
                        .into_any_element(),
                }
            }
        }
    }

    fn field_pane(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let text_style = self.base_text_style(window);
        let age = self.heat_age();
        let rows = self.field_rows(&text_style, age);
        self.field_scene(window, cx, rows)
    }

    fn field_scene(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        rows: Vec<AnyElement>,
    ) -> AnyElement {
        let age = self.heat_age();
        let selected = self.scrubber_view.read(cx).state().selected();
        let phase = age
            .map(|age| age.as_secs_f32())
            .unwrap_or_else(|| self.field_epoch.elapsed().as_secs_f32());
        let pane_height = field::pane_height(window.viewport_size().height.to_f64() as f32);
        let ash = self.active_transition().and_then(|transition| {
            let removed: Vec<f32> = transition
                .plan
                .old
                .iter()
                .enumerate()
                .filter(|(_, fate)| **fate == OldLineFate::Removed)
                .map(|(index, _)| (index as f32 - transition.old_scroll as f32) * LINE_HEIGHT)
                .collect();
            (!removed.is_empty()).then_some(AshSpec {
                seq: transition.seq,
                removed_rows: removed,
            })
        });
        let cone = self.active_transition().and_then(|transition| {
            self.cone
                .as_ref()
                .filter(|cone| cone.seq == transition.seq)
                .map(|cone| ConeSpec {
                    seq: cone.seq,
                    from: cone.from,
                    to: cone.to,
                })
        });
        field::scene(
            rows,
            &self.scrub_commits,
            selected,
            SceneState {
                pane_height,
                phase_seconds: phase,
                ash,
                cone,
                terrain: self.terrain_spec().map(Rc::new),
                bloom_rank: self.visible_bloom_rank(),
            },
        )
    }

    fn file_lang(&self) -> Lang {
        self.files
            .selected_path()
            .map(Lang::from_path)
            .unwrap_or(Lang::Generic)
    }

    fn terrain_spec(&self) -> Option<TerrainSpec> {
        if self.lines.is_empty() {
            return None;
        }
        let rows = self.lines.len();
        let age = self.heat_age();
        let transition = self.active_transition();
        let old_rows = transition.map(|transition| transition.old_lines.len());
        let mut marks = Vec::new();
        for (index, construct) in self.constructs.iter().enumerate() {
            let old = transition
                .and_then(|transition| transition.construct_partner(construct))
                .map(|(_, old_start, old_end, old_arms)| TerrainGeo {
                    start: old_start,
                    end: old_end,
                    arms: old_arms,
                    rows: old_rows.unwrap_or(0),
                });
            marks.push(TerrainMark {
                kind: construct.kind,
                start: construct.start_line,
                end: construct.end_line,
                depth: construct.depth,
                arms: self.construct_arms[index],
                heat: self.construct_heat(&self.lines, construct, age),
                old,
            });
        }
        let mut deaths = Vec::new();
        if let Some(transition) = transition {
            for index in transition.death_indices(&self.constructs) {
                let construct = &transition.old_constructs[index];
                deaths.push(TerrainDeath {
                    kind: construct.kind,
                    start: construct.start_line,
                    end: construct.end_line,
                    depth: construct.depth,
                    arms: transition.old_arms[index],
                    heat: self.construct_heat(&transition.old_lines, construct, None),
                    rows: transition.old_lines.len(),
                });
            }
        }
        if marks.is_empty() && deaths.is_empty() {
            return None;
        }
        Some(TerrainSpec {
            seq: transition.map(|transition| transition.seq).unwrap_or(0),
            morphing: transition.is_some(),
            rows,
            marks,
            deaths,
        })
    }

    fn construct_heat(
        &self,
        lines: &[LineChange],
        construct: &Construct,
        age: Option<Duration>,
    ) -> HeatLevel {
        if lines.is_empty() {
            return HeatLevel::Cool;
        }
        let start = construct.start_line.min(lines.len() - 1);
        let end = construct.end_line.min(lines.len() - 1);
        let mut rank = 0u8;
        for line in lines.iter().take(end + 1).skip(start) {
            let fresh = surface::style_from_line(line);
            rank = rank.max(heat_rank(decayed_line_style(fresh, age).background_heat));
        }
        match rank {
            0 => HeatLevel::Cool,
            1 => HeatLevel::Warm,
            _ => HeatLevel::Hot,
        }
    }

    fn visible_bloom_rank(&self) -> u8 {
        let age = self.heat_age();
        let start = self.surface.scroll_offset();
        let end = (start + self.field_fit.max(1)).min(self.lines.len());
        let mut rank = 0u8;
        for index in start..end {
            let fresh = surface::style_from_line(&self.lines[index]);
            rank = rank.max(heat_rank(decayed_line_style(fresh, age).background_heat));
        }
        rank
    }

    fn field_alive(&self) -> bool {
        if self.active_transition().is_some() {
            return true;
        }
        if !matches!(self.pane, DiffPane::Ready(_)) {
            return false;
        }
        let age = self.heat_age();
        let start = self.surface.scroll_offset();
        let end = (start + self.field_fit.max(1)).min(self.lines.len());
        (start..end).any(|index| {
            let fresh = surface::style_from_line(&self.lines[index]);
            decayed_line_style(fresh, age).background_heat != HeatLevel::Cool
        })
    }

    fn field_rows(&self, base: &TextStyle, age: Option<Duration>) -> Vec<AnyElement> {
        let start = self.surface.scroll_offset();
        let end = (start + self.field_fit.max(1)).min(self.lines.len());
        (start..end)
            .map(|index| self.code_row(index, base, age))
            .collect()
    }

    fn wave_pane(&self, age: Option<Duration>) -> AnyElement {
        let heats: Vec<HeatLevel> = self
            .lines
            .iter()
            .map(|line| decayed_line_style(surface::style_from_line(line), age).background_heat)
            .collect();
        let points = Rc::new(waveform(&self.lines, &heats));
        let playhead = self
            .surface
            .scroll_offset()
            .min(self.lines.len().saturating_sub(1));
        match self.active_transition() {
            Some(transition) => {
                let old_heats: Vec<HeatLevel> = transition
                    .old_lines
                    .iter()
                    .map(|line| surface::style_from_line(line).background_heat)
                    .collect();
                let old_points = Rc::new(waveform(&transition.old_lines, &old_heats));
                let morph = Rc::new(WaveMorph {
                    old_points,
                    plan: Rc::new(transition.plan.clone()),
                });
                let seq = transition.seq;
                wave::wave_element(points.clone(), Some(Rc::clone(&morph)), playhead, 1.0)
                    .with_animation(
                        ElementId::named_usize(format!("dw-wave-{seq}"), 0),
                        Animation::new(TRANSITION_MS).with_easing(ease_out_quint()),
                        move |_canvas, delta| {
                            wave::wave_element(
                                points.clone(),
                                Some(Rc::clone(&morph)),
                                playhead,
                                delta,
                            )
                        },
                    )
                    .into_any_element()
            }
            None => wave::wave_element(points, None, playhead, 1.0).into_any_element(),
        }
    }

    fn base_text_style(&self, window: &Window) -> TextStyle {
        let mut text_style = window.text_style();
        text_style.font_family = self.font_family.clone();
        text_style.font_size = px(FONT_SIZE).into();
        text_style.line_height = px(LINE_HEIGHT).into();
        text_style.white_space = WhiteSpace::Nowrap;
        text_style
    }

    fn split_columns(&self, base: &TextStyle, age: Option<Duration>) -> AnyElement {
        let start = self.surface.scroll_offset();
        let end = (start + self.last_fit.max(1)).min(self.row_count());
        let mut old_rows = Vec::new();
        let mut new_rows = Vec::new();
        for row in start..end {
            old_rows.push(self.split_row(row, true, base, age));
            new_rows.push(self.split_row(row, false, base, age));
        }
        div()
            .flex()
            .flex_row()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .border_r_1()
                    .border_color(rgb(surface::BORDER))
                    .children(old_rows)
                    .children(self.split_ghosts(true, base, age)),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .children(new_rows)
                    .children(self.split_ghosts(false, base, age)),
            )
            .into_any_element()
    }

    fn split_row(
        &self,
        row: usize,
        is_old: bool,
        base: &TextStyle,
        age: Option<Duration>,
    ) -> AnyElement {
        let pair = self.pairs.get(row).copied().unwrap_or(LinePair {
            old: None,
            new: None,
        });
        let line_index = if is_old { pair.old } else { pair.new };
        let counterpart = if is_old { pair.new } else { pair.old };
        let mut row_el = div()
            .h(px(LINE_HEIGHT))
            .w_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .line_height(px(LINE_HEIGHT))
            .text_size(px(FONT_SIZE));
        let style = decayed_line_style(
            line_index
                .and_then(|index| self.lines.get(index))
                .map(surface::style_from_line)
                .unwrap_or_default(),
            age,
        );
        let blank_bg = counterpart
            .and_then(|index| self.lines.get(index))
            .and_then(|line| {
                let fresh = surface::style_from_line(line);
                pulsed_background(decayed_line_style(fresh, age), age)
            });
        if let Some(bg) = pulsed_background(style, age) {
            row_el = row_el.bg(rgb(bg));
        } else if let Some(bg) = blank_bg {
            row_el = row_el.bg(rgb(bg));
        }
        let width = self.surface.gutter_width();
        match line_index {
            Some(index) => {
                let line = &self.lines[index];
                let label = if is_old {
                    &self.side_gutters[index].0
                } else {
                    &self.side_gutters[index].1
                };
                let glyph = match (is_old, line.kind) {
                    (true, LineKind::Removed) => "-",
                    (false, LineKind::Added) => "+",
                    _ => " ",
                };
                self.animate_new_line(
                    index,
                    row_el
                        .child(
                            div()
                                .flex_none()
                                .px_1()
                                .text_color(rgb(surface::GUTTER_TEXT))
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(GLYPH_WIDTH))
                                .text_color(rgb(surface::kind_color(line.kind)))
                                .child(glyph),
                        )
                        .child(self.code_text(index, style.dimmed, base, age)),
                )
            }
            None => row_el
                .child(
                    div()
                        .flex_none()
                        .px_1()
                        .text_color(rgb(surface::GUTTER_TEXT))
                        .child(" ".repeat(width)),
                )
                .child(div().flex_none().w(px(GLYPH_WIDTH)).child(" "))
                .child(div().flex_1().child(" "))
                .into_any_element(),
        }
    }

    fn code_rows(&self, base: &TextStyle, age: Option<Duration>) -> Vec<AnyElement> {
        let start = self.surface.scroll_offset();
        let end = (start + self.last_fit.max(1)).min(self.lines.len());
        (start..end)
            .map(|index| self.code_row(index, base, age))
            .collect()
    }

    fn code_row(&self, index: usize, base: &TextStyle, age: Option<Duration>) -> AnyElement {
        let line = &self.lines[index];
        let style = decayed_line_style(self.surface.line_style(index), age);
        let mut row = div()
            .h(px(LINE_HEIGHT))
            .w_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .line_height(px(LINE_HEIGHT))
            .text_size(px(FONT_SIZE));
        if let Some(bg) = pulsed_background(style, age) {
            row = row.bg(rgb(bg));
        }
        let body = if self.active_transition().is_some() {
            self.token_line(index, style.dimmed, age).into_any_element()
        } else {
            self.code_text(index, style.dimmed, base, age)
                .into_any_element()
        };
        row.child(
            div()
                .flex_none()
                .px_1()
                .text_color(rgb(surface::GUTTER_TEXT))
                .child(self.unified_gutters[index].clone()),
        )
        .child(
            div()
                .flex_none()
                .w(px(GLYPH_WIDTH))
                .text_color(rgb(surface::kind_color(line.kind)))
                .child(surface::kind_glyph(line.kind)),
        )
        .child(body)
        .into_any_element()
    }

    fn token_line(&self, line_index: usize, dimmed: bool, age: Option<Duration>) -> Div {
        let transition = self
            .active_transition()
            .expect("token line needs a transition");
        let edits = &transition.token_paths[line_index];
        let context = TokenRowContext {
            seq: transition.seq,
            line_index,
            dimmed,
            age,
            line: &self.lines[line_index],
        };
        let mut row = div()
            .flex_1()
            .flex()
            .flex_row()
            .overflow_hidden()
            .whitespace_nowrap();
        let mut offset = 0usize;
        for (token_index, edit) in edits.iter().enumerate() {
            let start = offset;
            if edit.fate != TokenFate::Evaporated {
                offset += edit.text.len();
            }
            for element in token_edit_elements(edit, start, token_index, &context) {
                row = row.child(element);
            }
        }
        row
    }

    fn code_text(
        &self,
        index: usize,
        dimmed: bool,
        base: &TextStyle,
        age: Option<Duration>,
    ) -> StyledText {
        let mut text_style = base.clone();
        text_style.color = rgb(surface::text_color(dimmed)).into();
        let highlights = match age {
            Some(age) => highlight_ranges(&self.lines[index], Some(age)),
            None => self.line_highlights[index].clone(),
        };
        StyledText::new(self.line_text[index].clone())
            .with_default_highlights(&text_style, highlights)
    }

    fn animate_new_line(&self, line_index: usize, row: Div) -> AnyElement {
        let Some(transition) = self.active_transition() else {
            return row.into_any_element();
        };
        let seq = transition.seq;
        match transition.plan.new.get(line_index) {
            Some(NewLineFate::Added) => row
                .with_animation(
                    ElementId::named_usize(format!("tw-row-{seq}"), line_index),
                    Animation::new(TRANSITION_MS).with_easing(ease_out_quint()),
                    |row, progress| row.opacity(progress).top(px(SLIDE_PX * (1.0 - progress))),
                )
                .into_any_element(),
            Some(NewLineFate::MorphedFrom(_)) => row
                .with_animation(
                    ElementId::named_usize(format!("tw-row-{seq}"), line_index),
                    Animation::new(TRANSITION_MS).with_easing(ease_out_quint()),
                    |row, progress| row.opacity(MORPH_FADE + (1.0 - MORPH_FADE) * progress),
                )
                .into_any_element(),
            _ => row.into_any_element(),
        }
    }

    fn active_transition(&self) -> Option<&TransitionState> {
        self.transition
            .as_ref()
            .filter(|transition| transition.started.elapsed() < TRANSITION_MS)
    }

    fn unified_ghosts(&self, base: &TextStyle, age: Option<Duration>) -> Vec<AnyElement> {
        let Some(transition) = self.active_transition() else {
            return Vec::new();
        };
        let mut ghosts = Vec::new();
        for (index, fate) in transition.plan.old.iter().enumerate() {
            if *fate != OldLineFate::Removed {
                continue;
            }
            let top = (index as f32 - transition.old_scroll as f32) * LINE_HEIGHT;
            ghosts.push(self.unified_ghost(index, top, base, age));
        }
        ghosts
    }

    fn split_ghosts(
        &self,
        is_old_side: bool,
        base: &TextStyle,
        age: Option<Duration>,
    ) -> Vec<AnyElement> {
        let Some(transition) = self.active_transition() else {
            return Vec::new();
        };
        let mut ghosts = Vec::new();
        for (index, fate) in transition.plan.old.iter().enumerate() {
            if *fate != OldLineFate::Removed {
                continue;
            }
            let line = &transition.old_lines[index];
            if (line.kind == LineKind::Added) == is_old_side {
                continue;
            }
            let pair_row = transition.old_pair_of[index].unwrap_or(index);
            let top = (pair_row as f32 - transition.old_scroll as f32) * LINE_HEIGHT;
            ghosts.push(self.split_ghost(index, is_old_side, top, base, age));
        }
        ghosts
    }

    fn split_ghost_layer(&self, base: &TextStyle, age: Option<Duration>) -> AnyElement {
        div()
            .absolute()
            .size_full()
            .top(px(0.0))
            .flex()
            .flex_row()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .border_r_1()
                    .border_color(rgb(surface::BORDER))
                    .children(self.split_ghosts(true, base, age)),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .children(self.split_ghosts(false, base, age)),
            )
            .into_any_element()
    }

    fn unified_ghost(
        &self,
        index: usize,
        top: f32,
        base: &TextStyle,
        age: Option<Duration>,
    ) -> AnyElement {
        let transition = self
            .active_transition()
            .expect("unified ghost needs a transition");
        let line = &transition.old_lines[index];
        let seq = transition.seq;
        let width = transition.old_gutter_width;
        let style = decayed_line_style(surface::style_from_line(line), age);
        let mut row = ghost_base(top);
        if let Some(bg) = pulsed_background(style, age) {
            row = row.bg(rgb(bg));
        }
        row.child(
            div()
                .flex_none()
                .px_1()
                .text_color(rgb(surface::GUTTER_TEXT))
                .child(unified_gutter(line, width)),
        )
        .child(
            div()
                .flex_none()
                .w(px(GLYPH_WIDTH))
                .text_color(rgb(surface::kind_color(line.kind)))
                .child(surface::kind_glyph(line.kind)),
        )
        .child(ghost_text(line, style.dimmed, base, age))
        .with_animation(
            ElementId::named_usize(format!("tw-ghost-{seq}"), index),
            Animation::new(TRANSITION_MS).with_easing(ease_out_quint()),
            move |row, progress| {
                row.opacity(1.0 - progress)
                    .top(px(top - GHOST_DRIFT_PX * progress))
            },
        )
        .into_any_element()
    }

    fn split_ghost(
        &self,
        index: usize,
        is_old_side: bool,
        top: f32,
        base: &TextStyle,
        age: Option<Duration>,
    ) -> AnyElement {
        let transition = self
            .active_transition()
            .expect("split ghost needs a transition");
        let line = &transition.old_lines[index];
        let seq = transition.seq;
        let width = transition.old_gutter_width;
        let labels = surface::gutter_labels(line.old_line_no, line.new_line_no, width);
        let label = if is_old_side { labels.0 } else { labels.1 };
        let style = decayed_line_style(surface::style_from_line(line), age);
        let mut row = ghost_base(top);
        if let Some(bg) = pulsed_background(style, age) {
            row = row.bg(rgb(bg));
        }
        let glyph = match (is_old_side, line.kind) {
            (true, LineKind::Removed) => "-",
            (false, LineKind::Added) => "+",
            _ => " ",
        };
        row.child(
            div()
                .flex_none()
                .px_1()
                .text_color(rgb(surface::GUTTER_TEXT))
                .child(label),
        )
        .child(
            div()
                .flex_none()
                .w(px(GLYPH_WIDTH))
                .text_color(rgb(surface::kind_color(line.kind)))
                .child(glyph),
        )
        .child(ghost_text(line, style.dimmed, base, age))
        .with_animation(
            ElementId::named_usize(format!("tw-ghost-{seq}"), index),
            Animation::new(TRANSITION_MS).with_easing(ease_out_quint()),
            move |row, progress| {
                row.opacity(1.0 - progress)
                    .top(px(top - GHOST_DRIFT_PX * progress))
            },
        )
        .into_any_element()
    }
}

fn heat_rank(heat: HeatLevel) -> u8 {
    match heat {
        HeatLevel::Cool => 0,
        HeatLevel::Warm => 1,
        HeatLevel::Hot => 2,
    }
}

impl TransitionState {
    fn construct_partner(&self, construct: &Construct) -> Option<(usize, usize, usize, usize)> {
        let old_at = |line: usize| match self.plan.new.get(line) {
            Some(NewLineFate::CarriedFrom(old)) | Some(NewLineFate::MorphedFrom(old)) => Some(*old),
            _ => None,
        };
        let (keyword, closing) = construct.anchor;
        let old_keyword = old_at(keyword)?;
        let old_closing = old_at(closing)?;
        self.old_constructs
            .iter()
            .enumerate()
            .find(|(_, old)| old.kind == construct.kind && old.anchor == (old_keyword, old_closing))
            .map(|(index, old)| (index, old.start_line, old.end_line, self.old_arms[index]))
    }

    fn death_indices(&self, new_constructs: &[Construct]) -> Vec<usize> {
        let mut claimed = vec![false; self.old_constructs.len()];
        for construct in new_constructs {
            if let Some((index, _, _, _)) = self.construct_partner(construct) {
                if let Some(slot) = claimed.get_mut(index) {
                    *slot = true;
                }
            }
        }
        (0..self.old_constructs.len())
            .filter(|index| !claimed[*index])
            .collect()
    }
}

fn token_edit_elements(
    edit: &TokenEdit,
    line_offset: usize,
    token_index: usize,
    context: &TokenRowContext,
) -> Vec<AnyElement> {
    let id = ElementId::named_usize(
        format!("tw-tok-{}-{}", context.seq, context.line_index),
        token_index,
    );
    let animation = Animation::new(TRANSITION_MS).with_easing(ease_out_quint());
    let (span_bg, span_color) =
        token_span_style(context.line, line_offset, edit.text.len(), context.age);
    let color = span_color.unwrap_or_else(|| surface::text_color(context.dimmed));
    match edit.fate {
        TokenFate::Kept => {
            let mut element = div().flex_none().whitespace_nowrap().text_color(rgb(color));
            if let Some(bg) = span_bg {
                element = element.bg(rgb(bg));
            }
            vec![element.child(edit.text.clone()).into_any_element()]
        }
        TokenFate::Ignited => {
            let text = SharedString::from(edit.text.clone());
            vec![div()
                .flex_none()
                .whitespace_nowrap()
                .with_animation(id, animation, move |element, progress| {
                    let t = linear_time(progress);
                    let flash = (1.0 - t / FLASH_SPAN).clamp(0.0, 1.0);
                    element
                        .bg(rgba(flash_hex(span_bg, flash)))
                        .shadow(vec![
                            BoxShadow {
                                color: rgba(
                                    (surface::EMBER_RIM << 8) | ((flash * 255.0).round() as u32),
                                )
                                .into(),
                                offset: point(px(0.0), px(0.0)),
                                blur_radius: px(1.0),
                                spread_radius: px(1.0),
                            },
                            BoxShadow {
                                color: rgba(
                                    (surface::EMBER_CORE << 8) | ((flash * 217.0).round() as u32),
                                )
                                .into(),
                                offset: point(px(0.0), px(0.0)),
                                blur_radius: px(4.0),
                                spread_radius: px(2.0),
                            },
                        ])
                        .text_color(rgb(mix_hex(color, surface::EMBER_RIM, flash)))
                        .child(div().opacity(1.0 - flash).child(text.clone()))
                })
                .into_any_element()]
        }
        TokenFate::Evaporated => {
            let text = SharedString::from(edit.text.clone());
            let start_color = span_color.unwrap_or(surface::TEXT_PRIMARY);
            vec![div()
                .flex_none()
                .relative()
                .whitespace_nowrap()
                .with_animation(id, animation, move |element, progress| {
                    let t = linear_time(progress);
                    element
                        .text_size(px(FONT_SIZE * (1.0 - EVAP_SHRINK * t)))
                        .top(px(-EVAP_LIFT_PX * t))
                        .opacity(1.0 - t)
                        .text_color(rgb(mix_hex(start_color, surface::EVAP_GHOST, t)))
                        .child(text.clone())
                })
                .into_any_element()]
        }
        TokenFate::Morphed => {
            let (start, len) = edit.changed_range.unwrap_or((0, edit.text.len()));
            let prefix = SharedString::from(edit.text[..start].to_string());
            let changed = SharedString::from(edit.text[start..start + len].to_string());
            let suffix = SharedString::from(edit.text[start + len..].to_string());
            let mut elements = Vec::new();
            if !prefix.is_empty() {
                elements.push(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .child(prefix)
                        .into_any_element(),
                );
            }
            elements.push(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .with_animation(id, animation, move |element, progress| {
                        let t = linear_time(progress);
                        let bump = 4.0 * t * (1.0 - t);
                        let settle = MORPH_FADE + (1.0 - MORPH_FADE) * t;
                        element
                            .bg(rgba(flare_hex(span_bg, bump)))
                            .shadow(vec![BoxShadow {
                                color: rgba(
                                    (surface::EMBER_CORE << 8) | ((bump * 200.0).round() as u32),
                                )
                                .into(),
                                offset: point(px(0.0), px(0.0)),
                                blur_radius: px(2.0),
                                spread_radius: px(1.0),
                            }])
                            .child(
                                div()
                                    .opacity(settle)
                                    .text_color(rgba(
                                        (mix_hex(color, surface::EMBER_RIM, 1.0 - t) << 8) | 0xff,
                                    ))
                                    .child(changed.clone()),
                            )
                    })
                    .into_any_element(),
            );
            if !suffix.is_empty() {
                elements.push(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .child(suffix)
                        .into_any_element(),
                );
            }
            elements
        }
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self
            .transition
            .as_ref()
            .is_some_and(|transition| transition.started.elapsed() >= TRANSITION_MS)
        {
            self.transition = None;
        }
        let viewport_height = window.viewport_size().height.to_f64() as f32;
        self.last_fit = visible_fit(viewport_height, LINE_HEIGHT);
        self.field_fit = field::ribbon_fit(field::pane_height(viewport_height)).min(self.last_fit);
        if matches!(self.layout, Layout::Field) && !self.field_folded && self.field_alive() {
            window.request_animation_frame();
        }
        self.update_overview(cx);
        if !self.rendered_once {
            self.rendered_once = true;
            qol_runtime::probe!(
                "DIFF_VIEWER",
                "rendered=true viewport={:?}x{:?}",
                window.viewport_size().width,
                window.viewport_size().height
            );
        }
        let bar = WindowBar::new("DIFF VIEWER")
            .drag_gesture(self.drag_gesture.clone())
            .on_collapse({
                let this = cx.entity();
                move |window, app| {
                    this.update(app, |view, cx| {
                        if view.chrome.is_collapsed() {
                            view.chrome.expand(window, cx);
                        } else {
                            view.chrome.collapse(window);
                        }
                        cx.notify();
                    });
                }
            })
            .on_hide({
                let this = cx.entity();
                move |_window, app| {
                    this.update(app, |view, cx| view.hide_panel(cx));
                }
            });
        if self.chrome.is_collapsed() {
            return bar.into_any_element();
        }
        let scrubber = if self.scrub_commits.is_empty() {
            div().h(px(0.0)).into_any_element()
        } else {
            self.scrubber_view.clone().into_any_element()
        };
        div()
            .id("diff-viewer")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(surface::CANVAS_BG))
            .font_family(self.font_family.clone())
            .text_size(px(FONT_SIZE))
            .on_key_down(cx.listener(Self::on_key))
            .child(bar)
            .child({
                let mut row = div().flex().flex_row().flex_1().h_full();
                if !self.files_collapsed {
                    row = row.child(self.render_file_list(cx));
                }
                row.child(self.render_pane(window, cx))
                    .child(self.overview_view.clone())
            })
            .child(scrubber)
            .into_any_element()
    }
}

impl DiffView {
    fn update_overview(&mut self, cx: &mut Context<Self>) {
        let current = match &self.pane {
            DiffPane::Ready(diff) => Some(Arc::clone(diff)),
            _ => None,
        };
        let diff_changed = match (&current, &self.last_markers_diff) {
            (Some(diff), Some(previous)) => !Arc::ptr_eq(previous, diff),
            (Some(_), None) => true,
            (None, Some(_)) => true,
            (None, None) => false,
        };
        if diff_changed || self.layout != self.last_markers_layout {
            let markers = match &self.pane {
                DiffPane::Ready(diff) => {
                    let starts = match self.layout {
                        Layout::Unified | Layout::Wave | Layout::Field => &self.hunk_line_starts,
                        Layout::Split => &self.hunk_pair_starts,
                    };
                    let total = self.row_count();
                    let mut markers = Vec::new();
                    for (index, hunk) in diff.hunks.iter().enumerate() {
                        let weight = hunk
                            .lines
                            .iter()
                            .filter(|line| line.kind != LineKind::Context)
                            .count() as u32;
                        let start = starts.get(index).copied().unwrap_or(0);
                        markers.push(HunkMarker::new(
                            if total == 0 {
                                0.0
                            } else {
                                start as f32 / total as f32
                            },
                            weight,
                        ));
                    }
                    markers
                }
                _ => Vec::new(),
            };
            self.last_markers_diff = current;
            self.last_markers_layout = self.layout;
            self.overview_view.update(cx, |view, _cx| {
                view.set_markers(markers);
            });
        }
        let total = self.row_count();
        let viewport = if total == 0 {
            (0.0f32, 1.0f32)
        } else {
            let start = self.surface.scroll_offset();
            let end = (start + self.last_fit.max(1)).min(total);
            (start as f32 / total as f32, end as f32 / total as f32)
        };
        self.overview_view.update(cx, |view, _cx| {
            view.set_viewport(viewport);
        });
    }
}

fn first_changed_line(diff: &FileDiff) -> Option<usize> {
    let mut flat = 0usize;
    for hunk in &diff.hunks {
        for line in &hunk.lines {
            if line.kind != LineKind::Context {
                return Some(flat);
            }
            flat += 1;
        }
    }
    None
}

fn ribbon_center_target(
    first_changed: Option<usize>,
    total: usize,
    fit: usize,
    anchor: usize,
) -> usize {
    let max = total.saturating_sub(fit);
    match first_changed {
        Some(first) => first.saturating_sub(fit / 2).min(max),
        None => anchor.min(max),
    }
}

fn flatten(diff: &FileDiff) -> Vec<LineChange> {
    diff.hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter().cloned())
        .collect()
}

fn build_pairs(hunks: &[qol_diff::Hunk]) -> (Vec<LinePair>, Vec<usize>, Vec<usize>) {
    let mut pairs = Vec::new();
    let mut hunk_pair_starts = Vec::new();
    let mut hunk_line_starts = Vec::new();
    let mut flat = 0usize;
    for hunk in hunks {
        hunk_line_starts.push(flat);
        hunk_pair_starts.push(pairs.len());
        let mut cursor = 0usize;
        while cursor < hunk.lines.len() {
            if hunk.lines[cursor].kind == LineKind::Context {
                let index = flat;
                flat += 1;
                cursor += 1;
                pairs.push(LinePair {
                    old: Some(index),
                    new: Some(index),
                });
                continue;
            }
            let mut removed = Vec::new();
            let mut added = Vec::new();
            while cursor < hunk.lines.len() && hunk.lines[cursor].kind != LineKind::Context {
                let index = flat;
                flat += 1;
                match hunk.lines[cursor].kind {
                    LineKind::Removed => removed.push(index),
                    _ => added.push(index),
                }
                cursor += 1;
            }
            let width = removed.len().max(added.len());
            for index in 0..width {
                pairs.push(LinePair {
                    old: removed.get(index).copied(),
                    new: added.get(index).copied(),
                });
            }
        }
    }
    (pairs, hunk_pair_starts, hunk_line_starts)
}

fn old_pair_rows(pairs: &[LinePair], line_count: usize) -> Vec<Option<usize>> {
    let mut rows = vec![None; line_count];
    for (row, pair) in pairs.iter().enumerate() {
        if let Some(index) = pair.old {
            if let Some(slot) = rows.get_mut(index) {
                *slot = Some(row);
            }
        }
        if let Some(index) = pair.new {
            if let Some(slot) = rows.get_mut(index) {
                *slot = Some(row);
            }
        }
    }
    rows
}

fn ghost_base(top: f32) -> Div {
    div()
        .absolute()
        .top(px(top))
        .left(px(0.0))
        .w_full()
        .h(px(LINE_HEIGHT))
        .flex()
        .flex_row()
        .overflow_hidden()
        .line_height(px(LINE_HEIGHT))
        .text_size(px(FONT_SIZE))
}

fn ghost_text(
    line: &LineChange,
    dimmed: bool,
    base: &TextStyle,
    age: Option<Duration>,
) -> StyledText {
    let mut text_style = base.clone();
    text_style.color = rgb(surface::text_color(dimmed)).into();
    let highlights = match age {
        Some(age) => highlight_ranges(line, Some(age)),
        None => highlight_ranges(line, None),
    };
    StyledText::new(SharedString::from(line.text.clone()))
        .with_default_highlights(&text_style, highlights)
}

fn decayed_line_style(fresh: LineStyle, age: Option<Duration>) -> LineStyle {
    match age {
        Some(age) => LineStyle {
            background_heat: qol_diff::decayed_heat(fresh.background_heat, age),
            dimmed: fresh.dimmed,
        },
        None => fresh,
    }
}

fn token_span_style(
    line: &LineChange,
    offset: usize,
    len: usize,
    age: Option<Duration>,
) -> (Option<u32>, Option<u32>) {
    let end = offset + len;
    for span in &line.token_spans {
        if span.start < end && span.start + span.len > offset {
            let heat = age.map_or(span.heat, |age| qol_diff::decayed_heat(span.heat, age));
            return (
                surface::token_background(heat),
                match (heat, span.kind) {
                    (HeatLevel::Cool, TokenKind::Plain) => None,
                    (HeatLevel::Cool, kind) => surface::token_kind_color(kind),
                    _ => None,
                },
            );
        }
    }
    (None, None)
}

fn mix_hex(from: u32, to: u32, amount: f32) -> u32 {
    let channel = |a: u32, b: u32| (a as f32 + (b as f32 - a as f32) * amount).round() as u32;
    (channel(from >> 16 & 0xff, to >> 16 & 0xff) << 16)
        | (channel(from >> 8 & 0xff, to >> 8 & 0xff) << 8)
        | channel(from & 0xff, to & 0xff)
}

fn linear_time(progress: f32) -> f32 {
    1.0 - (1.0 - progress).powf(0.2)
}

fn scale_hex(hex: u32, factor: f32) -> u32 {
    let channel = |shift: u32| {
        ((hex >> shift & 0xff) as f32 * factor)
            .round()
            .clamp(0.0, 255.0) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn pulsed_background(style: LineStyle, age: Option<Duration>) -> Option<u32> {
    let bg = surface::line_background(style)?;
    let phase = age.map_or(0.0, |age| {
        age.as_secs_f32() * std::f32::consts::TAU / PULSE_PERIOD_S
    });
    Some(scale_hex(bg, 1.0 + PULSE_AMPLITUDE * phase.sin()))
}

fn flash_hex(base: Option<u32>, amount: f32) -> u32 {
    match base {
        Some(bg) => (mix_hex(surface::EMBER_CORE, bg, 1.0 - amount) << 8) | 0xff,
        None => (surface::EMBER_CORE << 8) | ((amount * 255.0).round() as u32),
    }
}

fn flare_hex(base: Option<u32>, amount: f32) -> u32 {
    match base {
        Some(bg) => (mix_hex(bg, surface::TOKEN_MORPH_FLARE, amount) << 8) | 0xff,
        None => (surface::TOKEN_MORPH_FLARE << 8) | ((amount * 255.0).round() as u32),
    }
}

fn highlight_ranges(
    line: &LineChange,
    age: Option<Duration>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    line.token_spans
        .iter()
        .filter_map(|span| {
            let heat = age.map_or(span.heat, |age| qol_diff::decayed_heat(span.heat, age));
            let start = span.start.min(line.text.len());
            let end = (span.start + span.len).min(line.text.len());
            (start < end).then_some((
                start..end,
                HighlightStyle {
                    background_color: surface::token_background(heat).map(|hex| rgb(hex).into()),
                    color: match (heat, span.kind) {
                        (HeatLevel::Cool, TokenKind::Plain) => None,
                        (HeatLevel::Cool, kind) => {
                            surface::token_kind_color(kind).map(|hex| rgb(hex).into())
                        }
                        _ => None,
                    },
                    ..HighlightStyle::default()
                },
            ))
        })
        .collect()
}

fn unified_gutter(line: &LineChange, width: usize) -> String {
    let (old, new) = surface::gutter_labels(line.old_line_no, line.new_line_no, width);
    format!("{old}  {new}")
}

fn side_gutter_labels(line: &LineChange, width: usize) -> (String, String) {
    (
        line.old_line_no
            .map(|number| format!("{number:>width$}"))
            .unwrap_or_else(|| " ".repeat(width)),
        line.new_line_no
            .map(|number| format!("{number:>width$}"))
            .unwrap_or_else(|| " ".repeat(width)),
    )
}

fn pick_monospace(cx: &App) -> SharedString {
    const CHAIN: [&str; 4] = [
        "Noto Sans Mono",
        "DejaVu Sans Mono",
        "Liberation Mono",
        ".SystemUIFont",
    ];
    let names = cx.text_system().all_font_names();
    CHAIN
        .iter()
        .find(|candidate| names.iter().any(|name| name.as_str() == **candidate))
        .map(|name| SharedString::from(*name))
        .unwrap_or_else(|| SharedString::from(".SystemUIFont"))
}

fn visible_fit(height: f32, row_height: f32) -> usize {
    (height / row_height).floor().max(1.0) as usize
}

fn list_max_visible() -> usize {
    (WINDOW_HEIGHT / ROW_HEIGHT).floor() as usize
}

fn center_message(label: &str, color: u32) -> AnyElement {
    div()
        .flex_1()
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .p_2()
        .text_color(rgb(color))
        .child(label.to_string())
        .into_any_element()
}

fn count_cell(label: Option<&str>, color: u32) -> AnyElement {
    let (label, color) = label
        .map(|label| (label, color))
        .unwrap_or_else(|| ("bin", surface::TEXT_MUTED));
    div()
        .flex_none()
        .text_color(rgb(color))
        .child(label.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, added: Option<u64>, deleted: Option<u64>) -> FileRow {
        FileRow::new(path.to_string(), added, deleted)
    }

    #[test]
    fn count_labels_format_added_and_deleted() {
        let file = row("a.rs", Some(3), Some(2));
        assert_eq!(file.added_label, Some("+3".to_string()));
        assert_eq!(file.deleted_label, Some("-2".to_string()));
        let binary = row("logo.png", None, None);
        assert_eq!(binary.added_label, None, "binary files carry no counts");
        assert_eq!(binary.deleted_label, None);
    }

    #[test]
    fn file_list_navigation_clamps_at_both_ends() {
        let mut state = FileListState::new(5);
        state.set_files(vec![
            row("a.rs", Some(1), Some(0)),
            row("b.rs", Some(2), Some(1)),
            row("c.rs", Some(3), Some(2)),
        ]);
        state.move_down();
        state.move_down();
        state.move_down();
        assert_eq!(state.selected_path(), Some("c.rs"));
        state.move_up();
        state.move_up();
        state.move_up();
        assert_eq!(state.selected_path(), Some("a.rs"));
    }

    #[test]
    fn file_list_page_moves_by_the_visible_window() {
        let mut state = FileListState::new(5);
        let files: Vec<FileRow> = (0..20)
            .map(|i| row(&format!("f{i}.rs"), Some(1), Some(0)))
            .collect();
        state.set_files(files);
        state.page(1);
        assert_eq!(state.selected_path(), Some("f5.rs"));
        state.page(1);
        assert_eq!(state.selected_path(), Some("f10.rs"));
        state.page(-1);
        assert_eq!(state.selected_path(), Some("f5.rs"));
        state.page(1);
        state.page(1);
        assert_eq!(state.selected_path(), Some("f15.rs"));
        state.page(1);
        assert_eq!(
            state.selected_path(),
            Some("f19.rs"),
            "page clamps at the end"
        );
    }

    #[test]
    fn file_list_selection_survives_a_refresh_by_path() {
        let mut state = FileListState::new(5);
        state.set_files(vec![
            row("a.rs", Some(1), Some(0)),
            row("b.rs", Some(2), Some(0)),
        ]);
        state.move_down();
        assert_eq!(state.selected_path(), Some("b.rs"));
        state.set_files(vec![
            row("a.rs", Some(4), Some(1)),
            row("b.rs", Some(9), Some(3)),
            row("c.rs", Some(1), Some(0)),
        ]);
        assert_eq!(state.selected_path(), Some("b.rs"));
    }

    #[test]
    fn file_list_selection_restarts_at_the_top_when_the_path_vanishes() {
        let mut state = FileListState::new(5);
        state.set_files(vec![row("a.rs", Some(1), Some(0))]);
        state.move_down();
        assert_eq!(state.selected_path(), Some("a.rs"));
        state.set_files(vec![row("c.rs", Some(1), Some(0))]);
        assert_eq!(
            state.selected_path(),
            Some("c.rs"),
            "a vanished path resets the selection to the top"
        );
    }

    #[test]
    fn file_list_selection_resets_when_files_disappear() {
        let mut state = FileListState::new(5);
        state.set_files(vec![row("a.rs", Some(1), Some(0))]);
        state.move_down();
        state.set_files(Vec::new());
        assert_eq!(state.selected_path(), None);
        state.set_files(vec![row("b.rs", Some(1), Some(0))]);
        assert_eq!(
            state.selected_path(),
            Some("b.rs"),
            "fresh lists restart at the top"
        );
    }

    #[test]
    fn page_size_and_visible_fit_follow_the_viewport() {
        assert_eq!(visible_fit(640.0, 18.0), 35);
        assert_eq!(visible_fit(18.0, 18.0), 1);
        assert_eq!(list_max_visible(), 24);
    }

    #[test]
    fn first_changed_line_skips_context_to_the_first_change() {
        let diff = FileDiff {
            hunks: hunks(vec![
                vec![
                    change(LineKind::Context, Some(1), Some(1)),
                    change(LineKind::Context, Some(2), Some(2)),
                    change(LineKind::Added, None, Some(3)),
                    change(LineKind::Removed, Some(4), None),
                ],
                vec![change(LineKind::Added, None, Some(5))],
            ]),
            ..FileDiff::empty()
        };
        assert_eq!(first_changed_line(&diff), Some(2));
        assert_eq!(first_changed_line(&FileDiff::empty()), None);
        let context_only = FileDiff {
            hunks: hunks(vec![vec![change(LineKind::Context, Some(1), Some(1))]]),
            ..FileDiff::empty()
        };
        assert_eq!(first_changed_line(&context_only), None);
    }

    #[test]
    fn ribbon_center_target_centers_changes_and_falls_back_to_anchor() {
        assert_eq!(ribbon_center_target(Some(10), 100, 20, 0), 0);
        assert_eq!(ribbon_center_target(Some(30), 100, 20, 0), 20);
        assert_eq!(ribbon_center_target(Some(90), 100, 20, 0), 80);
        assert_eq!(
            ribbon_center_target(Some(5), 100, 20, 0),
            0,
            "near the top clamps to zero"
        );
        assert_eq!(
            ribbon_center_target(Some(3), 4, 20, 0),
            0,
            "a tiny file has no range"
        );
        assert_eq!(
            ribbon_center_target(None, 100, 20, 55),
            55,
            "no changes keep the anchor"
        );
        assert_eq!(
            ribbon_center_target(None, 100, 20, 500),
            80,
            "anchor clamps to the file"
        );
        assert_eq!(
            ribbon_center_target(None, 0, 20, 0),
            0,
            "an empty file lands at zero"
        );
    }

    fn change(kind: LineKind, old: Option<u32>, new: Option<u32>) -> LineChange {
        LineChange {
            kind,
            text: String::new(),
            token_spans: Vec::new(),
            old_line_no: old,
            new_line_no: new,
        }
    }

    fn code(text: &str, old: Option<u32>, new: Option<u32>) -> LineChange {
        LineChange {
            kind: LineKind::Context,
            text: text.to_string(),
            token_spans: qol_diff::lexer::classify(text, qol_diff::lexer::Lang::Rust),
            old_line_no: old,
            new_line_no: new,
        }
    }

    fn transition_state(old: Vec<LineChange>, new: Vec<LineChange>) -> TransitionState {
        let plan = TransitionPlan::between(&old, &new);
        let old_constructs = constructs::detect_constructs(&old, Lang::Rust);
        let old_arms = old_constructs
            .iter()
            .map(|construct| constructs::branch_arms(&old, construct, Lang::Rust))
            .collect();
        TransitionState {
            seq: 1,
            started: Instant::now(),
            old_lines: old,
            old_pair_of: Vec::new(),
            old_layout: Layout::Field,
            old_scroll: 0,
            old_gutter_width: 0,
            plan,
            token_paths: Vec::new(),
            old_constructs,
            old_arms,
        }
    }

    #[test]
    fn construct_identity_links_carried_anchors_only() {
        let old = vec![
            code("fn alpha() {", Some(1), Some(1)),
            code("    x();", Some(2), Some(2)),
            code("}", Some(3), Some(3)),
            code("fn beta() {", Some(4), Some(4)),
            code("    y();", Some(5), Some(5)),
            code("}", Some(6), Some(6)),
        ];
        let new = vec![
            code("fn alpha() {", Some(1), Some(1)),
            code("    x();", Some(2), Some(2)),
            code("}", Some(3), Some(3)),
            code("fn beta() {", Some(4), Some(4)),
            code("    y(); // tweak", Some(5), Some(5)),
            code("}", Some(6), Some(6)),
            code("fn gamma() {", Some(7), Some(7)),
            code("    z();", Some(8), Some(8)),
            code("}", Some(9), Some(9)),
        ];
        let state = transition_state(old, new);
        let new_constructs = vec![
            Construct {
                kind: qol_diff::constructs::ConstructKind::Arc,
                start_line: 0,
                end_line: 2,
                depth: 0,
                anchor: (0, 2),
            },
            Construct {
                kind: qol_diff::constructs::ConstructKind::Arc,
                start_line: 3,
                end_line: 5,
                depth: 0,
                anchor: (3, 5),
            },
            Construct {
                kind: qol_diff::constructs::ConstructKind::Arc,
                start_line: 6,
                end_line: 8,
                depth: 0,
                anchor: (6, 8),
            },
        ];
        let (index, start, end, arms) = state
            .construct_partner(&new_constructs[0])
            .expect("alpha is carried");
        assert_eq!((index, start, end, arms), (0, 0, 2, 0));
        assert!(
            state.construct_partner(&new_constructs[1]).is_some(),
            "beta morphs"
        );
        assert!(
            state.construct_partner(&new_constructs[2]).is_none(),
            "gamma is born, not morphed"
        );
        assert_eq!(state.death_indices(&new_constructs), Vec::<usize>::new());
    }

    #[test]
    fn construct_kind_change_is_a_birth_and_a_death() {
        let old = vec![
            code("fn alpha() {", Some(1), Some(1)),
            code("    x();", Some(2), Some(2)),
            code("}", Some(3), Some(3)),
        ];
        let changed = vec![
            code("struct alpha {", Some(1), Some(1)),
            code("    x: u32,", Some(2), Some(2)),
            code("}", Some(3), Some(3)),
        ];
        let state = transition_state(old, changed);
        let new_constructs = vec![Construct {
            kind: qol_diff::constructs::ConstructKind::Lattice,
            start_line: 0,
            end_line: 2,
            depth: 0,
            anchor: (0, 2),
        }];
        assert!(
            state.construct_partner(&new_constructs[0]).is_none(),
            "a kind change breaks identity"
        );
        assert_eq!(
            state.death_indices(&new_constructs),
            vec![0],
            "the old arc collapses"
        );
    }

    #[test]
    fn removed_anchors_leave_no_partner_and_no_claim() {
        let old = vec![
            code("fn alpha() {", Some(1), Some(1)),
            code("    x();", Some(2), Some(2)),
            code("}", Some(3), Some(3)),
            code("fn beta() {", Some(4), Some(4)),
            code("    y();", Some(5), Some(5)),
            code("}", Some(6), Some(6)),
        ];
        let new = vec![
            code("fn alpha() {", Some(1), Some(1)),
            code("    x();", Some(2), Some(2)),
            code("}", Some(3), Some(3)),
        ];
        let state = transition_state(old, new);
        let new_constructs = vec![Construct {
            kind: qol_diff::constructs::ConstructKind::Arc,
            start_line: 0,
            end_line: 2,
            depth: 0,
            anchor: (0, 2),
        }];
        assert_eq!(
            state.death_indices(&new_constructs),
            vec![1],
            "the removed beta collapses"
        );
    }

    fn hunks(blocks: Vec<Vec<LineChange>>) -> Vec<qol_diff::Hunk> {
        blocks
            .into_iter()
            .map(|lines| qol_diff::Hunk {
                old_start: 1,
                old_lines: 0,
                new_start: 1,
                new_lines: 0,
                lines,
            })
            .collect()
    }

    #[test]
    fn split_pairs_align_context_one_to_one() {
        let (pairs, pair_starts, line_starts) = build_pairs(&hunks(vec![vec![
            change(LineKind::Context, Some(1), Some(1)),
            change(LineKind::Context, Some(2), Some(2)),
        ]]));
        assert_eq!(
            pairs,
            vec![
                LinePair {
                    old: Some(0),
                    new: Some(0)
                },
                LinePair {
                    old: Some(1),
                    new: Some(1)
                },
            ]
        );
        assert_eq!(pair_starts, vec![0]);
        assert_eq!(line_starts, vec![0]);
    }

    #[test]
    fn split_pairs_align_change_runs_by_position() {
        let (pairs, ..) = build_pairs(&hunks(vec![vec![
            change(LineKind::Removed, Some(1), None),
            change(LineKind::Removed, Some(2), None),
            change(LineKind::Added, None, Some(2)),
            change(LineKind::Added, None, Some(3)),
            change(LineKind::Added, None, Some(4)),
        ]]));
        assert_eq!(
            pairs,
            vec![
                LinePair {
                    old: Some(0),
                    new: Some(2)
                },
                LinePair {
                    old: Some(1),
                    new: Some(3)
                },
                LinePair {
                    old: None,
                    new: Some(4)
                },
            ]
        );
    }

    #[test]
    fn split_pairs_interleave_removed_and_added() {
        let (pairs, ..) = build_pairs(&hunks(vec![vec![
            change(LineKind::Removed, Some(1), None),
            change(LineKind::Added, None, Some(1)),
            change(LineKind::Removed, Some(2), None),
            change(LineKind::Added, None, Some(2)),
        ]]));
        assert_eq!(
            pairs,
            vec![
                LinePair {
                    old: Some(0),
                    new: Some(1)
                },
                LinePair {
                    old: Some(2),
                    new: Some(3)
                },
            ]
        );
    }

    #[test]
    fn split_pairs_track_hunk_starts_across_hunks() {
        let (pairs, pair_starts, line_starts) = build_pairs(&hunks(vec![
            vec![change(LineKind::Context, Some(1), Some(1))],
            vec![
                change(LineKind::Removed, Some(2), None),
                change(LineKind::Added, None, Some(2)),
                change(LineKind::Context, Some(3), Some(3)),
            ],
        ]));
        assert_eq!(pairs.len(), 3);
        assert_eq!(pair_starts, vec![0, 1]);
        assert_eq!(line_starts, vec![0, 1]);
    }

    #[test]
    fn split_pair_count_is_max_of_sides_per_run() {
        let (pairs, ..) = build_pairs(&hunks(vec![vec![
            change(LineKind::Removed, Some(1), None),
            change(LineKind::Context, Some(2), Some(2)),
            change(LineKind::Added, None, Some(3)),
        ]]));
        assert_eq!(pairs.len(), 3);
        assert_eq!(
            pairs[1],
            LinePair {
                old: Some(1),
                new: Some(1)
            },
            "context between one-sided runs stays aligned"
        );
    }

    #[test]
    fn old_pair_rows_map_every_flat_line_to_its_split_row() {
        let pairs = vec![
            LinePair {
                old: Some(0),
                new: Some(0),
            },
            LinePair {
                old: Some(1),
                new: Some(2),
            },
            LinePair {
                old: None,
                new: Some(3),
            },
        ];
        assert_eq!(
            old_pair_rows(&pairs, 4),
            vec![Some(0), Some(1), Some(1), Some(2)]
        );
        assert_eq!(
            old_pair_rows(&pairs, 2),
            vec![Some(0), Some(1)],
            "unlisted flat lines stay unplaced"
        );
        assert_eq!(old_pair_rows(&[], 0), Vec::<Option<usize>>::new());
    }

    #[test]
    fn side_gutter_pads_and_blanks() {
        let line = |old: Option<u32>, new: Option<u32>| LineChange {
            kind: LineKind::Context,
            text: String::new(),
            token_spans: Vec::new(),
            old_line_no: old,
            new_line_no: new,
        };
        assert_eq!(
            side_gutter_labels(&line(Some(7), Some(8)), 4),
            ("   7".to_string(), "   8".to_string())
        );
        assert_eq!(
            side_gutter_labels(&line(None, None), 4),
            ("    ".to_string(), "    ".to_string())
        );
        assert_eq!(
            unified_gutter(&line(Some(300), None), 3),
            "300     ".to_string()
        );
    }

    #[test]
    fn decayed_line_style_feeds_cooler_heat_to_the_color_mapping() {
        let hot = LineStyle {
            background_heat: HeatLevel::Hot,
            dimmed: false,
        };
        let warm = LineStyle {
            background_heat: HeatLevel::Warm,
            dimmed: false,
        };
        let cool = LineStyle {
            background_heat: HeatLevel::Cool,
            dimmed: false,
        };
        assert_eq!(decayed_line_style(hot, None), hot, "no age renders fresh");
        assert_eq!(decayed_line_style(hot, Some(Duration::from_secs(30))), hot);
        assert_eq!(decayed_line_style(hot, Some(Duration::from_secs(60))), warm);
        assert_eq!(
            decayed_line_style(hot, Some(Duration::from_secs(300))),
            cool
        );
        assert_eq!(
            decayed_line_style(warm, Some(Duration::from_secs(299))),
            warm
        );
        assert_eq!(
            decayed_line_style(warm, Some(Duration::from_secs(300))),
            cool
        );
        assert_eq!(
            decayed_line_style(cool, Some(Duration::from_secs(86_400))),
            cool
        );
        assert_eq!(
            decayed_line_style(
                LineStyle {
                    background_heat: HeatLevel::Hot,
                    dimmed: true
                },
                Some(Duration::from_secs(60))
            ),
            LineStyle {
                background_heat: HeatLevel::Warm,
                dimmed: true
            },
            "dimmed survives decay"
        );
    }

    #[test]
    fn token_span_style_maps_heat_and_kind() {
        let line = LineChange {
            kind: LineKind::Added,
            text: "fn main".to_string(),
            token_spans: vec![
                qol_diff::TokenSpan {
                    start: 0,
                    len: 2,
                    heat: HeatLevel::Cool,
                    kind: TokenKind::Keyword,
                },
                qol_diff::TokenSpan {
                    start: 2,
                    len: 5,
                    heat: HeatLevel::Hot,
                    kind: TokenKind::Plain,
                },
            ],
            old_line_no: None,
            new_line_no: Some(1),
        };
        assert_eq!(
            token_span_style(&line, 0, 2, None),
            (None, surface::token_kind_color(TokenKind::Keyword))
        );
        assert_eq!(
            token_span_style(&line, 2, 2, None),
            (surface::token_background(HeatLevel::Hot), None)
        );
        assert_eq!(
            token_span_style(&line, 2, 2, Some(Duration::from_secs(60))),
            (surface::token_background(HeatLevel::Warm), None),
            "age decays hot to warm before the mapping runs"
        );
        assert_eq!(token_span_style(&line, 7, 2, None), (None, None));
        assert_eq!(token_span_style(&line, 20, 4, None), (None, None));
    }

    #[test]
    fn flash_hex_burns_ember_core_and_settles_onto_the_span_bg() {
        assert_eq!(flash_hex(None, 1.0), (surface::EMBER_CORE << 8) | 0xff);
        assert_eq!(flash_hex(None, 0.0), surface::EMBER_CORE << 8);
        assert_eq!(
            flash_hex(Some(0x000000), 1.0),
            (surface::EMBER_CORE << 8) | 0xff,
            "the peak flash is the ember core, not white"
        );
        assert_eq!(flash_hex(Some(0x000000), 0.0), 0x000000ff);
    }

    #[test]
    fn flare_hex_peaks_at_the_ember_core() {
        assert_eq!(flare_hex(None, 0.0), surface::TOKEN_MORPH_FLARE << 8);
        assert_eq!(
            flare_hex(None, 1.0),
            (surface::TOKEN_MORPH_FLARE << 8) | 0xff
        );
        assert_eq!(
            flare_hex(Some(0x000000), 1.0),
            (surface::TOKEN_MORPH_FLARE << 8) | 0xff
        );
        assert_eq!(surface::TOKEN_MORPH_FLARE, surface::EMBER_CORE);
    }

    #[test]
    fn linear_time_inverts_the_quint_out_easing() {
        assert_eq!(linear_time(0.0), 0.0);
        assert_eq!(linear_time(1.0), 1.0);
        let mid = linear_time(0.5);
        assert!(
            (mid - 0.12945).abs() < 0.001,
            "quint-out reaches 50% progress at 13% of wall time"
        );
        assert!((linear_time(0.96875) - 0.5).abs() < 0.001);
        assert!(linear_time(0.5) < linear_time(0.96875));
    }

    #[test]
    fn scale_hex_multiplies_each_channel_within_bounds() {
        assert_eq!(scale_hex(0x808080, 1.5), 0xc0c0c0);
        assert_eq!(scale_hex(0xff8080, 1.5), 0xffc0c0);
        assert_eq!(scale_hex(0x000000, 0.5), 0x000000);
        assert_eq!(scale_hex(0xffffff, 0.0), 0x000000);
    }

    #[test]
    fn pulsed_background_breathes_heat_and_leaves_cool_alone() {
        let hot = LineStyle {
            background_heat: HeatLevel::Hot,
            dimmed: false,
        };
        let cool = LineStyle {
            background_heat: HeatLevel::Cool,
            dimmed: false,
        };
        let base = surface::line_background(hot).unwrap();
        assert_eq!(
            pulsed_background(hot, None),
            Some(base),
            "no heat age means no pulse"
        );
        assert_eq!(pulsed_background(cool, Some(Duration::from_secs(5))), None);
        let peak = pulsed_background(hot, Some(Duration::from_secs_f32(0.625))).unwrap();
        assert!(peak > base, "hot backgrounds brighten on the pulse upswing");
        let trough = pulsed_background(hot, Some(Duration::from_secs_f32(1.875))).unwrap();
        assert!(trough < base, "hot backgrounds dim on the pulse downswing");
    }

    #[test]
    fn mix_hex_interpolates_each_channel() {
        assert_eq!(mix_hex(0x000000, 0xffffff, 0.5), 0x808080);
        assert_eq!(mix_hex(0x112233, 0x112233, 0.75), 0x112233);
        assert_eq!(mix_hex(0xff0000, 0x0000ff, 1.0), 0x0000ff);
        assert_eq!(mix_hex(0xff0000, 0x0000ff, 0.0), 0xff0000);
    }
}
