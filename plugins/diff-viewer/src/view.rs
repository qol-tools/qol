use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, App, Context, Entity, FocusHandle, Focusable, HighlightStyle,
    KeyDownEvent, ScrollDelta, ScrollWheelEvent, SharedString, StyledText, WhiteSpace, Window,
};
use qol_cache::TtlCache;
use qol_diff::{DiffError, FileDiff, HeatLevel, LineChange, LineKind, TokenKind};
use qol_gpui::scroll_list::ScrollList;
use qol_gpui::surface::SurfaceDismisser;

use crate::overview::{HunkMarker, OverviewView};
use crate::pipeline::{self, commit_range, Facts, GitRequest, GitResult};
use crate::scrubber::{Commit as ScrubCommit, ScrubberView};
use crate::surface::{self, CodeSurface};

pub const WINDOW_WIDTH: f32 = 960.0;
pub const WINDOW_HEIGHT: f32 = 640.0;
const LIST_WIDTH: f32 = 340.0;
const ROW_HEIGHT: f32 = 26.0;
const LINE_HEIGHT: f32 = 18.0;
const FONT_SIZE: f32 = 13.0;
const GLYPH_WIDTH: f32 = 12.0;
const PIXELS_PER_NOTCH: f32 = 50.0;
const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const FACTS_TTL: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

impl FileRow {
    pub fn counts(&self) -> (Option<String>, Option<String>) {
        (
            self.added.map(|count| format!("+{count}")),
            self.deleted.map(|count| format!("-{count}")),
        )
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
        if let Some(path) = selected {
            if let Some(index) = self.files.iter().position(|file| file.path == path) {
                self.list.selected = index;
                self.list.sync(self.files.len());
            }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivePane {
    Files,
    Code,
    Scrubber,
}

impl ActivePane {
    fn other(self) -> Self {
        match self {
            Self::Files => Self::Code,
            Self::Code => Self::Scrubber,
            Self::Scrubber => Self::Files,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum DiffPane {
    Empty,
    Error(DiffError),
    Ready(FileDiff),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Unified,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinePair {
    old: Option<usize>,
    new: Option<usize>,
}

pub struct DiffView {
    repo: Option<PathBuf>,
    files: FileListState,
    pane: DiffPane,
    surface: CodeSurface,
    lines: Vec<LineChange>,
    active_pane: ActivePane,
    layout: Layout,
    pairs: Vec<LinePair>,
    hunk_pair_starts: Vec<usize>,
    hunk_line_starts: Vec<usize>,
    facts_error: Option<String>,
    last_fit: usize,
    generation: Arc<AtomicU64>,
    git_tx: mpsc::Sender<GitRequest>,
    results: Option<mpsc::Receiver<GitResult>>,
    facts_cache: TtlCache<PathBuf, Facts>,
    font_family: SharedString,
    focus_handle: FocusHandle,
    dismisser: SurfaceDismisser,
    scrubber_view: Entity<ScrubberView>,
    overview_view: Entity<OverviewView>,
    jump_rx: Option<mpsc::Receiver<f32>>,
    scrub_commits: Vec<ScrubCommit>,
    last_scrub_selected: Option<usize>,
}

impl DiffView {
    pub fn new(
        repo: Option<PathBuf>,
        dismisser: SurfaceDismisser,
        git_tx: mpsc::Sender<GitRequest>,
        generation: Arc<AtomicU64>,
        results: mpsc::Receiver<GitResult>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (jump_tx, jump_rx) = mpsc::channel::<f32>();
        let scrubber_view = cx.new(|cx| ScrubberView::new(cx, None));
        let overview_view = cx.new(|_cx| {
            let tx = jump_tx.clone();
            OverviewView::new(move |ratio, _app| {
                let _ = tx.send(ratio);
            })
        });
        let mut view = Self {
            repo,
            files: FileListState::new(list_max_visible()),
            pane: DiffPane::Empty,
            surface: CodeSurface::new(),
            lines: Vec::new(),
            active_pane: ActivePane::Files,
            layout: Layout::Split,
            pairs: Vec::new(),
            hunk_pair_starts: Vec::new(),
            hunk_line_starts: Vec::new(),
            facts_error: None,
            last_fit: 0,
            generation,
            git_tx,
            results: Some(results),
            facts_cache: TtlCache::new(FACTS_TTL),
            font_family: pick_monospace(cx),
            focus_handle: cx.focus_handle(),
            dismisser,
            scrubber_view,
            overview_view,
            jump_rx: Some(jump_rx),
            scrub_commits: Vec::new(),
            last_scrub_selected: None,
        };
        if view.repo.is_none() {
            view.facts_error =
                Some("no repository: set QOL_DIFF_REPO or launch from a git worktree".to_string());
            return view;
        }
        view.spawn_loops(cx);
        pipeline::send_refresh(&view.git_tx, &view.generation);
        pipeline::send_history(&view.git_tx, &view.generation);
        view
    }

    fn spawn_loops(&mut self, cx: &mut Context<Self>) {
        self.spawn_result_poll(cx);
        self.spawn_backstop(cx);
    }

    fn spawn_result_poll(&mut self, cx: &mut Context<Self>) {
        let rx = self.results.take().expect("result poll spawned once");
        let jump_rx = self.jump_rx.take().expect("jump poll spawned once");
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
            if results.is_empty() && jumps.is_empty() {
                continue;
            }
            let current = generation.load(Ordering::SeqCst);
            let _ = this.update(cx, |view, cx| {
                let mut changed = view.apply_results(results, current, cx);
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
        let this = cx.weak_entity();
        cx.spawn(async move |_view, cx| loop {
            cx.background_executor().timer(REFRESH_INTERVAL).await;
            let _ = this.update(cx, |view, cx| {
                if view.apply_cached_facts() {
                    cx.notify();
                }
            });
            pipeline::send_refresh(&git_tx, &generation);
        })
        .detach();
    }

    fn apply_cached_facts(&mut self) -> bool {
        let Some(facts) = self
            .facts_cache
            .get(
                self.repo
                    .as_ref()
                    .expect("repo present when facts are cached"),
            )
            .cloned()
        else {
            return false;
        };
        self.apply_facts(facts);
        true
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
                    self.apply_facts(facts);
                    changed = true;
                }
                GitResult::FactsFailed {
                    generation,
                    message,
                } if generation == current => {
                    self.facts_error = Some(message);
                    changed = true;
                }
                GitResult::Diff {
                    generation,
                    path,
                    diff,
                } if generation == current && self.files.selected_path() == Some(path.as_str()) => {
                    self.apply_diff(diff);
                    changed = true;
                }
                GitResult::History {
                    generation,
                    commits,
                } if generation == current => {
                    let commits: Vec<ScrubCommit> = commits
                        .into_iter()
                        .map(|entry| ScrubCommit::new(entry.sha, entry.subject))
                        .collect();
                    self.scrub_commits = commits;
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
        self.last_scrub_selected = Some(selected);
        self.request_scrub_diff(selected);
        true
    }

    fn request_scrub_diff(&mut self, index: usize) {
        let Some(path) = self.files.selected_path() else {
            return;
        };
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.git_tx.send(GitRequest::SelectFile {
            generation,
            path: path.to_owned(),
            range: commit_range(index),
        });
    }

    fn apply_facts(&mut self, facts: Facts) {
        self.facts_error = None;
        let files = facts
            .numstat
            .iter()
            .map(|entry| FileRow {
                path: entry.path.clone(),
                added: entry.added,
                deleted: entry.deleted,
            })
            .collect();
        self.files.set_files(files);
        self.facts_cache.insert(
            self.repo
                .clone()
                .expect("repo present when facts are cached"),
            facts,
        );
        if self.files.files.is_empty() {
            self.pane = DiffPane::Empty;
        }
    }

    fn apply_diff(&mut self, diff: Result<FileDiff, DiffError>) {
        match diff {
            Ok(diff) if diff.is_empty() => {
                self.lines.clear();
                self.surface.set_lines(Vec::new());
                self.pane = DiffPane::Empty;
            }
            Ok(diff) => {
                let lines = flatten(&diff);
                let (pairs, hunk_pair_starts, hunk_line_starts) = build_pairs(&diff.hunks);
                self.surface.set_lines(lines.clone());
                self.lines = lines;
                self.pairs = pairs;
                self.hunk_pair_starts = hunk_pair_starts;
                self.hunk_line_starts = hunk_line_starts;
                self.pane = DiffPane::Ready(diff);
            }
            Err(error) => self.pane = DiffPane::Error(error),
        }
    }

    fn select_current_file(&mut self) {
        let Some(path) = self.files.selected_path() else {
            return;
        };
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.git_tx.send(GitRequest::SelectFile {
            generation,
            path: path.to_owned(),
            range: pipeline::DEFAULT_RANGE.to_string(),
        });
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let page = self.page_size();
        match (self.active_pane, key) {
            (_, "tab") => {
                self.active_pane = self.active_pane.other();
                self.refocus(_window, cx);
            }
            (_, "escape") | (_, "esc") => {
                self.dismisser.dismiss(cx);
                return;
            }
            (ActivePane::Files, "down") | (ActivePane::Files, "j") => self.files.move_down(),
            (ActivePane::Files, "up") | (ActivePane::Files, "k") => self.files.move_up(),
            (ActivePane::Files, "pagedown") | (ActivePane::Files, "page_down") => {
                self.files.page(1)
            }
            (ActivePane::Files, "pageup") | (ActivePane::Files, "page_up") => self.files.page(-1),
            (ActivePane::Files, "enter") | (ActivePane::Files, "return") => {
                self.select_current_file();
            }
            (ActivePane::Code, "down") | (ActivePane::Code, "j") => self.scroll_lines(1),
            (ActivePane::Code, "up") | (ActivePane::Code, "k") => self.scroll_lines(-1),
            (ActivePane::Code, "s") => {
                self.layout = match self.layout {
                    Layout::Unified => Layout::Split,
                    Layout::Split => Layout::Unified,
                };
            }
            (ActivePane::Code, "pagedown") | (ActivePane::Code, "page_down") => {
                self.scroll_lines(page as isize)
            }
            (ActivePane::Code, "pageup") | (ActivePane::Code, "page_up") => {
                self.scroll_lines(-(page as isize))
            }
            _ => return,
        }
        cx.notify();
    }

    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_pane {
            ActivePane::Scrubber => {
                let handle = self
                    .scrubber_view
                    .update(cx, |view, cx| view.focus_handle(cx));
                window.focus(&handle);
            }
            _ => window.focus(&self.focus_handle),
        }
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
        let max = self.row_count().saturating_sub(self.last_fit.max(1));
        let target = self.surface.scroll_offset() as isize + delta;
        self.surface
            .set_scroll_offset(target.clamp(0, max as isize) as usize);
    }

    fn row_count(&self) -> usize {
        match self.layout {
            Layout::Unified => self.lines.len(),
            Layout::Split => self.pairs.len(),
        }
    }

    fn page_size(&self) -> usize {
        self.last_fit.max(1)
    }

    fn render_file_list(&self) -> AnyElement {
        let rows: Vec<AnyElement> = self
            .files
            .visible_range()
            .map(|index| self.file_row(index))
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

    fn file_row(&self, index: usize) -> AnyElement {
        let file = &self.files.files[index];
        let selected = index == self.files.list.selected;
        let (added, deleted) = file.counts();
        let mut row = div()
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .text_size(px(12.0));
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
        .child(count_cell(added, surface::TEXT_ADDED))
        .child(count_cell(deleted, surface::TEXT_REMOVED))
        .into_any_element()
    }

    fn render_pane(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pane = div()
            .id("diff-pane")
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(rgb(surface::CANVAS_BG))
            .on_scroll_wheel(cx.listener(Self::on_scroll));
        match &self.pane {
            DiffPane::Empty => pane
                .child(center_message("No changes", surface::TEXT_MUTED))
                .into_any_element(),
            DiffPane::Error(error) => pane
                .child(center_message(&error.to_string(), surface::ERROR_TEXT))
                .into_any_element(),
            DiffPane::Ready(_) => match self.layout {
                Layout::Split => pane.child(self.split_columns(window)).into_any_element(),
                Layout::Unified => pane.children(self.code_rows(window)).into_any_element(),
            },
        }
    }

    fn split_columns(&self, window: &Window) -> AnyElement {
        let start = self.surface.scroll_offset();
        let end = (start + self.last_fit.max(1)).min(self.row_count());
        let mut old_rows = Vec::new();
        let mut new_rows = Vec::new();
        for row in start..end {
            old_rows.push(self.split_row(row, true, window));
            new_rows.push(self.split_row(row, false, window));
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
                    .children(old_rows),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .children(new_rows),
            )
            .into_any_element()
    }

    fn split_row(&self, row: usize, is_old: bool, window: &Window) -> AnyElement {
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
        let style = line_index
            .and_then(|index| self.lines.get(index))
            .map(surface::style_from_line)
            .unwrap_or_default();
        let blank_bg = counterpart
            .and_then(|index| self.lines.get(index))
            .and_then(|line| surface::line_background(surface::style_from_line(line)));
        if let Some(bg) = surface::line_background(style) {
            row_el = row_el.bg(rgb(bg));
        } else if let Some(bg) = blank_bg {
            row_el = row_el.bg(rgb(bg));
        }
        let width = self.surface.gutter_width();
        match line_index {
            Some(index) => {
                let line = &self.lines[index];
                let label = if is_old {
                    line.old_line_no
                } else {
                    line.new_line_no
                };
                let glyph = match (is_old, line.kind) {
                    (true, LineKind::Removed) => "-",
                    (false, LineKind::Added) => "+",
                    _ => " ",
                };
                row_el
                    .child(
                        div()
                            .flex_none()
                            .px_1()
                            .text_color(rgb(surface::GUTTER_TEXT))
                            .child(side_gutter(label, width)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(GLYPH_WIDTH))
                            .text_color(rgb(surface::kind_color(line.kind)))
                            .child(glyph),
                    )
                    .child(self.code_text(line, style.dimmed, window))
                    .into_any_element()
            }
            None => row_el
                .child(
                    div()
                        .flex_none()
                        .px_1()
                        .text_color(rgb(surface::GUTTER_TEXT))
                        .child(side_gutter(None, width)),
                )
                .child(div().flex_none().w(px(GLYPH_WIDTH)).child(" "))
                .child(div().flex_1().child(" "))
                .into_any_element(),
        }
    }

    fn code_rows(&self, window: &Window) -> Vec<AnyElement> {
        let start = self.surface.scroll_offset();
        let end = (start + self.last_fit.max(1)).min(self.surface.line_count());
        (start..end)
            .map(|index| self.code_row(index, window))
            .collect()
    }

    fn code_row(&self, index: usize, window: &Window) -> AnyElement {
        let line = &self.lines[index];
        let style = self.surface.line_style(index);
        let mut row = div()
            .h(px(LINE_HEIGHT))
            .w_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .line_height(px(LINE_HEIGHT))
            .text_size(px(FONT_SIZE));
        if let Some(bg) = surface::line_background(style) {
            row = row.bg(rgb(bg));
        }
        let (old, new) = surface::gutter_labels(
            line.old_line_no,
            line.new_line_no,
            self.surface.gutter_width(),
        );
        row.child(
            div()
                .flex_none()
                .px_1()
                .text_color(rgb(surface::GUTTER_TEXT))
                .child(format!("{old}  {new}")),
        )
        .child(
            div()
                .flex_none()
                .w(px(GLYPH_WIDTH))
                .text_color(rgb(surface::kind_color(line.kind)))
                .child(surface::kind_glyph(line.kind)),
        )
        .child(self.code_text(line, style.dimmed, window))
        .into_any_element()
    }

    fn code_text(&self, line: &LineChange, dimmed: bool, window: &Window) -> StyledText {
        let mut text_style = window.text_style();
        text_style.font_family = self.font_family.clone();
        text_style.font_size = px(FONT_SIZE).into();
        text_style.line_height = px(LINE_HEIGHT).into();
        text_style.color = rgb(surface::text_color(dimmed)).into();
        text_style.white_space = WhiteSpace::Nowrap;
        let highlights = line
            .token_spans
            .iter()
            .filter_map(|span| {
                let start = span.start.min(line.text.len());
                let end = (span.start + span.len).min(line.text.len());
                (start < end).then_some((
                    start..end,
                    HighlightStyle {
                        background_color: surface::token_background(span.heat)
                            .map(|hex| rgb(hex).into()),
                        color: match (span.heat, span.kind) {
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
            .collect::<Vec<_>>();
        StyledText::new(line.text.clone()).with_default_highlights(&text_style, highlights)
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.last_fit = visible_fit(window.viewport_size().height.to_f64() as f32, LINE_HEIGHT);
        self.update_overview(cx);
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
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .h_full()
                    .child(self.render_file_list())
                    .child(self.render_pane(window, cx))
                    .child(self.overview_view.clone()),
            )
            .child(scrubber)
    }
}

impl DiffView {
    fn update_overview(&mut self, cx: &mut Context<Self>) {
        let (markers, total) = match &self.pane {
            DiffPane::Ready(diff) => {
                let starts = match self.layout {
                    Layout::Unified => self.hunk_line_starts.clone(),
                    Layout::Split => self.hunk_pair_starts.clone(),
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
                (markers, total)
            }
            _ => (Vec::new(), 0),
        };
        let viewport = if total == 0 {
            (0.0f32, 1.0f32)
        } else {
            let start = self.surface.scroll_offset();
            let end = (start + self.last_fit.max(1)).min(total);
            (start as f32 / total as f32, end as f32 / total as f32)
        };
        self.overview_view.update(cx, |view, _cx| {
            view.set_markers(markers);
            view.set_viewport(viewport);
        });
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

fn side_gutter(label: Option<u32>, width: usize) -> String {
    match label {
        Some(number) => format!("{number:>width$}"),
        None => " ".repeat(width),
    }
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

fn count_cell(label: Option<String>, color: u32) -> AnyElement {
    let (label, color) = label
        .map(|label| (label, color))
        .unwrap_or_else(|| ("bin".to_string(), surface::TEXT_MUTED));
    div()
        .flex_none()
        .text_color(rgb(color))
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, added: Option<u64>, deleted: Option<u64>) -> FileRow {
        FileRow {
            path: path.to_string(),
            added,
            deleted,
        }
    }

    #[test]
    fn count_labels_format_added_and_deleted() {
        assert_eq!(
            row("a.rs", Some(3), Some(2)).counts(),
            (Some("+3".to_string()), Some("-2".to_string()))
        );
        assert_eq!(
            row("logo.png", None, None).counts(),
            (None, None),
            "binary files carry no counts"
        );
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
    fn active_pane_cycles_files_code_scrubber() {
        assert_eq!(ActivePane::Files.other(), ActivePane::Code);
        assert_eq!(ActivePane::Code.other(), ActivePane::Scrubber);
        assert_eq!(ActivePane::Scrubber.other(), ActivePane::Files);
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
    fn side_gutter_pads_and_blanks() {
        assert_eq!(side_gutter(Some(7), 4), "   7");
        assert_eq!(side_gutter(None, 4), "    ");
        assert_eq!(side_gutter(Some(300), 3), "300");
    }
}
