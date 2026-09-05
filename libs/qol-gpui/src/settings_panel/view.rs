use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_config::contract::{resolve_slider_action, ResolvedRowAction};
use qol_config::object_array::pretty_label;

use super::components::{
    paint_settings_selection, rail_caption, settings_label, settings_label_group, settings_page,
    SettingsFeedback, SettingsGroupHeader, SettingsRow, SettingsSelectValue, SettingsToggle,
};
use super::object_array_row::{
    shared_key_chip, Chip, ChipTone, DraftField, DraftValue, ItemChips, ObjectArrayOutcome,
    ObjectArrayState,
};
use super::persistence::{panel_base, save_values};
use super::rows::{
    apply_runtime_query, begin_list_item_action, filtered_list_items, list_item_actions,
    list_slider_value, merged_config, primary_list_item_action, query_flag_value, row_action,
    row_query_names, row_streams, runtime_query_names, selected_list_item, stream_gated,
    ListActions, ListItem, ListSlider, Row, RowControl, RowQueryState, RowSection, SelectOption,
    SliderHold,
};
use super::{
    CustomPanelCallback, CustomPanelContext, CustomPanelFactory, CustomPanelNotifier,
    CustomPanelView, PanelSourceGroup, SettingsPanel, SettingsRuntime, SourceState,
};
use crate::color_wheel::{ColorWheel, ColorWheelPopup, WheelCallbacks, WheelStyle};
use crate::deck::{self, Motion as DeckMotion, Slide as DeckSlide};
use crate::dropdown::{Dropdown, DropdownEvent, DropdownItem, DropdownStyle};
use crate::gamepad::{gamepad_panel, GamepadPalette};
use crate::phantom_nav::{NavAxis, PhantomNavGuard};
use crate::spinner::Spinner;
use crate::status_indicator::{StatusIndicator, StatusTone};
use crate::surface::{PanelDragArea, SurfaceDismisser};
use crate::theme::{settings_panel_runtime, SettingsPanelPalette};

type SampledQueryResults =
    std::sync::Arc<std::sync::Mutex<Vec<(String, Result<serde_json::Value, String>)>>>;

const FRAME_PACED_QUERY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const FILTER_OVERLAY_HEIGHT: f32 = super::PANEL_FILTER_HEIGHT + qol_theme::SPACE_GUTTER;
const SLIDER_DISPATCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(200);
/// How long the rail selection must hold still before its source starts polling.
const QUERY_SETTLE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(140);
/// How long a runtime query may take before its row admits it is waiting.
const QUERY_LOADING_GRACE: std::time::Duration = std::time::Duration::from_millis(300);
const SLIDER_HOLD_DURATION: std::time::Duration = std::time::Duration::from_secs(10);
const LIST_FIT_MIN_VISIBLE: usize = 3;
const BAND_TEXT_LINE_HEIGHT: f32 = 20.0;
const CRUMB_MAX_WIDTH: f32 = 200.0;
const RAIL_CARD_OVERLAP: f32 = 98.0;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PanelFocus {
    Sources,
    Body,
}

fn focus_level(source_menu: bool, sources: usize) -> PanelFocus {
    if source_menu && sources > 1 {
        PanelFocus::Sources
    } else {
        PanelFocus::Body
    }
}

const RAIL_TRANSITION: std::time::Duration = std::time::Duration::from_millis(180);
const RAIL_CARD_ACCENT: f32 = 1.5;
const RAIL_DIM: f32 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TransitionAction {
    Animate,
    Snap,
}

#[derive(Debug, Default)]
struct TransitionTracker {
    step: usize,
    started: Option<std::time::Instant>,
    snapped: bool,
}

impl TransitionTracker {
    fn state_changed(&mut self, changed: bool, now: std::time::Instant) {
        let Some(action) = transition_policy(transition_in_flight(self.started, now), changed)
        else {
            return;
        };
        match action {
            TransitionAction::Animate => {
                self.step = self.step.wrapping_add(1);
                self.started = Some(now);
                self.snapped = false;
            }
            TransitionAction::Snap => self.snapped = true,
        }
    }
}

fn transition_policy(in_flight: bool, state_changed: bool) -> Option<TransitionAction> {
    if !state_changed {
        return None;
    }
    if in_flight {
        return Some(TransitionAction::Snap);
    }
    Some(TransitionAction::Animate)
}

fn transition_in_flight(started: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    started.is_some_and(|started| now.duration_since(started) < RAIL_TRANSITION)
}

/// Collapses the states of every query a row depends on into the one the row
/// should show. Unavailable wins over loading, loading wins over ready, and a
/// query still inside its grace period is treated as ready so healthy plugins
/// never flash an indicator.
fn rollup_query_state<'a>(
    states: impl Iterator<Item = Option<&'a RowQueryState>>,
    grace: std::time::Duration,
    now: std::time::Instant,
) -> RowQueryState {
    let mut rolled = RowQueryState::Idle;
    for state in states {
        match state {
            Some(RowQueryState::Unavailable(message)) => {
                return RowQueryState::Unavailable(message.clone());
            }
            Some(RowQueryState::Loading { since })
                if now.saturating_duration_since(*since) >= grace =>
            {
                rolled = RowQueryState::Loading { since: *since };
            }
            Some(RowQueryState::Ready) if rolled == RowQueryState::Idle => {
                rolled = RowQueryState::Ready
            }
            _ => {}
        }
    }
    rolled
}

fn query_is_due(due: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    due.is_none_or(|due| due <= now)
}

fn due_query_indices(due: &[Option<std::time::Instant>], now: std::time::Instant) -> Vec<usize> {
    due.iter()
        .enumerate()
        .filter_map(|(index, due)| query_is_due(*due, now).then_some(index))
        .collect()
}

#[derive(Debug, Default)]
struct HeightCache {
    revision: u64,
    cached: Option<f32>,
}

impl HeightCache {
    fn value(&mut self, revision: u64, compute: impl FnOnce() -> f32) -> f32 {
        if self.cached.is_none() || self.revision != revision {
            self.revision = revision;
            self.cached = Some(compute());
        }
        self.cached.unwrap()
    }
}

use std::sync::PoisonError;

#[derive(Default)]
struct SampleSignal {
    state: std::sync::Mutex<SampleSignalState>,
    requested: std::sync::Condvar,
}

#[derive(Default)]
struct SampleSignalState {
    requested: bool,
    stopped: bool,
}

impl SampleSignal {
    fn request(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.requested = true;
        self.requested.notify_one();
    }

    fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.stopped = true;
        self.requested.notify_one();
    }

    fn wait_for_request(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while !state.requested && !state.stopped {
            state = self
                .requested
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.requested = false;
        !state.stopped
    }
}

pub(super) struct SettingsPanelView {
    panel: SettingsPanel,
    stack: Vec<Level>,
    subtitle: Option<String>,
    runtime: SettingsRuntime,
    runtime_queries: Vec<String>,
    sources: Vec<SourceState>,
    selected_source: usize,
    /// The source the body is actually showing. It lags `selected_source`
    /// while the rail is moving, so scrolling past a plugin never materialises
    /// its page.
    materialized_source: usize,
    source_settle_generation: u64,
    source_menu: bool,
    rail_transition: TransitionTracker,
    deck_transition: TransitionTracker,
    deck_motion: Option<DeckMotion>,
    height_cap: f32,
    height_revision: u64,
    height_cache: HeightCache,
    save_error: Option<String>,
    filter: String,
    filter_open: bool,
    wheel_generation: u64,
    runtime_poll_generation: u64,
    slider_dispatch_generation: u64,
    slider_drag: Option<(usize, String)>,
    slider_pending: std::collections::HashSet<(usize, String)>,
    frame_paced_samples: Option<SampledQueryResults>,
    applied_query_payloads: std::collections::HashMap<String, Result<serde_json::Value, String>>,
    /// Per-source query freshness, so a row backed by a wedged daemon can say
    /// so instead of silently rendering its contract default. Keyed by source
    /// because query names repeat across plugins.
    query_states: std::collections::HashMap<(usize, String), RowQueryState>,
    frame_pump_armed: bool,
    motion_tick: Option<std::time::Instant>,
    sample_signal: Option<std::sync::Arc<SampleSignal>>,
    sampler_stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    poll_visible: std::sync::Arc<std::sync::atomic::AtomicBool>,
    dismisser: SurfaceDismisser,
    palette: SettingsPanelPalette,
    kit: crate::kit::Kit,
    streams: Vec<super::stream::StreamClient>,
    focus_handle: FocusHandle,
    nav_guard: PhantomNavGuard,
    custom_views: Vec<Option<CustomPanelView>>,
    custom_focus_pending: bool,
}

pub(super) struct SettingsPanelState {
    pub(super) subtitle: Option<String>,
    pub(super) rows: Vec<Row>,
    pub(super) sections: Vec<RowSection>,
    pub(super) sources: Vec<SourceState>,
    pub(super) height_cap: f32,
}

enum ActiveControl {
    Edit(String),
    Dropdown(Dropdown),
    ListActions(ListActionMenu),
    Wheel(WheelControl),
}

struct Level {
    rows: Vec<Row>,
    sections: Vec<RowSection>,
    selected: usize,
    active_section: Option<usize>,
    selected_section: usize,
    body_scroll: crate::scroll_list::SelectionScroll,
    active_control: Option<ActiveControl>,
    row_bounds: Vec<Rc<Cell<Option<Bounds<Pixels>>>>>,
    title: Option<String>,
    origin_row: Option<usize>,
    object_array: Option<ObjectArrayState>,
    list_card: bool,
    live_card: bool,
}

struct ListActionMenu {
    row: usize,
    item_id: String,
    dropdown: Dropdown,
    actions: Vec<ResolvedRowAction>,
}

struct WheelControl {
    generation: u64,
    row: usize,
    value: String,
    popup: WindowHandle<ColorWheelPopup>,
}

impl SettingsPanelView {
    pub(super) fn new(
        panel: SettingsPanel,
        state: SettingsPanelState,
        dismisser: SurfaceDismisser,
        custom_factories: Vec<(String, CustomPanelFactory)>,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        let row_bounds = (0..state.rows.len())
            .map(|_| Rc::new(Cell::new(None)))
            .collect();
        let focused_source = panel.focused_index();
        let has_many_sources = state.sources.len() > 1;
        let runtime_queries =
            runtime_query_names(state.rows.iter().filter(|row| row.source == focused_source));
        cx.on_release(|view, cx| view.close_wheel_popup(cx))
            .detach();
        let root = Level {
            rows: state.rows,
            sections: state.sections,
            selected: 0,
            active_section: None,
            selected_section: 0,
            body_scroll: crate::scroll_list::SelectionScroll::new(),
            active_control: None,
            row_bounds,
            title: None,
            origin_row: None,
            object_array: None,
            list_card: false,
            live_card: false,
        };
        let mut view = Self {
            panel,
            runtime: state
                .sources
                .get(focused_source)
                .map(|source| source.runtime.clone())
                .unwrap_or_else(SettingsRuntime::empty),
            runtime_queries,
            streams: state
                .sources
                .iter()
                .map(|source| {
                    super::stream::StreamClient::new(
                        source
                            .daemon_port
                            .map(|port| format!("ws://127.0.0.1:{port}")),
                    )
                })
                .collect(),
            sources: state.sources,
            selected_source: focused_source,
            materialized_source: focused_source,
            source_settle_generation: 0,
            source_menu: has_many_sources,
            rail_transition: TransitionTracker::default(),
            deck_transition: TransitionTracker::default(),
            deck_motion: None,
            subtitle: state.subtitle,
            height_cap: state.height_cap,
            height_revision: 0,
            height_cache: HeightCache::default(),
            save_error: None,
            filter: String::new(),
            filter_open: false,
            stack: vec![root],
            wheel_generation: 0,
            runtime_poll_generation: 0,
            slider_dispatch_generation: 0,
            slider_drag: None,
            slider_pending: std::collections::HashSet::new(),
            frame_paced_samples: None,
            applied_query_payloads: std::collections::HashMap::new(),
            query_states: std::collections::HashMap::new(),
            frame_pump_armed: false,
            motion_tick: None,
            sample_signal: None,
            sampler_stop: None,
            poll_visible: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            dismisser: dismisser.clone(),
            palette: settings_panel_runtime(),
            kit: crate::kit::kit(),
            focus_handle: cx.focus_handle(),
            nav_guard: PhantomNavGuard::new(),
            custom_views: Vec::new(),
            custom_focus_pending: false,
        };
        let parent = cx.weak_entity();
        view.custom_views = view
            .panel
            .sources
            .iter()
            .map(|source| {
                custom_factories
                    .iter()
                    .find(|(plugin_id, _)| plugin_id == &source.plugin_id)
                    .map(|(_, factory)| {
                        let parent = parent.clone();
                        let on_back: CustomPanelCallback = Rc::new(move |window, app| {
                            let _ = parent.update(app, |view, cx| view.custom_back(window, cx));
                        });
                        factory(
                            CustomPanelContext {
                                dismisser: dismisser.clone(),
                                on_back,
                                notify: Rc::clone(&notify),
                            },
                            cx,
                        )
                    })
            })
            .collect();
        let focused_source = view.panel.focused_index();
        let fallback_section = (0..view.level().sections.len())
            .find(|index| !view.section_visible_rows(*index).is_empty());
        let selected_section = (0..view.level().sections.len())
            .find(|index| {
                let section = &view.level().sections[*index];
                section.source == focused_source && !view.section_visible_rows(*index).is_empty()
            })
            .or(fallback_section)
            .unwrap_or(0);
        view.level_mut().selected_section = selected_section;
        let first_visible = view.current_visible_rows().into_iter().next().unwrap_or(0);
        view.level_mut().selected = first_visible;
        if view.panel_names_a_source() {
            view.set_source_menu(false);
            view.open_selected_section();
            view.custom_focus_pending = view.current_source_is_custom();
        }
        view.resume_runtime_poll(cx);
        view
    }

    fn level(&self) -> &Level {
        self.stack.last().expect("stack never empty")
    }

    fn level_mut(&mut self) -> &mut Level {
        self.stack.last_mut().expect("stack never empty")
    }

    fn root(&self) -> &Level {
        self.stack.first().expect("stack never empty")
    }

    fn root_mut(&mut self) -> &mut Level {
        self.stack.first_mut().expect("stack never empty")
    }

    fn slider_rows(&mut self) -> &mut [Row] {
        if self.level().list_card {
            self.root_mut().rows.as_mut_slice()
        } else {
            self.level_mut().rows.as_mut_slice()
        }
    }

    fn source_for(&self, row: usize) -> Option<&SourceState> {
        let source = self.level().rows.get(row)?.source;
        self.sources.get(source)
    }

    fn root_source_for(&self, row: usize) -> Option<&SourceState> {
        let source = self.root().rows.get(row)?.source;
        self.sources.get(source)
    }

    fn stream_for(&self, row: usize) -> Option<&super::stream::StreamClient> {
        let source = self.level().rows.get(row)?.source;
        self.streams.get(source)
    }

    fn current_visible_rows(&self) -> Vec<usize> {
        filtered_visible_rows(
            &self.level().rows,
            &self.level().sections,
            self.sources.len(),
            self.filter_needle().as_deref(),
            self.materialized_source,
        )
    }

    fn filtering(&self) -> bool {
        self.filter_needle().is_some()
    }

    fn filter_needle(&self) -> Option<String> {
        let needle = self.filter.trim().to_lowercase();
        if needle.is_empty() {
            return None;
        }
        Some(needle)
    }

    fn section_filtered_rows(&self, index: usize) -> Vec<usize> {
        let visible = self.section_visible_rows(index);
        let Some(needle) = self.filter_needle() else {
            return visible;
        };
        visible
            .into_iter()
            .filter(|row| row_matches(&self.level().rows[*row], &needle))
            .collect()
    }

    fn set_panel_filter(&mut self, next: String) {
        self.filter = next;
        let visible = self.current_visible_rows();
        let selected = self.level().selected;
        self.level_mut().selected = match self.filtering() {
            true => visible.first().copied().unwrap_or(selected),
            false => clamp_selected(&visible, selected),
        };
        self.sync_scroll();
    }

    fn open_filter(&mut self, seed: Option<String>) {
        self.filter_open = true;
        if self.rail_has_key_focus() {
            self.set_source_menu(false);
        }
        if let Some(seed) = seed {
            self.set_panel_filter(seed);
        }
    }

    fn handle_panel_filter_key(&mut self, key: &str, key_char: Option<&str>) -> bool {
        match key {
            "escape" => {
                self.filter_open = false;
                self.set_panel_filter(String::new());
                if self.sources.len() > 1 {
                    self.set_source_menu(true);
                }
                true
            }
            "enter" | "tab" => {
                self.filter_open = false;
                true
            }
            "backspace" => {
                let mut next = self.filter.clone();
                next.pop();
                self.set_panel_filter(next);
                true
            }
            "up" | "down" => false,
            _ => match key_char.filter(|text| !text.chars().any(char::is_control)) {
                Some(text) => {
                    let next = format!("{}{text}", self.filter);
                    self.set_panel_filter(next);
                    true
                }
                None => false,
            },
        }
    }

    fn fit_lists(&mut self) {
        for (index, visible_items) in list_fit_updates(
            &self.root().rows,
            &self.root().sections,
            self.materialized_source,
            self.height_cap,
        ) {
            if let RowControl::List { list, .. } = &mut self.root_mut().rows[index].control {
                list.max_visible = visible_items;
            }
        }
    }

    fn window_height(&mut self) -> f32 {
        let revision = self.height_revision;
        self.height_cache.value(revision, || {
            let rows = &self.stack[0].rows;
            let sections = &self.stack[0].sections;
            let source_count = self.sources.len();
            let height_cap = self.height_cap;
            (0..source_count.max(1))
                .map(|source| source_window_height_for(rows, sections, source, height_cap))
                .fold(0.0f32, f32::max)
        })
    }

    fn rail_is_open(&self) -> bool {
        rail_open(self.sources.len(), self.filtering())
    }

    fn focus_level(&self) -> PanelFocus {
        focus_level(self.source_menu, self.sources.len())
    }

    fn can_ascend(&self) -> bool {
        !matches!(self.focus_level(), PanelFocus::Sources) && self.sources.len() > 1
    }

    fn ascend(&mut self) -> bool {
        if !self.can_ascend() {
            return false;
        }
        self.set_source_menu(true);
        true
    }

    fn pop_card(&mut self, cx: &mut Context<Self>) {
        if self.stack.len() > 1 && self.level().object_array.is_some() {
            self.sync_object_array_to_root();
        }
        if self.stack.len() > 1 && self.level().live_card {
            self.sync_live_card_to_root();
        }
        if pop_level(&mut self.stack).is_some() {
            self.deck_transition
                .state_changed(true, std::time::Instant::now());
            self.deck_motion = Some(DeckMotion::Pop);
            let visible = self.current_visible_rows();
            let selected = self.level().selected;
            self.level_mut().selected = clamp_selected(&visible, selected);
            self.sync_scroll();
            cx.notify();
        }
    }

    fn rail_has_key_focus(&self) -> bool {
        !matches!(self.focus_level(), PanelFocus::Body)
    }

    fn section_visible_rows(&self, index: usize) -> Vec<usize> {
        let Some(section) = self.level().sections.get(index) else {
            return Vec::new();
        };
        section
            .rows
            .iter()
            .copied()
            .filter(|row| super::rows::row_is_visible(&self.level().rows, *row))
            .collect()
    }

    fn open_selected_section(&mut self) {
        if self.level().sections.is_empty() {
            return;
        }
        let target = self
            .level()
            .selected_section
            .min(self.level().sections.len() - 1);
        let Some(first) = self.section_visible_rows(target).into_iter().next() else {
            return;
        };
        self.level_mut().active_section = Some(target);
        self.level_mut().selected = first;
        self.sync_scroll();
        self.level_mut().active_control = None;
    }

    pub(super) fn retarget_focus(
        &mut self,
        plugin_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(source_index) = self
            .sources
            .iter()
            .position(|source| source.plugin_id == plugin_id)
        else {
            return false;
        };
        self.forget_last_visit();
        self.select_source(source_index, cx);
        self.materialize_source(cx);
        if self.current_source_is_custom() {
            self.set_source_menu(false);
            self.custom_focus_pending = true;
            if let Some(custom) = self.custom_view() {
                (custom.focus)(window, cx);
            }
            cx.notify();
            return true;
        }
        let Some(section_index) = (0..self.level().sections.len()).find(|index| {
            let section = &self.level().sections[*index];
            section.source == source_index && !self.section_visible_rows(*index).is_empty()
        }) else {
            return false;
        };
        self.level_mut().selected_section = section_index;
        if focus_enters_the_body(plugin_id) {
            self.set_source_menu(false);
            self.open_selected_section();
        } else {
            self.set_source_menu(self.sources.len() > 1);
        }
        self.sync_scroll();
        cx.notify();
        true
    }

    fn panel_names_a_source(&self) -> bool {
        let Some(focus) = self.panel.focus.as_deref() else {
            return false;
        };
        focus_enters_the_body(focus) && self.sources.iter().any(|source| source.plugin_id == focus)
    }

    fn current_source_is_custom(&self) -> bool {
        self.panel
            .sources
            .get(self.materialized_source)
            .is_some_and(|source| source.custom)
    }

    fn custom_view(&self) -> Option<&CustomPanelView> {
        self.custom_views
            .get(self.materialized_source)
            .and_then(Option::as_ref)
    }

    fn custom_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.current_source_is_custom() {
            return;
        }
        self.pause_runtime_poll();
        self.set_source_menu(true);
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn forget_last_visit(&mut self) {
        self.stack.truncate(1);
        self.filter.clear();
        self.filter_open = false;
        self.level_mut().active_control = None;
    }

    fn rail_source_level(&self) -> bool {
        matches!(self.focus_level(), PanelFocus::Sources)
    }

    fn step_selected_source(&mut self, direction: isize, cx: &mut Context<Self>) {
        let last = self.sources.len().saturating_sub(1);
        let next = if direction < 0 {
            self.selected_source.saturating_sub(1)
        } else {
            (self.selected_source + 1).min(last)
        };
        if next != self.selected_source {
            self.select_source(next, cx);
        }
    }

    /// Moves the rail highlight and the page together, instantly: a page
    /// build is microseconds, so every keystroke lands on the next frame.
    /// Only the landed source's queries wait for the rail to settle.
    fn select_source(&mut self, next: usize, cx: &mut Context<Self>) {
        if next == self.selected_source {
            return;
        }
        #[cfg(debug_assertions)]
        let switch_started = std::time::Instant::now();
        self.selected_source = next;
        self.materialize_source(cx);
        self.resume_poll_when_settled(cx);
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "SETTINGS_NAV",
            "plugin={} phase=source-switch elapsed_us={}",
            self.sources
                .get(next)
                .map_or("unknown", |source| source.plugin_id.as_str()),
            switch_started.elapsed().as_micros()
        );
    }

    /// Builds the selected source's page: rows, selection, scroll, and the
    /// query list. Pauses the previous source's poller; the caller decides
    /// when the new source's queries start.
    fn materialize_source(&mut self, cx: &mut Context<Self>) {
        let next = self.selected_source;
        if next == self.materialized_source {
            return;
        }
        #[cfg(debug_assertions)]
        let started = std::time::Instant::now();
        self.materialized_source = next;
        self.height_revision += 1;
        self.runtime = self
            .sources
            .get(next)
            .map(|source| source.runtime.clone())
            .unwrap_or_else(SettingsRuntime::empty);
        self.runtime_queries =
            runtime_query_names(self.level().rows.iter().filter(|row| row.source == next));
        self.level_mut().active_control = None;
        let selected_section = (0..self.level().sections.len())
            .find(|index| {
                self.level().sections[*index].source == next
                    && !self.section_visible_rows(*index).is_empty()
            })
            .unwrap_or(0);
        self.level_mut().selected_section = selected_section;
        let first_visible = self.current_visible_rows().into_iter().next().unwrap_or(0);
        self.level_mut().selected = first_visible;
        self.level().body_scroll.rewind();
        self.pause_runtime_poll();
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "SETTINGS_NAV",
            "plugin={} phase=materialize queries={} elapsed_us={}",
            self.sources
                .get(next)
                .map_or("unknown", |source| source.plugin_id.as_str()),
            self.runtime_queries.len(),
            started.elapsed().as_micros()
        );
        cx.notify();
    }

    /// Starts the landed source's queries once the rail stops moving.
    fn resume_poll_when_settled(&mut self, cx: &mut Context<Self>) {
        self.source_settle_generation = self.source_settle_generation.wrapping_add(1);
        let generation = self.source_settle_generation;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx
                    .background_executor()
                    .timer(QUERY_SETTLE_DEBOUNCE)
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if this.source_settle_generation == generation {
                        this.resume_runtime_poll(cx);
                    }
                });
            }
        })
        .detach();
    }

    fn descend_source_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.materialize_source(cx);
        self.source_settle_generation = self.source_settle_generation.wrapping_add(1);
        self.resume_runtime_poll(cx);
        self.set_source_menu(false);
        self.open_selected_section();
        if self.current_source_is_custom() {
            self.custom_focus_pending = false;
            if let Some(custom) = self.custom_view() {
                (custom.focus)(window, cx);
            }
        }
    }

    fn set_source_menu(&mut self, open: bool) {
        let changed = self.source_menu != open;
        self.source_menu = open;
        self.rail_transition
            .state_changed(changed, std::time::Instant::now());
    }

    fn on_rail_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            "up" => self.step_selected_source(-1, cx),
            "down" => self.step_selected_source(1, cx),
            "enter" | "return" => self.descend_source_menu(window, cx),
            "escape" => {
                self.pause_runtime_poll();
                self.dismisser.dismiss(cx);
                return;
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn resume_runtime_poll(&mut self, cx: &mut Context<Self>) {
        let initial_delay = self
            .level()
            .rows
            .iter()
            .any(|row| {
                matches!(&row.control, RowControl::Action { pending: true, .. })
                    || matches!(
                        &row.control,
                        RowControl::List { items, .. }
                            if items.iter().any(|item| item.pending)
                    )
            })
            .then_some(self.runtime.poll_interval);
        self.start_runtime_poll(initial_delay, cx);
    }

    fn start_runtime_poll(
        &mut self,
        initial_delay: Option<std::time::Duration>,
        cx: &mut Context<Self>,
    ) {
        if self.runtime_queries.is_empty() {
            return;
        }
        let source = self.materialized_source;
        let since = std::time::Instant::now();
        for query in &self.runtime_queries {
            self.query_states
                .entry((source, query.clone()))
                .or_insert(RowQueryState::Loading { since });
        }
        self.notify_after_loading_grace(cx);
        self.pause_runtime_poll();
        let generation = self.runtime_poll_generation;
        let runtime = self.runtime.clone();
        let queries = self.runtime_queries.clone();
        let apply_tick = queries
            .iter()
            .map(|query| runtime.query_interval(query))
            .min()
            .unwrap_or(runtime.poll_interval);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let samples = SampledQueryResults::default();
        self.sampler_stop = Some(stop.clone());
        let visible = self.poll_visible.clone();
        self.frame_paced_samples =
            (apply_tick <= FRAME_PACED_QUERY_INTERVAL).then(|| samples.clone());
        let frame_paced = self.frame_paced_samples.is_some();
        if frame_paced {
            let signal = std::sync::Arc::new(SampleSignal::default());
            self.sample_signal = Some(signal.clone());
            let runtime = runtime.clone();
            let queries = queries.clone();
            let samples = samples.clone();
            std::thread::spawn(move || {
                Self::sample_queries_on_demand(runtime, queries, signal, samples)
            });
        }
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                if let Some(initial_delay) = initial_delay {
                    async_cx.background_executor().timer(initial_delay).await;
                }
                if stop.load(std::sync::atomic::Ordering::Relaxed) || frame_paced {
                    return;
                }
                async_cx
                    .background_spawn(Self::sample_queries_at_contract_cadence(
                        runtime,
                        queries,
                        stop.clone(),
                        visible,
                        samples.clone(),
                        async_cx.background_executor().clone(),
                    ))
                    .detach();
                loop {
                    async_cx.background_executor().timer(apply_tick).await;
                    let batch = std::mem::take(&mut *samples.lock().unwrap());
                    let applied = this
                        .update(&mut async_cx, |this, cx| {
                            if this.runtime_poll_generation != generation {
                                return false;
                            }
                            if !batch.is_empty() {
                                for (query, result) in batch {
                                    this.apply_query(&query, result);
                                }
                                cx.notify();
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !applied {
                        break;
                    }
                }
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .detach();
    }

    /// A query that never answers produces no sample and therefore no redraw,
    /// so nothing would ever reveal that the row is waiting. One timer past the
    /// grace period gives the indicator a chance to appear.
    fn notify_after_loading_grace(&mut self, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx
                    .background_executor()
                    .timer(QUERY_LOADING_GRACE)
                    .await;
                let _ = this.update(&mut async_cx, |_, cx| cx.notify());
            }
        })
        .detach();
    }

    fn pump_frame_paced_samples(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.frame_paced_samples.is_none() {
            self.frame_pump_armed = false;
            return;
        }
        self.frame_pump_armed = true;
        let entity = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = entity.update(cx, |this, cx| {
                let changed = this.apply_frame_paced_samples();
                if this.step_gamepad_motion() || changed {
                    cx.notify();
                }
                this.pump_frame_paced_samples(window, cx);
            });
        });
    }

    fn step_gamepad_motion(&mut self) -> bool {
        let now = std::time::Instant::now();
        let dt = self
            .motion_tick
            .map(|tick| now.duration_since(tick).as_secs_f32())
            .unwrap_or_default();
        self.motion_tick = Some(now);
        let mut animating = false;
        if self.stack.len() > 1 {
            for row in &mut self.root_mut().rows {
                if let RowControl::Gamepad { monitor, .. } = &mut row.control {
                    animating |= monitor.step_motion(dt);
                }
            }
        }
        for row in &mut self.level_mut().rows {
            if let RowControl::Gamepad { monitor, .. } = &mut row.control {
                animating |= monitor.step_motion(dt);
            }
        }
        animating
    }

    fn apply_frame_paced_samples(&mut self) -> bool {
        let Some(samples) = self.frame_paced_samples.clone() else {
            return false;
        };
        let batch = std::mem::take(&mut *samples.lock().unwrap_or_else(PoisonError::into_inner));
        if let Some(signal) = &self.sample_signal {
            signal.request();
        }
        let mut changed = false;
        for (query, result) in batch {
            if self.applied_query_payloads.get(&query) == Some(&result) {
                continue;
            }
            self.applied_query_payloads
                .insert(query.clone(), result.clone());
            self.apply_query(&query, result);
            changed = true;
        }
        changed
    }

    fn apply_query(&mut self, query: &str, result: Result<serde_json::Value, String>) {
        self.query_states.insert(
            (self.materialized_source, query.to_string()),
            match &result {
                Ok(_) => RowQueryState::Ready,
                Err(message) => RowQueryState::Unavailable(message.clone()),
            },
        );
        let drag = self.slider_drag.clone();
        let pending = self.slider_pending.clone();
        apply_runtime_query(&mut self.root_mut().rows, query, result, &|index, id| {
            drag.as_ref()
                .is_some_and(|(drag_index, drag_id)| *drag_index == index && drag_id == id)
                || pending.contains(&(index, id.to_string()))
        });
        self.height_revision += 1;
        self.sync_list_card(query);
        self.sync_live_card(query);
    }

    fn sync_list_card(&mut self, query: &str) {
        if self.stack.len() <= 1 || !self.level().list_card {
            return;
        }
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let Some(parent) = self.root().rows.get(origin_row) else {
            return;
        };
        let RowControl::List {
            query: row_query, ..
        } = &parent.control
        else {
            return;
        };
        if row_query.as_str() != query {
            return;
        }
        let (root, front) = self.stack.split_at_mut(1);
        list_card_sync(&mut root[0].rows, front.last_mut().expect("front level"));
        self.height_revision += 1;
    }

    fn sync_live_card(&mut self, query: &str) {
        if self.stack.len() <= 1 || !self.level().live_card {
            return;
        }
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let Some(parent) = self.root().rows.get(origin_row) else {
            return;
        };
        let row_query = match &parent.control {
            RowControl::Gamepad { query, .. } | RowControl::QrCode { query, .. } => query,
            _ => return,
        };
        if row_query != query {
            return;
        }
        let (root, front) = self.stack.split_at_mut(1);
        live_card_sync(&mut root[0].rows, front.last_mut().expect("front level"));
        self.height_revision += 1;
    }

    fn sync_live_card_to_root(&mut self) {
        let (root, front) = self.stack.split_at_mut(1);
        live_card_sync_back(&mut root[0].rows, front.last_mut().expect("front level"));
        self.height_revision += 1;
    }

    pub(super) fn pause_runtime_poll(&mut self) {
        self.runtime_poll_generation = self.runtime_poll_generation.wrapping_add(1);
        if let Some(stop) = self.sampler_stop.take() {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(signal) = self.sample_signal.take() {
            signal.stop();
        }
        self.frame_paced_samples = None;
    }

    fn sample_queries_on_demand(
        runtime: SettingsRuntime,
        queries: Vec<String>,
        signal: std::sync::Arc<SampleSignal>,
        latest: SampledQueryResults,
    ) {
        let intervals = queries
            .iter()
            .map(|query| runtime.query_interval(query))
            .collect::<Vec<_>>();
        let mut due = vec![None::<std::time::Instant>; queries.len()];
        while signal.wait_for_request() {
            let started = std::time::Instant::now();
            let mut fresh = Vec::new();
            for index in due_query_indices(&due, started) {
                let query = &queries[index];
                fresh.push((query.clone(), runtime.query(query)));
                due[index] = Some(started + intervals[index]);
            }
            let mut latest = latest.lock().unwrap_or_else(PoisonError::into_inner);
            for (query, result) in fresh {
                latest.retain(|(name, _)| name != &query);
                latest.push((query, result));
            }
        }
    }

    async fn sample_queries_at_contract_cadence(
        runtime: SettingsRuntime,
        queries: Vec<String>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        visible: std::sync::Arc<std::sync::atomic::AtomicBool>,
        latest: SampledQueryResults,
        executor: BackgroundExecutor,
    ) {
        let intervals = queries
            .iter()
            .map(|query| runtime.query_interval(query))
            .collect::<Vec<_>>();
        let mut due = vec![std::time::Instant::now(); queries.len()];
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            if !visible.load(std::sync::atomic::Ordering::Relaxed) {
                executor.timer(FRAME_PACED_QUERY_INTERVAL).await;
                continue;
            }
            let started = std::time::Instant::now();
            let mut fresh = Vec::new();
            for (index, query) in queries.iter().enumerate() {
                if due[index] <= started {
                    fresh.push((query.clone(), runtime.query(query)));
                    due[index] = started + intervals[index];
                }
            }
            {
                let mut latest = latest.lock().unwrap();
                for (query, result) in fresh {
                    latest.retain(|(name, _)| name != &query);
                    latest.push((query, result));
                }
            }
            let next_due = due.iter().min().copied().unwrap_or(started);
            let wait = next_due.saturating_duration_since(std::time::Instant::now());
            if !wait.is_zero() {
                executor.timer(wait).await;
            }
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let key_char = event.keystroke.key_char.as_deref();
        if self.current_source_is_custom() && self.body_has_focus() {
            return;
        }
        if !event.keystroke.modifiers.modified() && !self.filter_open {
            let editing = matches!(self.level().active_control, Some(ActiveControl::Edit(_)));
            if !editing && self.level().active_control.is_none() {
                if key == "/" || key_char == Some("/") {
                    self.open_filter(None);
                    cx.notify();
                    return;
                }
                if let Some(text) = bare_filter_seed(key, key_char) {
                    self.open_filter(Some(text));
                    cx.notify();
                    return;
                }
            }
        }
        if self.rail_has_key_focus() {
            self.on_rail_key(key, window, cx);
            return;
        }
        if let Some(ActiveControl::Wheel(wheel)) = &self.level().active_control {
            let popup = wheel.popup;
            let _ = popup.update(cx, |popup, popup_window, popup_cx| {
                popup.handle_key(key, event.keystroke.modifiers.shift, popup_window, popup_cx);
            });
            return;
        }
        if matches!(
            self.level().active_control,
            Some(ActiveControl::ListActions(_))
        ) {
            self.on_list_actions_key(key, cx);
            return;
        }
        if matches!(
            self.level().active_control,
            Some(ActiveControl::Dropdown(_))
        ) {
            self.on_dropdown_key(key, cx);
            return;
        }
        if !self.filter_open
            && self.stack.len() > 1
            && self.level().object_array.is_some()
            && self.on_object_array_key(key, key_char, cx)
        {
            return;
        }
        if !self.filter_open
            && self.stack.len() > 1
            && self.level().list_card
            && self.on_list_card_key(key, key_char, cx)
        {
            return;
        }
        if !self.filter_open
            && self.stack.len() > 1
            && self.level().live_card
            && self.on_live_card_key(key, key_char, cx)
        {
            return;
        }
        let editing = matches!(self.level().active_control, Some(ActiveControl::Edit(_)));
        if self.filter_open && self.handle_panel_filter_key(key, key_char) {
            cx.notify();
            return;
        }
        if editing {
            if let Some(direction) = horizontal_step_direction(key) {
                if self.nav_guard.swallow(NavAxis::Horizontal, direction) {
                    return;
                }
                if self.step_number_edit(direction) {
                    cx.notify();
                    return;
                }
            }
            if matches!(key, "up" | "down") {
                self.commit_edit(cx);
            }
        }
        if !editing
            && !self.filter_open
            && self.level().active_control.is_none()
            && matches!(key, "backspace" | "delete")
            && self.remove_selected_text_list_value()
        {
            self.persist();
            cx.notify();
            return;
        }
        let Some(intent) = intent(key, key_char, editing) else {
            return;
        };
        match intent {
            Intent::Up => {
                if self.nav_guard.swallow(NavAxis::Vertical, -1.0) {
                    return;
                }
                let visible = self.current_visible_rows();
                let selected = self.level().selected;
                self.level_mut().selected = adjacent_visible_row(&visible, selected, -1);
                self.sync_scroll();
            }
            Intent::Down => {
                if self.nav_guard.swallow(NavAxis::Vertical, 1.0) {
                    return;
                }
                let visible = self.current_visible_rows();
                let selected = self.level().selected;
                self.level_mut().selected = adjacent_visible_row(&visible, selected, 1);
                self.sync_scroll();
            }
            Intent::Activate => self.activate(window, cx),
            Intent::CommitEdit => self.commit_edit(cx),
            Intent::Backspace => {
                if let Some(ActiveControl::Edit(edit)) = self.level_mut().active_control.as_mut() {
                    edit.pop();
                    self.stream_edit();
                }
            }
            Intent::Insert(ch) => {
                if let Some(ActiveControl::Edit(edit)) = self.level_mut().active_control.as_mut() {
                    edit.push_str(&ch);
                    self.stream_edit();
                }
            }
            Intent::CancelEdit => {
                if row_streams(&self.level().rows, self.level().selected) {
                    if let Some(stream) = self.stream_for(self.level().selected) {
                        stream.close();
                    }
                }
                self.level_mut().active_control = None;
            }
            Intent::Close => {
                match escape_step(self.stack.len() - 1, self.filter_open, self.can_ascend()) {
                    EscapeStep::CloseFilter => {
                        self.handle_panel_filter_key("escape", None);
                    }
                    EscapeStep::PopCard => self.pop_card(cx),
                    EscapeStep::AscendRail => {
                        if self.ascend() {
                            cx.notify();
                            return;
                        }
                    }
                    EscapeStep::Dismiss => {
                        self.pause_runtime_poll();
                        self.dismisser.dismiss(cx);
                        return;
                    }
                }
            }
        }
        cx.notify();
    }

    fn open_color_wheel(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.level().rows.get(index) else {
            return;
        };
        let RowControl::Color(value) = &row.control else {
            return;
        };
        let Some(anchor) = self
            .level()
            .row_bounds
            .get(index)
            .and_then(|bounds| bounds.get())
        else {
            return;
        };
        let value = value.clone();
        self.close_wheel_popup(cx);
        self.level_mut().selected = index;
        if row_streams(&self.level().rows, index) && !stream_gated(&self.level().rows) {
            if let Some(stream) = self.stream_for(index) {
                stream.open();
            }
        }
        self.sync_scroll();
        self.wheel_generation = self.wheel_generation.wrapping_add(1);
        let generation = self.wheel_generation;
        let wheel = ColorWheel::open(&value);
        let preview = wheel.hex();
        let preview_parent = cx.weak_entity();
        let commit_parent = preview_parent.clone();
        let Some(popup) = ColorWheelPopup::open(
            wheel,
            self.wheel_style(),
            anchor,
            window,
            self.focus_handle.clone(),
            WheelCallbacks::new(
                move |value, cx| {
                    let _ = preview_parent.update(cx, |parent, cx| {
                        parent.preview_wheel(generation, value, cx);
                    });
                },
                move |value, cx| {
                    let _ = commit_parent.update(cx, |parent, cx| {
                        parent.commit_wheel(generation, value, cx);
                    });
                },
            ),
            cx,
        ) else {
            return;
        };
        self.level_mut().active_control = Some(ActiveControl::Wheel(WheelControl {
            generation,
            row: index,
            value: preview,
            popup,
        }));
        cx.notify();
    }

    fn preview_wheel(&mut self, generation: u64, value: String, cx: &mut Context<Self>) {
        let row = {
            let Some(ActiveControl::Wheel(wheel)) = self.level_mut().active_control.as_mut() else {
                return;
            };
            if wheel.generation != generation {
                return;
            }
            wheel.row
        };
        if row_streams(&self.level().rows, row) && !stream_gated(&self.level().rows) {
            if let Some(frame) = super::stream::color_frame(&value) {
                if let Some(stream) = self.streams.get(self.level().rows[row].source) {
                    stream.send(frame);
                }
            }
        }
        let Some(ActiveControl::Wheel(wheel)) = self.level_mut().active_control.as_mut() else {
            return;
        };
        wheel.value = value;
        cx.notify();
    }

    fn commit_wheel(&mut self, generation: u64, value: String, cx: &mut Context<Self>) {
        let Some(ActiveControl::Wheel(wheel)) = self.level().active_control.as_ref() else {
            return;
        };
        if wheel.generation != generation {
            return;
        }
        let row_index = wheel.row;
        self.level_mut().active_control = None;
        let streams = row_streams(&self.level().rows, row_index);
        let action = row_action(&self.level().rows, row_index);
        let Some(row) = self.level_mut().rows.get_mut(row_index) else {
            return;
        };
        let RowControl::Color(row_value) = &mut row.control else {
            return;
        };
        *row_value = value;
        if streams {
            if let Some(stream) = self.stream_for(row_index) {
                stream.close();
            }
        }
        self.persist();
        if let Some(action) = action {
            self.dispatch_stream_action(row_index, &action, cx);
        }
        cx.notify();
    }

    fn close_wheel_popup(&mut self, cx: &mut App) {
        if !matches!(self.level().active_control, Some(ActiveControl::Wheel(_))) {
            return;
        }
        let Some(ActiveControl::Wheel(wheel)) = self.level_mut().active_control.take() else {
            return;
        };
        if row_streams(&self.level().rows, wheel.row) {
            if let Some(stream) = self.stream_for(wheel.row) {
                stream.close();
            }
        }
        let _ = wheel
            .popup
            .update(cx, |_, window, _| window.remove_window());
    }

    fn on_dropdown_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(ActiveControl::Dropdown(dropdown)) = self.level_mut().active_control.as_mut()
        else {
            return;
        };
        let Some(event) = dropdown.handle_key(key) else {
            return;
        };
        match event {
            DropdownEvent::Moved => {}
            DropdownEvent::Pick(_) => self.pick_dropdown(),
            DropdownEvent::Close => self.level_mut().active_control = None,
        }
        cx.notify();
    }

    fn on_object_array_key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.on_object_array_card_key(key, key_char, cx)
    }

    fn on_object_array_card_key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let selected = self.level().selected;
        let outcome = match self.level_mut().object_array.as_mut() {
            Some(state) if state.draft.is_some() => state.handle_key(key, key_char),
            Some(state) => {
                if matches!(key, "up" | "down") {
                    return false;
                }
                let count = state.entry_count();
                state.list.selected = selected.min(count - 1);
                state.list.sync(count);
                state.handle_key(key, key_char)
            }
            None => return false,
        };
        match outcome {
            ObjectArrayOutcome::Ignored => return false,
            ObjectArrayOutcome::Handled => {}
            ObjectArrayOutcome::Persist => {
                self.sync_object_array_to_root();
                self.persist();
            }
            ObjectArrayOutcome::Close => {
                self.pop_card(cx);
                return true;
            }
        }
        self.sync_scroll();
        cx.notify();
        true
    }

    fn on_list_card_key(
        &mut self,
        key: &str,
        _key_char: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(direction) = horizontal_step_direction(key) {
            if self.card_list_has_slider() {
                if self.nav_guard.swallow(NavAxis::Horizontal, direction) {
                    return true;
                }
                if self.step_card_list_slider(direction, cx) {
                    cx.notify();
                    return true;
                }
            }
            if key == "right" {
                self.dispatch_list_card_action(cx);
                cx.notify();
                return true;
            }
            return false;
        }
        match key {
            "enter" | "return" | "space" => {
                self.dispatch_list_card_action(cx);
                cx.notify();
                true
            }
            "backspace" | "delete" => true,
            _ => false,
        }
    }

    fn on_live_card_key(
        &mut self,
        key: &str,
        _key_char: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        match key {
            "escape" => {
                self.pop_card(cx);
                true
            }
            "enter" | "return" | "space" => {
                if matches!(
                    self.level()
                        .rows
                        .get(self.level().selected)
                        .map(|row| &row.control),
                    Some(RowControl::Gamepad { .. })
                ) {
                    self.select_next_gamepad();
                }
                cx.notify();
                true
            }
            _ => false,
        }
    }

    fn card_list_has_slider(&self) -> bool {
        let Some(origin_row) = self.level().origin_row else {
            return false;
        };
        matches!(
            self.root().rows.get(origin_row).map(|row| &row.control),
            Some(RowControl::List {
                slider: Some(_),
                ..
            })
        )
    }

    fn on_list_actions_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(ActiveControl::ListActions(menu)) = self.level_mut().active_control.as_mut()
        else {
            return;
        };
        let Some(event) = menu.dropdown.handle_key(key) else {
            return;
        };
        match event {
            DropdownEvent::Moved => {}
            DropdownEvent::Pick(selected) => self.dispatch_list_menu_action(selected, cx),
            DropdownEvent::Close => self.close_list_actions_menu(),
        }
        cx.notify();
    }

    fn close_list_actions_menu(&mut self) {
        self.level_mut().active_control = None;
    }

    fn pick_dropdown(&mut self) {
        let Some(ActiveControl::Dropdown(dropdown)) = self.level().active_control.as_ref() else {
            return;
        };
        let pick = dropdown.selected();
        self.pick_dropdown_option(pick);
    }

    fn pick_dropdown_option(&mut self, pick: usize) {
        let selected = self.level().selected;
        let Some(row) = self.level_mut().rows.get_mut(selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Select { options, index, .. } => {
                if pick < options.len() {
                    *index = pick;
                    self.persist();
                }
                self.level_mut().active_control = None;
            }
            RowControl::MultiSelect { selected, .. } => {
                if let Some(flag) = selected.get_mut(pick) {
                    *flag = !*flag;
                    self.persist();
                }
            }
            RowControl::Toggle(_)
            | RowControl::Number { .. }
            | RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_)
            | RowControl::Action { .. }
            | RowControl::Status { .. }
            | RowControl::List { .. }
            | RowControl::ObjectArray(_)
            | RowControl::Gamepad { .. }
            | RowControl::QrCode { .. }
            | RowControl::Unsupported { .. } => self.level_mut().active_control = None,
        }
    }

    fn toggle(&mut self) {
        let selected = self.level().selected;
        let Some(row) = self.level_mut().rows.get_mut(selected) else {
            return;
        };
        if let RowControl::Toggle(value) = &mut row.control {
            *value = !*value;
            self.persist();
        }
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.current_visible_rows().contains(&self.level().selected) {
            return;
        }
        let selected = self.level().selected;
        if let Some(state) = self.level_mut().object_array.as_mut() {
            let count = state.entry_count();
            state.list.selected = selected.min(count - 1);
            state.list.sync(count);
            state.open_draft();
            return;
        }
        let Some(row) = self.level().rows.get(selected) else {
            return;
        };
        match &row.control {
            RowControl::Toggle(_) => self.toggle(),
            RowControl::Select { options, index, .. } => {
                let count = options.len();
                let initial = *index;
                self.level_mut().active_control =
                    Some(ActiveControl::Dropdown(Dropdown::open(count, initial)));
            }
            RowControl::MultiSelect { options, .. } => {
                let count = options.len();
                self.level_mut().active_control =
                    Some(ActiveControl::Dropdown(Dropdown::open(count, 0)));
            }
            RowControl::Color(_) => self.open_color_wheel(selected, window, cx),
            RowControl::Action { .. } => self.dispatch_action(cx),
            RowControl::Status { .. } => {}
            RowControl::List { items, .. } if !items.is_empty() => {
                self.open_list_card(selected);
                cx.notify();
            }
            RowControl::List { .. } => {}
            RowControl::ObjectArray(_) => {
                self.open_object_array_card(selected);
                cx.notify();
            }
            RowControl::Gamepad { .. } | RowControl::QrCode { .. } => {
                self.open_live_card(selected);
                cx.notify();
            }
            RowControl::Unsupported { .. } => {}
            RowControl::Number { .. } | RowControl::Text(_) => self.begin_edit(),
            RowControl::TextList(_) => {
                self.open_text_list_card(selected);
                cx.notify();
            }
        }
    }

    fn select_next_gamepad(&mut self) {
        if self.level().live_card {
            if let Some(origin_row) = self.level().origin_row {
                if let Some(RowControl::Gamepad { monitor, .. }) = self
                    .root_mut()
                    .rows
                    .get_mut(origin_row)
                    .map(|row| &mut row.control)
                {
                    monitor.select_next();
                }
            }
        }
        let selected = self.level().selected;
        let Some(RowControl::Gamepad { monitor, .. }) = self
            .level_mut()
            .rows
            .get_mut(selected)
            .map(|row| &mut row.control)
        else {
            return;
        };
        monitor.select_next();
    }

    fn dispatch_action(&mut self, cx: &mut Context<Self>) {
        let row_index = self.level().selected;
        let Some(runtime) = self
            .source_for(row_index)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        let Some(RowControl::Action {
            action,
            active_action,
            active_query,
            active_value_from,
            active,
            pending,
            error,
            ..
        }) = self
            .level_mut()
            .rows
            .get_mut(row_index)
            .map(|row| &mut row.control)
        else {
            return;
        };
        if *pending {
            return;
        }
        let action = if *active {
            active_action.as_ref().unwrap_or(action)
        } else {
            action
        }
        .clone();
        *pending = true;
        *error = None;
        let refresh_query = active_query.clone();
        let refresh_value_from = active_value_from.clone();
        #[cfg(debug_assertions)]
        let plugin_id = self.panel.primary_plugin_id().to_string();
        #[cfg(debug_assertions)]
        let dispatched_action = action.clone();
        let rearm_poll_generation = refresh_query.is_some().then(|| {
            self.pause_runtime_poll();
            self.runtime_poll_generation
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move {
                        let result = runtime.run_action(&action, serde_json::Value::Null);
                        let refreshed = if result.is_ok() {
                            refresh_query.map(|query| {
                                let payload =
                                    action_refresh_payload(&result, refresh_value_from.as_deref());
                                let result =
                                    payload.map(Ok).unwrap_or_else(|| runtime.query(&query));
                                (query, result)
                            })
                        } else {
                            None
                        };
                        (result, refreshed)
                    })
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    let (result, refreshed) = result;
                    if let Some(RowControl::Action { pending, error, .. }) = this
                        .level_mut()
                        .rows
                        .get_mut(row_index)
                        .map(|row| &mut row.control)
                    {
                        *pending = false;
                        *error = result.err();
                    }
                    if let Some((query, result)) = refreshed {
                        this.apply_query(&query, result);
                    }
                    #[cfg(debug_assertions)]
                    if let Some(RowControl::Action { active, error, .. }) =
                        this.level().rows.get(row_index).map(|row| &row.control)
                    {
                        qol_runtime::probe!(
                            "SETTINGS_ACTION_STATE",
                            "plugin={} action={} active={} outcome={}",
                            plugin_id,
                            dispatched_action,
                            active,
                            if error.is_some() { "error" } else { "applied" }
                        );
                    }
                    if rearm_poll_generation == Some(this.runtime_poll_generation) {
                        this.start_runtime_poll(Some(this.runtime.poll_interval), cx);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn dispatch_list_card_action(&mut self, cx: &mut Context<Self>) {
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let slot = self.level().selected;
        let Some(RowControl::List {
            actions,
            items,
            filter,
            ..
        }) = self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) = selected_list_item(actions, items, filter, slot) else {
            return;
        };
        let Some(action) = primary_list_item_action(actions, item) else {
            return;
        };
        let item_id = item.id.clone();
        self.dispatch_resolved_list_action(origin_row, &item_id, action, cx);
    }

    fn open_list_card_actions(&mut self) {
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let slot = self.level().selected;
        let Some(RowControl::List {
            actions,
            items,
            filter,
            ..
        }) = self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) =
            selected_list_item(actions, items, filter, slot).filter(|item| !item.pending)
        else {
            return;
        };
        let resolved = list_item_actions(actions, item);
        if resolved.len() < 2 {
            return;
        }
        self.level_mut().active_control = Some(ActiveControl::ListActions(ListActionMenu {
            row: origin_row,
            item_id: item.id.clone(),
            dropdown: Dropdown::open(resolved.len(), 0),
            actions: resolved,
        }));
    }

    fn dispatch_list_menu_action(&mut self, selected: usize, cx: &mut Context<Self>) {
        let Some(ActiveControl::ListActions(menu)) = self.level_mut().active_control.take() else {
            return;
        };
        let row_index = menu.row;
        let item_id = menu.item_id;
        let Some(action) = menu.actions.into_iter().nth(selected) else {
            self.close_list_actions_menu();
            return;
        };
        self.close_list_actions_menu();
        self.dispatch_resolved_list_action(row_index, &item_id, action, cx);
    }

    fn dispatch_resolved_list_action(
        &mut self,
        row_index: usize,
        item_id: &str,
        action: ResolvedRowAction,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self
            .root_source_for(row_index)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        let Some(RowControl::List { actions, items, .. }) = self
            .root_mut()
            .rows
            .get_mut(row_index)
            .map(|row| &mut row.control)
        else {
            return;
        };
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        if !list_item_actions(actions, item).contains(&action) {
            return;
        }
        let Some(action) = begin_list_item_action(item, action) else {
            return;
        };
        let item_id = item.id.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(
                        async move { runtime.run_action(&action.action, action.input) },
                    )
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if let Err(error) = result {
                        this.set_list_action_error(row_index, &item_id, error);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_list_action_error(&mut self, row_index: usize, item_id: &str, error: String) {
        let Some(RowControl::List { items, .. }) = self
            .root_mut()
            .rows
            .get_mut(row_index)
            .map(|row| &mut row.control)
        else {
            return;
        };
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        item.pending = false;
        item.error = Some(error);
    }

    fn step_card_list_slider(&mut self, direction: f64, cx: &mut Context<Self>) -> bool {
        let Some(origin_row) = self.level().origin_row else {
            return false;
        };
        let slot = self.level().selected;
        let Some(RowControl::List {
            actions,
            slider,
            items,
            filter,
            ..
        }) = self
            .root_mut()
            .rows
            .get_mut(origin_row)
            .map(|row| &mut row.control)
        else {
            return false;
        };
        let Some(slider) = slider else {
            return false;
        };
        let Some(item) = selected_list_item(actions, items, filter, slot) else {
            return false;
        };
        step_list_slider(slider, item, direction);
        let item_id = item.id.clone();
        self.schedule_slider_dispatch(origin_row, &item_id, cx);
        true
    }

    fn set_list_slider_value(
        &mut self,
        row_index: usize,
        item_id: &str,
        fraction: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(RowControl::List { slider, items, .. }) = self
            .slider_rows()
            .get_mut(row_index)
            .map(|row| &mut row.control)
        else {
            return;
        };
        let Some(slider) = slider else {
            return;
        };
        if !items.iter().any(|item| item.id == item_id) {
            return;
        }
        let value = slider_value_from_fraction(
            slider.spec.min,
            slider.spec.max,
            slider.spec.step,
            fraction,
        );
        slider.values.insert(
            item_id.to_string(),
            SliderHold {
                value,
                dispatched: None,
                until: std::time::Instant::now() + SLIDER_HOLD_DURATION,
            },
        );
        self.schedule_slider_dispatch(row_index, item_id, cx);
    }

    fn schedule_slider_dispatch(
        &mut self,
        row_index: usize,
        item_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.slider_dispatch_generation += 1;
        let generation = self.slider_dispatch_generation;
        self.slider_pending.clear();
        self.slider_pending.insert((row_index, item_id.to_string()));
        let item_id = item_id.to_string();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx
                    .background_executor()
                    .timer(SLIDER_DISPATCH_DEBOUNCE)
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if this.slider_dispatch_generation != generation {
                        return;
                    }
                    this.dispatch_list_slider(row_index, &item_id, cx);
                });
            }
        })
        .detach();
    }

    fn dispatch_list_slider(&mut self, row_index: usize, item_id: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self
            .root_source_for(row_index)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        self.slider_pending
            .remove(&(row_index, item_id.to_string()));
        let Some((action, item_id)) =
            resolve_list_slider_dispatch(self.slider_rows(), row_index, item_id)
        else {
            return;
        };
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(
                        async move { runtime.run_action(&action.action, action.input) },
                    )
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if let Err(error) = result {
                        this.set_list_action_error(row_index, &item_id, error);
                        super::rows::clear_slider_hold(this.slider_rows(), row_index, &item_id);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn begin_edit(&mut self) {
        let Some(row) = self.level().rows.get(self.level().selected) else {
            return;
        };
        let edit = match &row.control {
            RowControl::Text(value) => value.clone(),
            RowControl::Number { value, .. } => format_number(*value),
            RowControl::Toggle(_)
            | RowControl::TextList(_)
            | RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Color(_)
            | RowControl::Action { .. }
            | RowControl::Status { .. }
            | RowControl::List { .. }
            | RowControl::ObjectArray(_)
            | RowControl::Gamepad { .. }
            | RowControl::QrCode { .. }
            | RowControl::Unsupported { .. } => return,
        };
        self.level_mut().active_control = Some(ActiveControl::Edit(edit));
        if row_streams(&self.level().rows, self.level().selected)
            && !stream_gated(&self.level().rows)
        {
            if let Some(stream) = self.stream_for(self.level().selected) {
                stream.open();
            }
        }
    }

    fn step_number_edit(&mut self, direction: f64) -> bool {
        let Some(RowControl::Number {
            value,
            min,
            max,
            step,
        }) = self
            .level()
            .rows
            .get(self.level().selected)
            .map(|row| &row.control)
        else {
            return false;
        };
        let (value, min, max, step) = (*value, *min, *max, *step);
        let Some(ActiveControl::Edit(edit)) = self.level_mut().active_control.as_mut() else {
            return false;
        };
        *edit = stepped_number(edit, value, min, max, step.unwrap_or(1.0), direction);
        self.stream_edit();
        true
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(ActiveControl::Edit(edit)) = self.level_mut().active_control.take() else {
            return;
        };
        let streams = row_streams(&self.level().rows, self.level().selected);
        let action = row_action(&self.level().rows, self.level().selected);
        let selected = self.level().selected;
        let Some(row) = self.level_mut().rows.get_mut(selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Text(value) => *value = edit,
            RowControl::Number {
                value,
                min,
                max,
                step,
                ..
            } => {
                let Some(parsed) = parsed_number(&edit, *min, *max, *step) else {
                    return;
                };
                *value = parsed;
                if streams {
                    if let Some(stream) = self.stream_for(self.level().selected) {
                        stream.close();
                    }
                }
            }
            RowControl::Toggle(_)
            | RowControl::TextList(_)
            | RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Color(_)
            | RowControl::Action { .. }
            | RowControl::Status { .. }
            | RowControl::List { .. }
            | RowControl::ObjectArray(_)
            | RowControl::Gamepad { .. }
            | RowControl::QrCode { .. }
            | RowControl::Unsupported { .. } => return,
        }
        self.sync_text_list_to_root();
        self.persist();
        if let Some(action) = action {
            self.dispatch_stream_action(self.level().selected, &action, cx);
        }
        cx.notify();
    }

    fn push_card(&mut self, child: Level) {
        self.deck_transition
            .state_changed(true, std::time::Instant::now());
        self.deck_motion = Some(DeckMotion::Push);
        push_level(&mut self.stack, child);
    }

    fn open_text_list_card(&mut self, row_index: usize) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::TextList(values) = &row.control else {
            return;
        };
        let label = row.label.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        let values = values.clone();
        let rows = text_list_child_rows(&label, &config_key, source, &values);
        let section = RowSection {
            label: label.clone(),
            description: None,
            rows: (0..rows.len()).collect(),
            source,
        };
        let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
        let child = Level {
            rows,
            sections: vec![section],
            selected: initial_card_selection(values.len()),
            active_section: None,
            selected_section: 0,
            body_scroll: crate::scroll_list::SelectionScroll::new(),
            active_control: None,
            row_bounds,
            title: Some(label),
            origin_row: Some(row_index),
            object_array: None,
            list_card: false,
            live_card: false,
        };
        self.push_card(child);
        self.sync_scroll();
    }

    fn open_list_card(&mut self, row_index: usize) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::List {
            actions,
            items,
            filter,
            list,
            ..
        } = &row.control
        else {
            return;
        };
        let label = row.label.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        let child = list_card_level(
            ListCardOrigin {
                label: &label,
                config_key: &config_key,
                source,
                row: row_index,
            },
            actions,
            items,
            filter,
            list.selected,
        );
        self.push_card(child);
        self.sync_scroll();
    }

    fn open_object_array_card(&mut self, row_index: usize) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::ObjectArray(state) = &row.control else {
            return;
        };
        let label = row.label.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        let child = ObjectArrayState::from_entries(
            state.key_label.clone(),
            state.schema.clone(),
            state.entries.clone(),
        );
        let level = object_array_card_level(&label, &config_key, source, child, row_index);
        self.push_card(level);
        self.sync_scroll();
    }

    fn open_live_card(&mut self, row_index: usize) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let control = match &row.control {
            RowControl::Gamepad { query, monitor } => RowControl::Gamepad {
                query: query.clone(),
                monitor: monitor.clone(),
            },
            RowControl::QrCode {
                query,
                value_from,
                url,
                modules,
                error,
            } => RowControl::QrCode {
                query: query.clone(),
                value_from: value_from.clone(),
                url: url.clone(),
                modules: modules.clone(),
                error: error.clone(),
            },
            _ => return,
        };
        let label = row.label.clone();
        let description = row.description.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        let child = live_card_level(&label, description, &config_key, source, control, row_index);
        self.push_card(child);
        self.sync_scroll();
    }

    fn sync_text_list_to_root(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let add_value = self
            .level()
            .rows
            .first()
            .and_then(|row| match &row.control {
                RowControl::Text(value) if !value.is_empty() => Some(value.clone()),
                _ => None,
            });
        let mut values = self
            .level()
            .rows
            .iter()
            .skip(1)
            .filter_map(|row| match &row.control {
                RowControl::Text(value) => Some(value.clone()),
                _ => None,
            })
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let added = add_value.is_some();
        if let Some(value) = add_value {
            values.push(value);
        }
        let Some(row) = self.root().rows.get(origin_row) else {
            return;
        };
        let label = row.label.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        if let Some(RowControl::TextList(stored)) = self
            .root_mut()
            .rows
            .get_mut(origin_row)
            .map(|row| &mut row.control)
        {
            *stored = values.clone();
            self.height_revision += 1;
        }
        let rows = text_list_child_rows(&label, &config_key, source, &values);
        let section = RowSection {
            label: label.clone(),
            description: None,
            rows: (0..rows.len()).collect(),
            source,
        };
        let level = self.level_mut();
        level.rows = rows;
        level.sections = vec![section];
        level.row_bounds = (0..level.rows.len())
            .map(|_| Rc::new(Cell::new(None)))
            .collect();
        level.selected = if added {
            level.rows.len() - 1
        } else {
            level.selected.min(level.rows.len() - 1)
        };
    }

    fn sync_object_array_to_root(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let (root, front) = self.stack.split_at_mut(1);
        object_array_card_sync(&mut root[0].rows, front.last_mut().expect("front level"));
        self.height_revision += 1;
    }

    fn remove_selected_text_list_value(&mut self) -> bool {
        if self.stack.len() <= 1 || self.level().origin_row.is_none() {
            return false;
        }
        let selected = self.level().selected;
        if selected == 0 {
            return false;
        }
        let Some(RowControl::Text(value)) = self
            .level_mut()
            .rows
            .get_mut(selected)
            .map(|row| &mut row.control)
        else {
            return false;
        };
        *value = String::new();
        self.sync_text_list_to_root();
        true
    }

    fn stream_edit(&mut self) {
        let Some(ActiveControl::Edit(edit)) = self.level().active_control.as_ref() else {
            return;
        };
        if !row_streams(&self.level().rows, self.level().selected)
            || stream_gated(&self.level().rows)
        {
            return;
        }
        let Some(RowControl::Number { min, max, step, .. }) = self
            .level()
            .rows
            .get(self.level().selected)
            .map(|row| &row.control)
        else {
            return;
        };
        let Some(value) = parsed_number(edit, *min, *max, *step) else {
            return;
        };
        let level = value.clamp(0.0, 255.0) as u8;
        let hex = self.stream_hex();
        if let Some(stream) = self.stream_for(self.level().selected) {
            if let Some(frame) = super::stream::brightness_frame(level, &hex) {
                stream.send(frame);
            }
        }
    }

    fn stream_hex(&self) -> String {
        let Some(source) = self.source_for(self.level().selected) else {
            return format!("{:06x}", self.palette.live_color_fallback);
        };
        source
            .values
            .get("live_color_hex")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{:06x}", self.palette.live_color_fallback))
    }

    fn dispatch_stream_action(&self, row: usize, action: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self.source_for(row).map(|source| source.runtime.clone()) else {
            return;
        };
        let action = action.to_string();
        cx.spawn(move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let async_cx = cx.clone();
            async move {
                let _ = async_cx
                    .background_spawn(async move {
                        runtime.run_action(&action, serde_json::Value::Null)
                    })
                    .await;
            }
        })
        .detach();
    }

    fn persist(&mut self) {
        self.height_revision += 1;
        let Some(source_index) = self
            .root()
            .rows
            .get(self.root().selected)
            .map(|row| row.source)
        else {
            return;
        };
        let Some(source) = self.sources.get(source_index) else {
            return;
        };
        let previous_theme = (
            source.values.get("native_theme").cloned(),
            source.values.get("accent").cloned(),
        );
        let rows = self
            .root()
            .rows
            .iter()
            .filter(|row| row.source == source_index);
        let merged = merged_config(&source.values, rows);
        let Some(source) = self.sources.get_mut(source_index) else {
            return;
        };
        source.values = merged;
        self.save_error = save_values(
            &panel_base(&source.plugin_id),
            source.path.as_deref(),
            &source.values,
        )
        .err();
        if source.plugin_id == qol_conventions::CORE_PANEL_ID
            && (source.values.get("native_theme") != previous_theme.0.as_ref()
                || source.values.get("accent") != previous_theme.1.as_ref())
        {
            let (native, accent) = theme_override_values(&source.values);
            qol_theme::set_runtime_theme_override(native, accent);
        }
    }

    fn sync_scroll(&mut self) {
        let selected = self.level().selected;
        let child = self.body_child_index(selected);
        self.level().body_scroll.follow(child);
    }

    fn body_child_index(&self, row: usize) -> Option<usize> {
        let groups = self.body_groups();
        let headers = groups
            .iter()
            .map(|(title, rows)| {
                let labels = rows
                    .iter()
                    .map(|index| self.level().rows[*index].label.as_str())
                    .collect::<Vec<_>>();
                !header_is_redundant(title, &labels)
            })
            .collect::<Vec<_>>();
        let sections = groups.into_iter().map(|(_, rows)| rows).collect::<Vec<_>>();
        body_child_offset(&sections, &headers, row)
    }

    fn display_value(&self, index: usize) -> String {
        if index == self.level().selected {
            match &self.level().active_control {
                Some(ActiveControl::Edit(edit)) => return format!("{edit}_"),
                Some(ActiveControl::Wheel(wheel)) => return wheel.value.clone(),
                Some(ActiveControl::Dropdown(_)) | Some(ActiveControl::ListActions(_)) | None => {}
            }
        }
        match &self.level().rows[index].control {
            RowControl::Toggle(value) => binary_state_label(*value).into(),
            RowControl::Select { options, index, .. } => options
                .get(*index)
                .map(|option| option.label.clone())
                .unwrap_or_default(),
            RowControl::MultiSelect {
                options, selected, ..
            } => {
                let chosen: Vec<&str> = options
                    .iter()
                    .zip(selected)
                    .filter(|(_, on)| **on)
                    .map(|(option, _)| option.label.as_str())
                    .collect();
                if chosen.is_empty() {
                    "none".into()
                } else {
                    chosen.join(", ")
                }
            }
            RowControl::Number { value, .. } => format_number(*value),
            RowControl::Text(value) => {
                text_or_placeholder(value, self.level().rows[index].placeholder.as_deref())
            }
            RowControl::TextList(values) => text_or_placeholder(
                &values.join(", "),
                self.level().rows[index].placeholder.as_deref(),
            ),
            RowControl::Color(value) => color_display(value),
            RowControl::Action {
                active_action,
                active_query,
                state_labels,
                active,
                pending,
                error,
                ..
            } => action_value_label(
                *active,
                *pending,
                error.is_some(),
                active_query.is_some(),
                active_action.is_some(),
                state_labels,
            ),
            RowControl::Status { label, error, .. } => {
                if error.is_some() {
                    "unavailable".into()
                } else {
                    label.clone().unwrap_or_else(|| "loading\u{2026}".into())
                }
            }
            RowControl::List {
                actions,
                items,
                filter,
                error,
                ..
            } => {
                if error.is_some() {
                    "unavailable".into()
                } else if filter.trim().is_empty() {
                    format!("{} found", items.len())
                } else {
                    let visible = filtered_list_items(actions, items, filter).len();
                    format!("{visible}/{}", items.len())
                }
            }
            RowControl::ObjectArray(state) => item_count_label(state.entries.len()),
            RowControl::Gamepad { monitor, .. } => monitor
                .selected()
                .map(|controller| controller.name.clone())
                .unwrap_or_else(|| "Waiting".into()),
            RowControl::QrCode { url, .. } => url.clone().unwrap_or_default(),
            RowControl::Unsupported { reason, .. } => reason.clone(),
        }
    }

    fn value_color(&self, index: usize) -> u32 {
        if index == self.level().selected
            && matches!(self.level().active_control, Some(ActiveControl::Edit(_)))
        {
            return self.palette.label_text;
        }
        match &self.level().rows[index].control {
            RowControl::Toggle(value) => binary_state_color(self.palette, *value),
            RowControl::Select { .. }
            | RowControl::MultiSelect { .. }
            | RowControl::Number { .. }
            | RowControl::Color(_) => self.palette.label_text,
            RowControl::Text(value) if value.is_empty() => self.palette.status_muted,
            RowControl::TextList(values) if values.is_empty() => self.palette.status_muted,
            RowControl::ObjectArray(state) if state.entries.is_empty() => self.palette.status_muted,
            RowControl::Text(_) | RowControl::TextList(_) | RowControl::ObjectArray(_) => {
                self.palette.label_text
            }
            RowControl::Action { error: Some(_), .. }
            | RowControl::Status { error: Some(_), .. }
            | RowControl::List { error: Some(_), .. } => self.palette.state_off,
            RowControl::Action { state_labels, .. } if !state_labels.is_empty() => {
                self.palette.label_text
            }
            RowControl::Action {
                active_query: None, ..
            } => self.palette.state_on,
            RowControl::Action { active, .. } => binary_state_color(self.palette, *active),
            RowControl::Status { tone, .. } => status_tone_color(self.palette, *tone),
            RowControl::List { .. } | RowControl::Gamepad { .. } => self.palette.label_text,
            RowControl::QrCode { .. } => self.palette.label_text,
            RowControl::Unsupported { .. } => self.palette.status_muted,
        }
    }

    fn dropdown_style(&self) -> DropdownStyle {
        super::components::settings_dropdown_style(self.palette)
    }

    fn wheel_style(&self) -> WheelStyle {
        WheelStyle {
            bg: self.palette.dropdown_bg,
            border: self.palette.row_border_selected,
            thumb_border: self.palette.section_text,
        }
    }

    fn body_has_focus(&self) -> bool {
        matches!(self.focus_level(), PanelFocus::Body)
    }

    fn mark_selected<E: Styled + ParentElement>(&self, row: E, selected: bool) -> E {
        if !selected || !self.body_has_focus() {
            return row;
        }
        self.paint_body_selection(row)
    }

    fn paint_selection<E: Styled + ParentElement>(&self, row: E) -> E {
        self.kit.row_state(row, crate::kit::RowState::Current)
    }

    fn paint_body_selection<E: Styled + ParentElement>(&self, row: E) -> E {
        paint_settings_selection(row, self.palette)
    }

    /// The freshness of a row's value: unavailable wins over loading, and a row
    /// with no runtime query is always idle.
    fn row_query_state(&self, index: usize) -> RowQueryState {
        let row = &self.level().rows[index];
        rollup_query_state(
            row_query_names(row)
                .into_iter()
                .map(|query| self.query_states.get(&(row.source, query.to_string()))),
            QUERY_LOADING_GRACE,
            std::time::Instant::now(),
        )
    }

    /// Replaces a query-backed value with a spinner or an unavailable marker
    /// while its plugin has not answered. Status rows are excluded: they carry
    /// their own tone and error text.
    fn render_query_state_cell(&self, index: usize) -> Option<Div> {
        if matches!(self.level().rows[index].control, RowControl::Status { .. }) {
            return None;
        }
        let cell = || {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(qol_theme::SPACE_INSET))
                .flex_none()
                .w(px(value_cell_width(&self.level().rows[index].control)))
                .justify_end()
        };
        match self.row_query_state(index) {
            RowQueryState::Loading { .. } => Some(cell().child(Spinner::new(
                ("settings-query-spinner", index),
                rgb(self.palette.status_muted),
            ))),
            RowQueryState::Unavailable(_) => Some(
                cell().child(
                    div()
                        .text_size(px(qol_theme::TEXT_CAPTION))
                        .text_color(rgb(self.palette.status_muted))
                        .child("unavailable"),
                ),
            ),
            RowQueryState::Idle | RowQueryState::Ready => None,
        }
    }

    fn render_value_cell(&self, index: usize) -> Div {
        if let Some(cell) = self.render_query_state_cell(index) {
            return cell;
        }
        match &self.level().rows[index].control {
            RowControl::Toggle(active) => return self.render_toggle_value(*active),
            RowControl::Select { .. } | RowControl::MultiSelect { .. } => {
                return self.render_select_value(index);
            }
            RowControl::Number {
                value,
                min,
                max,
                step,
                ..
            } => return self.render_number_value(index, *value, *min, *max, *step),
            RowControl::Action { active, .. }
                if self.level().rows[index].variant.as_deref() == Some("toggle") =>
            {
                return self.render_toggle_value(*active);
            }
            RowControl::Action { .. } => return self.render_action_value(index),
            RowControl::TextList(values) => {
                return self
                    .kit
                    .count_chip(values.len(), plural(values.len(), "item"));
            }
            RowControl::Unsupported { reason, .. } => {
                return div()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.status_muted))
                    .child(format!("Unsupported: {reason}"));
            }
            RowControl::Text(_)
            | RowControl::Color(_)
            | RowControl::Status { .. }
            | RowControl::List { .. }
            | RowControl::ObjectArray(_)
            | RowControl::Gamepad { .. }
            | RowControl::QrCode { .. } => {}
        }
        let mut cell = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET));
        if let RowControl::Status { tone, .. } = self.level().rows[index].control {
            return cell.child(StatusIndicator::new(
                ("settings-status", index),
                self.display_value(index),
                rgb(status_tone_color(self.palette, tone)),
            ));
        }
        if self.action_is_busy(index) {
            cell = cell.child(Spinner::new(
                ("settings-action-spinner", index),
                rgb(self.palette.state_on),
            ));
        }
        if let Some(color) = self.swatch_color(index) {
            cell = cell.child(
                div()
                    .w_3()
                    .h_3()
                    .rounded(px(qol_theme::RADIUS_TIGHT))
                    .bg(rgb(color)),
            );
        }
        if let Some(accent) = self.option_accent(index) {
            cell = cell.child(div().w_2().h_2().rounded_full().bg(rgb(accent)));
        }
        cell.flex_none()
            .w(px(value_cell_width(&self.level().rows[index].control)))
            .justify_end()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(self.value_color(index)))
                    .child(self.display_value(index)),
            )
    }

    fn render_toggle_value(&self, active: bool) -> Div {
        div().child(SettingsToggle::new(active, self.palette))
    }

    fn render_select_value(&self, index: usize) -> Div {
        div().child(
            SettingsSelectValue::new(self.display_value(index), self.palette)
                .accent(self.option_accent(index)),
        )
    }

    fn render_number_value(
        &self,
        index: usize,
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    ) -> Div {
        let mut cell = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET));
        if self.level().rows[index].variant.as_deref() == Some("slider") {
            let edit = if index == self.level().selected {
                match &self.level().active_control {
                    Some(ActiveControl::Edit(edit)) => Some(edit.as_str()),
                    _ => None,
                }
            } else {
                None
            };
            let fill =
                slider_fraction(number_preview(edit, value, min, max, step), min, max) * 72.0;
            cell = cell.child(
                div()
                    .relative()
                    .w(px(72.))
                    .h(px(4.))
                    .rounded_full()
                    .overflow_hidden()
                    .bg(rgb(self.palette.panel_border))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(px(fill))
                            .rounded_full()
                            .bg(rgb(self.palette.row_border_selected)),
                    ),
            );
        }
        cell.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(qol_theme::SPACE_TIGHT))
                .px(px(qol_theme::SPACE_INSET))
                .py(px(qol_theme::SPACE_TIGHT))
                .rounded(px(qol_theme::RADIUS_CONTROL))
                .bg(rgb(self.palette.dropdown_bg))
                .text_size(px(qol_theme::TEXT_BODY))
                .text_color(rgb(self.palette.label_text))
                .child(self.display_value(index))
                .children(number_unit(&self.level().rows[index].id).map(|unit| {
                    div()
                        .text_size(px(qol_theme::TEXT_CAPTION))
                        .text_color(rgb(self.palette.status_muted))
                        .child(unit)
                })),
        )
    }

    fn render_action_value(&self, index: usize) -> Div {
        let variant = self.level().rows[index].variant.as_deref();
        let (background, text) = match variant {
            Some("ghost") => (rgb(self.palette.dropdown_bg), self.palette.label_text),
            Some("danger") => (
                rgba(qol_color::with_alpha(self.palette.state_off, 0x29)),
                self.palette.state_off,
            ),
            Some("primary") | None | Some(_) => {
                (rgb(self.palette.row_bg_selected), self.palette.section_text)
            }
        };
        let mut control = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_TIGHT));
        if self.action_is_busy(index) {
            control = control.child(
                Spinner::new(
                    ("settings-action-spinner", index),
                    rgb(self.palette.state_on),
                )
                .size(px(12.)),
            );
        }
        control
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .when(variant == Some("ghost"), |control| {
                control.shadow(crate::kit::raised_shadow(self.palette.section_text))
            })
            .bg(background)
            .text_size(px(qol_theme::TEXT_CAPTION))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(text))
            .child(self.display_value(index))
    }

    fn action_is_busy(&self, index: usize) -> bool {
        action_shows_spinner(&self.level().rows[index])
    }

    fn swatch_color(&self, index: usize) -> Option<u32> {
        let RowControl::Color(value) = &self.level().rows[index].control else {
            return None;
        };
        let text = if index == self.level().selected {
            match &self.level().active_control {
                Some(ActiveControl::Edit(edit)) => edit,
                Some(ActiveControl::Wheel(wheel)) => return parsed_color(&wheel.value),
                Some(ActiveControl::Dropdown(_)) | Some(ActiveControl::ListActions(_)) | None => {
                    value
                }
            }
        } else {
            value
        };
        parsed_color(text)
    }

    fn option_accent(&self, index: usize) -> Option<u32> {
        let RowControl::Select { options, index, .. } = &self.level().rows[index].control else {
            return None;
        };
        options.get(*index)?.accent
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let row = &self.level().rows[index];
        let mut container = div().flex().flex_col().gap(px(qol_theme::SPACE_TIGHT));

        if self.level().list_card {
            return container.child(self.render_list_card_item(index, cx));
        }
        if matches!(row.control, RowControl::List { .. }) {
            return container.child(self.render_list(index));
        }
        if matches!(row.control, RowControl::ObjectArray(_)) {
            return container.child(self.render_object_array(index, cx));
        }
        if matches!(row.control, RowControl::QrCode { .. }) && self.level().live_card {
            return container.child(self.render_qr_code(index));
        }
        if matches!(row.control, RowControl::Gamepad { .. }) && self.level().live_card {
            return container.child(self.render_gamepad(index, cx));
        }
        let label = match &row.control {
            RowControl::Action {
                active: true,
                active_label: Some(label),
                ..
            } => label.clone(),
            _ => row.label.clone(),
        };
        let card_chips = self.object_array_card_chips(index);
        let has_chips = card_chips.is_some();
        let label_group = match card_chips {
            Some(chips) => div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(qol_theme::SPACE_TIGHT))
                .min_w_0()
                .flex_1()
                .child(self.render_chip_row(chip_row_parts(&chips))),
            None => settings_label_group(
                label,
                row.description.clone().map(SharedString::from),
                self.palette,
            ),
        };
        let selected = index == self.level().selected;
        let mut value_cell = if has_chips {
            None
        } else {
            Some(self.render_value_cell(index))
        };
        if selected {
            if let Some(ActiveControl::Dropdown(dropdown)) = &self.level().active_control {
                let items = match &row.control {
                    RowControl::Select { options, .. } => Some(dropdown_items(options)),
                    RowControl::MultiSelect {
                        options, selected, ..
                    } => Some(
                        options
                            .iter()
                            .zip(selected)
                            .map(|(option, on)| DropdownItem {
                                label: format!(
                                    "{} {}",
                                    if *on { "[x]" } else { "[ ]" },
                                    option.label
                                ),
                                accent: option.accent,
                            })
                            .collect::<Vec<_>>(),
                    ),
                    RowControl::Toggle(_)
                    | RowControl::Number { .. }
                    | RowControl::Text(_)
                    | RowControl::TextList(_)
                    | RowControl::Color(_)
                    | RowControl::Action { .. }
                    | RowControl::Status { .. }
                    | RowControl::List { .. }
                    | RowControl::ObjectArray(_)
                    | RowControl::Gamepad { .. }
                    | RowControl::QrCode { .. }
                    | RowControl::Unsupported { .. } => None,
                };
                if let Some(items) = items {
                    let view = cx.weak_entity();
                    let menu = dropdown.render_items_clickable(
                        format!("settings-options-{index}"),
                        &items,
                        self.dropdown_style(),
                        move |selected, event, _, cx| {
                            if !event.standard_click() {
                                return;
                            }
                            cx.stop_propagation();
                            let view = view.clone();
                            cx.defer(move |cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.pick_dropdown_option(selected);
                                    cx.notify();
                                });
                            });
                        },
                    );
                    value_cell = value_cell.map(|cell| div().relative().child(menu).child(cell));
                }
            }
        }
        let row_bounds = Rc::clone(&self.level().row_bounds[index]);
        let bounds = canvas(
            move |bounds, _, _| row_bounds.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let mut line = if self.level().object_array.is_some() {
            SettingsRow::rule(("settings-row", index), self.palette)
        } else {
            SettingsRow::setting(("settings-row", index), self.palette)
        }
        .selected(selected, self.body_has_focus())
        .child(label_group)
        .child(bounds);
        if !matches!(
            row.control,
            RowControl::Status { .. } | RowControl::List { .. } | RowControl::Unsupported { .. }
        ) {
            line = line.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if !event.standard_click() {
                    return;
                }
                this.level_mut().selected = index;
                this.activate(window, cx);
                cx.notify();
            }));
        }
        if let Some(cell) = value_cell {
            line = line.child(cell);
        }
        container = container.child(line);
        let error = match &row.control {
            RowControl::Action {
                error: Some(error), ..
            }
            | RowControl::Status {
                error: Some(error), ..
            } => Some(error),
            _ => None,
        };
        if let Some(error) = error {
            container = container.child(
                div()
                    .px(px(qol_theme::SPACE_INSET))
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.state_off))
                    .child(error.clone()),
            );
        }
        container
    }

    fn render_gamepad(&self, index: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let row = &self.level().rows[index];
        let RowControl::Gamepad { monitor, .. } = &row.control else {
            return div().id(("settings-gamepad-empty", index));
        };
        let selected = index == self.level().selected;
        let palette = GamepadPalette {
            surface: self.palette.window_bg,
            raised: self.palette.dropdown_bg,
            border: self.palette.panel_border,
            text: self.palette.section_text,
            text_muted: self.palette.label_text,
            accent: self.palette.row_border_selected,
            info: self.palette.status_info,
            success: self.palette.status_success,
            warning: self.palette.status_warning,
            danger: self.palette.status_danger,
        };
        div()
            .id(("settings-gamepad", index))
            .h(px(super::PANEL_GAMEPAD_HEIGHT))
            .rounded(px(qol_theme::RADIUS_CARD))
            .border_1()
            .border_color(if selected {
                rgb(self.palette.row_border_selected)
            } else {
                rgba(self.palette.transparent_rgba)
            })
            .cursor(CursorStyle::PointingHand)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                let already_selected = this.level().selected == index;
                this.level_mut().selected = index;
                if already_selected {
                    this.select_next_gamepad();
                }
                cx.notify();
            }))
            .child(gamepad_panel(
                monitor,
                &row.label,
                row.description.as_deref(),
                palette,
            ))
    }

    fn render_source_menu_item(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self.plugin_title(index);
        let active = index == self.selected_source;
        let mut item = div()
            .id(("settings-source", index))
            .relative()
            .flex()
            .items_center()
            .w_full()
            .h(px(super::PANEL_RAIL_ITEM_HEIGHT))
            .px(px(qol_theme::SPACE_CELL))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .child(
                div()
                    .truncate()
                    .min_w_0()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .when(active, |label| label.font_weight(FontWeight::SEMIBOLD))
                    .text_color(rgb(if active {
                        self.palette.rail_active_text
                    } else {
                        self.palette.rail_text_muted
                    }))
                    .child(label),
            )
            .cursor(CursorStyle::PointingHand);
        if active {
            item = self.paint_selection(item);
        }
        item.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if !event.standard_click() {
                return;
            }
            this.select_source(index, cx);
            this.descend_source_menu(window, cx);
            cx.notify();
        }))
    }

    fn render_rail_plugin_caption(&self) -> impl IntoElement {
        rail_caption("QoL Plugin Settings").id("settings-rail-plugin-caption")
    }

    fn render_list(&self, index: usize) -> Div {
        let row = &self.level().rows[index];
        let RowControl::List {
            active_label,
            active: runtime_active,
            filter,
            ..
        } = &row.control
        else {
            return div();
        };
        let mut header_status = div().flex().items_center().gap(px(qol_theme::SPACE_INSET));
        if *runtime_active {
            header_status = header_status.child(
                StatusIndicator::new(
                    ("settings-list-activity", index),
                    active_label.as_deref().unwrap_or("Live").to_string(),
                    rgb(self.palette.state_on),
                )
                .pulse(),
            );
        }
        header_status = header_status.child(
            div()
                .text_color(rgb(self.value_color(index)))
                .child(self.display_value(index)),
        );
        let mut container = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(qol_theme::SPACE_TIGHT))
            .h(px(row_body_height(row, false)))
            .justify_center()
            .overflow_hidden()
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .rounded(px(qol_theme::RADIUS_CARD));
        if index == self.level().selected {
            container = self.mark_selected(container, true);
        }
        container = container.child(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .h(px(list_header_height(row)))
                .justify_between()
                .gap(px(qol_theme::SPACE_CELL))
                .text_size(px(qol_theme::TEXT_BODY))
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .flex_1()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(self.palette.section_text))
                                .child(if filter.trim().is_empty() {
                                    row.label.clone()
                                } else {
                                    filter.clone()
                                }),
                        )
                        .when_some(row.description.clone(), |group, description| {
                            group.child(
                                div()
                                    .truncate()
                                    .text_size(px(qol_theme::TEXT_CAPTION))
                                    .text_color(rgb(self.palette.label_text))
                                    .child(description),
                            )
                        }),
                )
                .child(header_status),
        );
        container
    }

    fn render_qr_code(&self, index: usize) -> Div {
        let row = &self.level().rows[index];
        let RowControl::QrCode {
            url,
            modules,
            error,
            ..
        } = &row.control
        else {
            return div();
        };
        let mut container = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(qol_theme::SPACE_TIGHT))
            .h(px(row_body_height(row, true)))
            .overflow_hidden()
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .rounded(px(qol_theme::RADIUS_CARD));
        if index == self.level().selected {
            container = self.mark_selected(container, true);
        }
        container = container.child(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .h(px(list_header_height(row)))
                .gap(px(qol_theme::SPACE_CELL))
                .text_size(px(qol_theme::TEXT_BODY))
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .flex_1()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(self.palette.section_text))
                                .child(row.label.clone()),
                        )
                        .when_some(row.description.clone(), |group, description| {
                            group.child(
                                div()
                                    .truncate()
                                    .text_size(px(qol_theme::TEXT_CAPTION))
                                    .text_color(rgb(self.palette.label_text))
                                    .child(description),
                            )
                        }),
                ),
        );
        let code_block = match url {
            Some(_) => {
                let module_px = qr_module_px(modules);
                let side = qr_side(modules);
                let mut grid = div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .bg(rgb(self.palette.qr_light))
                    .h(px(super::PANEL_QR_CODE_HEIGHT));
                for y in 0..side {
                    let mut line = div().flex().flex_row().flex_none().h(px(module_px));
                    for x in 0..side {
                        let dark = modules[y * side + x];
                        if dark {
                            line = line.child(
                                div()
                                    .w(px(module_px))
                                    .h(px(module_px))
                                    .bg(rgb(self.palette.qr_dark)),
                            );
                        } else {
                            line = line.child(div().w(px(module_px)).h(px(module_px)));
                        }
                    }
                    grid = grid.child(line);
                }
                grid
            }
            None => {
                let placeholder = row
                    .placeholder
                    .clone()
                    .unwrap_or_else(|| "Waiting\u{2026}".into());
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(px(super::PANEL_QR_CODE_HEIGHT))
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(self.palette.label_text))
                    .child(placeholder)
            }
        };
        container = container.child(code_block);
        let status = match (error.as_ref(), url.as_ref()) {
            (Some(message), _) => message.clone(),
            (None, Some(url)) => url.clone(),
            (None, None) => String::new(),
        };
        if !status.is_empty() {
            container = container.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(super::PANEL_QR_URL_HEIGHT))
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(if error.is_some() {
                        rgb(self.palette.state_off)
                    } else {
                        rgb(self.palette.label_text)
                    })
                    .child(status),
            );
        }
        container
    }

    fn render_object_array(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let row = &self.level().rows[index];
        let RowControl::ObjectArray(_) = &row.control else {
            return div();
        };
        self.render_block_frame(index, row, self.display_value(index), cx)
    }

    fn object_array_card_chips(&self, index: usize) -> Option<ItemChips> {
        let state = self.level().object_array.as_ref()?;
        if index == 0 || index > state.entries.len() {
            return None;
        }
        Some(state.chips(index - 1))
    }

    fn render_chip_row(&self, parts: Vec<ChipRowPart>) -> Div {
        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_TIGHT))
            .min_w_0()
            .overflow_hidden();
        for part in parts {
            strip = strip.child(match part {
                ChipRowPart::Arrow => div()
                    .flex_none()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.status_muted))
                    .child("\u{2192}"),
                ChipRowPart::Chip(chip) => match chip.tone {
                    ChipTone::Modifier | ChipTone::Key => self.kit.keycap(chip.label),
                    ChipTone::Plain => self.kit.chip(chip.label, self.palette.label_text),
                },
            });
        }
        strip
    }

    fn render_object_array_card_draft(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.stack.len() <= 1 {
            return None;
        }
        let state = self.level().object_array.as_ref()?;
        let draft = state.draft.as_ref()?;
        let mut container = div().flex().flex_col().gap(px(qol_theme::SPACE_TIGHT));
        let index = self.level().selected;
        for (field_index, field) in draft.fields.iter().enumerate() {
            container = container.child(self.render_draft_field(index, field_index, field, cx));
        }
        container = container.child(self.render_draft_save(index, draft.save_entry_selected(), cx));
        Some(container)
    }

    fn render_block_frame(
        &self,
        index: usize,
        row: &Row,
        value: String,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut container = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(qol_theme::SPACE_TIGHT))
            .h(px(row_body_height(row, false)))
            .justify_center()
            .overflow_hidden()
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .rounded(px(qol_theme::RADIUS_CARD));
        if index == self.level().selected {
            container = self.mark_selected(container, true);
        }
        container.child(
            div()
                .id(("settings-block-header", index))
                .cursor(CursorStyle::PointingHand)
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if !event.standard_click() {
                        return;
                    }
                    this.level_mut().selected = index;
                    this.activate(window, cx);
                    cx.notify();
                }))
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .h(px(list_header_height(row)))
                .justify_between()
                .gap(px(qol_theme::SPACE_CELL))
                .text_size(px(qol_theme::TEXT_BODY))
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .flex_1()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(self.palette.section_text))
                                .child(row.label.clone()),
                        )
                        .when_some(row.description.clone(), |group, description| {
                            group.child(
                                div()
                                    .truncate()
                                    .text_size(px(qol_theme::TEXT_CAPTION))
                                    .text_color(rgb(self.palette.label_text))
                                    .child(description),
                            )
                        }),
                )
                .child(div().text_color(rgb(self.value_color(index))).child(value)),
        )
    }

    fn object_array_line(&self, index: usize, entry: usize, selected: bool) -> SettingsRow {
        SettingsRow::rule(
            ("settings-object-entry", entry_id(index, entry)),
            self.palette,
        )
        .selected(selected, self.body_has_focus())
    }

    fn object_array_state(&self, index: usize) -> Option<&ObjectArrayState> {
        if let Some(state) = self.level().object_array.as_ref() {
            return Some(state);
        }
        match &self.level().rows[index].control {
            RowControl::ObjectArray(state) => Some(state),
            _ => None,
        }
    }

    fn object_array_state_mut(&mut self, index: usize) -> Option<&mut ObjectArrayState> {
        let level = self.level_mut();
        if level.object_array.is_some() {
            return level.object_array.as_mut();
        }
        match &mut level.rows[index].control {
            RowControl::ObjectArray(state) => Some(state),
            _ => None,
        }
    }

    fn render_draft_field(
        &self,
        index: usize,
        field_index: usize,
        field: &DraftField,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.object_array_state(index) else {
            return div()
                .id(("settings-draft-empty", entry_id(index, field_index)))
                .into_any_element();
        };
        let selected = state
            .draft
            .as_ref()
            .is_some_and(|draft| draft.selected == field_index);
        self.object_array_line(index, field_index, selected)
            .child(
                div()
                    .flex_none()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.label_text))
                    .child(pretty_label(&field.key)),
            )
            .child(self.render_draft_value(index, field_index, field, selected, cx))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.select_draft_field(index, field_index);
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_draft_value(
        &self,
        index: usize,
        field_index: usize,
        field: &DraftField,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut value = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(qol_theme::SPACE_TIGHT))
            .flex_1()
            .min_w_0()
            .overflow_hidden();
        if let DraftValue::Mods {
            options,
            selected: flags,
            cursor,
        } = &field.value
        {
            for (option_index, option) in options.iter().enumerate() {
                let on = flags.get(option_index).copied().unwrap_or(false);
                let focused = selected && option_index == *cursor;
                let slot = ModChipSlot {
                    row: index,
                    field: field_index,
                    option: option_index,
                };
                value = value.child(self.render_mod_chip(slot, option.clone(), on, focused, cx));
            }
            return value;
        }
        let text = match (&field.value, selected) {
            (DraftValue::Text(text), true) => format!("{text}_"),
            (DraftValue::Text(text), false) if text.is_empty() => {
                pretty_label(&field.key).to_lowercase()
            }
            _ => field.display(),
        };
        value.child(
            div()
                .truncate()
                .text_size(px(qol_theme::TEXT_BODY))
                .text_color(rgb(match &field.value {
                    DraftValue::Text(text) if text.is_empty() && !selected => {
                        self.palette.status_muted
                    }
                    DraftValue::Boolean(true) => self.palette.state_on,
                    _ => self.palette.label_text,
                }))
                .child(text),
        )
    }

    fn render_mod_chip(
        &self,
        slot: ModChipSlot,
        label: String,
        on: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("settings-mod-chip", slot.id()))
            .flex_none()
            .px(px(qol_theme::SPACE_SNUG))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .shadow(vec![BoxShadow {
                color: rgba(qol_color::with_alpha(
                    if focused {
                        self.palette.row_border_selected
                    } else {
                        self.palette.panel_border
                    },
                    0xff,
                ))
                .into(),
                offset: point(px(0.0), px(1.5)),
                blur_radius: px(0.0),
                spread_radius: px(0.0),
            }])
            .bg(if on {
                rgb(self.palette.dropdown_bg)
            } else {
                rgba(self.palette.transparent_rgba)
            })
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(if on {
                self.palette.state_on
            } else {
                self.palette.status_muted
            }))
            .cursor(CursorStyle::PointingHand)
            .child(label)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                cx.stop_propagation();
                this.toggle_draft_mod(slot.row, slot.field, slot.option);
                cx.notify();
            }))
    }

    fn render_draft_save(
        &self,
        index: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = usize::MAX;
        self.object_array_line(index, entry, selected)
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(self.palette.state_on))
                    .child("Save"),
            )
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.label_text))
                    .child("enter"),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.commit_object_array_draft(index);
                cx.notify();
            }))
            .into_any_element()
    }

    fn toggle_draft_mod(&mut self, index: usize, field_index: usize, option_index: usize) {
        let Some(state) = self.object_array_state_mut(index) else {
            return;
        };
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        draft.toggle_mod_at(field_index, option_index);
    }

    fn select_draft_field(&mut self, index: usize, field_index: usize) {
        let Some(state) = self.object_array_state_mut(index) else {
            return;
        };
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        draft.selected = field_index;
    }

    fn commit_object_array_draft(&mut self, index: usize) {
        let Some(state) = self.object_array_state_mut(index) else {
            return;
        };
        if state.commit_draft() {
            self.sync_object_array_to_root();
            self.persist();
        }
        self.sync_scroll();
    }

    fn render_list_slider_element(
        &self,
        row_index: usize,
        slider: &ListSlider,
        item: &super::rows::ListItem,
        value: f64,
        cx: &mut Context<Self>,
    ) -> Div {
        let fill = slider_fraction(value, Some(slider.spec.min), Some(slider.spec.max)) * 72.0;
        let percent = slider_percent_label(slider.spec.min, slider.spec.max, value);
        let item_id = item.id.clone();
        let track_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let bounds_for_down = track_bounds.clone();
        let bounds_for_move = track_bounds.clone();
        let id_for_down = item_id.clone();
        let id_for_move = item_id.clone();
        let id_for_up = item_id.clone();
        let id_for_up_out = item_id;
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .cursor(CursorStyle::PointingHand)
            .child(
                div()
                    .relative()
                    .w(px(72.))
                    .h(px(4.))
                    .rounded_full()
                    .overflow_hidden()
                    .bg(rgb(self.palette.panel_border))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(px(fill))
                            .rounded_full()
                            .bg(rgb(self.palette.row_border_selected)),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| track_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            let Some(bounds) = bounds_for_down.get() else {
                                return;
                            };
                            let fraction = (((event.position.x - bounds.left()).to_f64()
                                / bounds.size.width.to_f64())
                            .clamp(0.0, 1.0)) as f32;
                            this.slider_drag = Some((row_index, id_for_down.clone()));
                            this.set_list_slider_value(row_index, &id_for_down, fraction, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        if !event.dragging()
                            || this.slider_drag.as_ref() != Some(&(row_index, id_for_move.clone()))
                        {
                            return;
                        }
                        let Some(bounds) = bounds_for_move.get() else {
                            return;
                        };
                        let fraction = (((event.position.x - bounds.left()).to_f64()
                            / bounds.size.width.to_f64())
                        .clamp(0.0, 1.0)) as f32;
                        this.set_list_slider_value(row_index, &id_for_move, fraction, cx);
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, _| {
                            if this.slider_drag.as_ref() == Some(&(row_index, id_for_up.clone())) {
                                this.slider_drag = None;
                            }
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, _| {
                            if this.slider_drag.as_ref()
                                == Some(&(row_index, id_for_up_out.clone()))
                            {
                                this.slider_drag = None;
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.label_text))
                    .child(percent),
            )
    }

    fn render_list_card_item(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(origin_row) = self.level().origin_row else {
            return div()
                .id(("settings-list-card-empty", index))
                .into_any_element();
        };
        let Some(RowControl::List {
            actions,
            slider,
            items,
            filter,
            ..
        }) = self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return div()
                .id(("settings-list-card-empty", index))
                .into_any_element();
        };
        let Some(item) = selected_list_item(actions, items, filter, index) else {
            return div()
                .id(("settings-list-card-empty", index))
                .into_any_element();
        };
        let selected = index == self.level().selected;
        let mut line = SettingsRow::rule(("settings-list-card-item", index), self.palette)
            .selected(selected, self.body_has_focus())
            .child(
                settings_label(item.label.clone(), self.palette)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(self.render_list_card_value(item))
            .child(self.render_list_card_action(index, origin_row, item, selected, cx));
        if let Some((slider, value)) = slider.as_deref().and_then(|slider| {
            list_card_slider_value(slider, actions, items, filter, index)
                .map(|value| (slider, value))
        }) {
            line = line.child(self.render_list_slider_element(origin_row, slider, item, value, cx));
        }
        line.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            this.level_mut().selected = index;
            cx.notify();
        }))
        .into_any_element()
    }

    fn render_list_card_value(&self, item: &ListItem) -> Div {
        let text = list_item_value_text(item);
        let color = if item.error.is_some() {
            self.palette.state_off
        } else if item.badge.is_some() {
            status_tone_color(self.palette, item.effective_badge_tone())
        } else {
            self.palette.label_text
        };
        div()
            .flex_none()
            .text_size(px(qol_theme::TEXT_BODY))
            .text_color(rgb(color))
            .child(text)
    }

    fn render_list_card_action(
        &self,
        index: usize,
        origin_row: usize,
        item: &ListItem,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(RowControl::List { actions, .. }) =
            self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return div();
        };
        let mut cell = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET));
        if item.pending {
            cell = cell.child(Spinner::new(
                ("settings-list-card-spinner", index),
                rgb(self.palette.state_on),
            ));
        }
        let Some(action) = primary_list_item_action(actions, item) else {
            return cell;
        };
        let action_count = list_item_actions(actions, item).len();
        let label = list_action_affordance(&action.label, action_count);
        let mut affordance = div()
            .id(("settings-list-card-action", index))
            .px(px(qol_theme::SPACE_TIGHT))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .bg(if selected {
                rgb(self.palette.row_bg_selected)
            } else {
                rgb(self.palette.dropdown_bg)
            })
            .when(!selected, |control| {
                control.shadow(crate::kit::raised_shadow(self.palette.section_text))
            })
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(if selected {
                self.palette.state_on
            } else {
                self.palette.label_text
            }))
            .child(label);
        if !item.pending {
            affordance = affordance
                .cursor(CursorStyle::PointingHand)
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if !event.standard_click() {
                        return;
                    }
                    cx.stop_propagation();
                    this.level_mut().selected = index;
                    if action_count > 1 {
                        this.open_list_card_actions();
                        cx.notify();
                        return;
                    }
                    this.dispatch_list_card_action(cx);
                    cx.notify();
                }));
        }
        let open_menu = match &self.level().active_control {
            Some(ActiveControl::ListActions(menu))
                if menu.row == origin_row && menu.item_id == item.id =>
            {
                let labels = menu
                    .actions
                    .iter()
                    .map(|action| action.label.clone())
                    .collect::<Vec<_>>();
                let view = cx.weak_entity();
                Some(menu.dropdown.render_clickable(
                    format!("settings-list-card-actions-{index}"),
                    &labels,
                    self.dropdown_style(),
                    move |selected, event, _, cx| {
                        if !event.standard_click() {
                            return;
                        }
                        cx.stop_propagation();
                        let _ = view.update(cx, |this, cx| {
                            this.dispatch_list_menu_action(selected, cx);
                            cx.notify();
                        });
                    },
                ))
            }
            _ => None,
        };
        let Some(menu) = open_menu else {
            return cell.child(affordance);
        };
        cell.child(div().relative().child(menu).child(affordance))
    }
}

fn theme_override_values(values: &serde_json::Value) -> (Option<&str>, Option<&str>) {
    (
        values
            .get("native_theme")
            .and_then(serde_json::Value::as_str),
        values.get("accent").and_then(serde_json::Value::as_str),
    )
}

impl Focusable for SettingsPanelView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        let build_started = std::time::Instant::now();
        self.palette = settings_panel_runtime();
        self.poll_visible.store(
            window.is_window_active(),
            std::sync::atomic::Ordering::Relaxed,
        );
        if !self.frame_pump_armed {
            self.pump_frame_paced_samples(window, cx);
        }
        self.fit_lists();
        if self.custom_focus_pending {
            if let Some(custom) = self.custom_view() {
                (custom.focus)(window, cx);
            }
            self.custom_focus_pending = false;
        }
        let rail: Vec<AnyElement> = if self.rail_is_open() {
            let mut rail = vec![rail_caption("QoL Settings")
                .id("settings-rail-core-caption")
                .into_any_element()];
            let groups = self
                .panel
                .sources
                .iter()
                .map(|source| source.group)
                .collect::<Vec<_>>();
            let separators = rail_group_breaks(&groups);
            for index in 0..self.sources.len() {
                if separators.contains(&index) {
                    rail.push(self.render_rail_plugin_caption().into_any_element());
                }
                rail.push(self.render_source_menu_item(index, cx).into_any_element());
            }
            rail
        } else {
            Vec::new()
        };
        let mut items: Vec<AnyElement> = Vec::new();
        if let Some(draft_body) = self.render_object_array_card_draft(cx) {
            items.push(draft_body.into_any_element());
        } else {
            for (title, rows) in self.body_groups() {
                let labels: Vec<&str> = rows
                    .iter()
                    .map(|index| self.level().rows[*index].label.as_str())
                    .collect();
                if !header_is_redundant(&title, &labels) {
                    items.push(
                        self.render_group_header(&title, rows.len())
                            .into_any_element(),
                    );
                }
                for index in rows {
                    items.push(self.render_row(index, cx).into_any_element());
                }
            }
        }
        let custom_view = self.custom_view().map(|custom| custom.view.clone());
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "SETTINGS_FRAME",
            "phase=build rows={} rail={} elapsed_us={}",
            items.len(),
            rail.len(),
            build_started.elapsed().as_micros()
        );
        div()
            .id("settings-panel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .relative()
            .overflow_hidden()
            .flex()
            .flex_col()
            .rounded_none()
            .shadow(crate::kit::float_shadow(self.palette.section_text))
            .bg(rgb(self.palette.window_bg))
            .text_color(rgb(self.palette.section_text))
            .child(self.render_band())
            .child(self.render_content(
                window.viewport_size().width.to_f64() as f32,
                rail,
                items,
                custom_view,
            ))
            .when_some(self.save_error.clone(), |root, message| {
                root.child(self.render_failure_bar(message))
            })
            .child(self.render_hint_bar())
            .child(self.resize_canvas())
    }
}

impl SettingsPanelView {
    fn render_card(
        &self,
        level_index: usize,
        items: Vec<AnyElement>,
        custom_view: Option<AnyView>,
    ) -> Div {
        let front = level_index + 1 == self.stack.len();
        let has_custom_view = custom_view.is_some();
        let body = if let Some(custom_view) = custom_view {
            div()
                .id(("settings-panel-body", level_index))
                .size_full()
                .flex()
                .flex_col()
                .gap(px(qol_theme::SPACE_TIGHT))
                .child(custom_view)
        } else {
            settings_page()
                .id(("settings-panel-body", level_index))
                .track_scroll(self.stack[level_index].body_scroll.handle())
                .overflow_y_scroll()
                .when(front && self.filter_open, |body| {
                    body.pt(px(FILTER_OVERLAY_HEIGHT))
                })
                .children(items)
        };
        div().flex_1().min_w_0().h_full().flex().flex_col().child(
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .child(body)
                .when(front && !has_custom_view, |frame| {
                    frame.child(crate::scrollbar::seam_track(
                        self.stack[level_index].body_scroll.handle().clone(),
                        crate::kit::alpha(self.palette.panel_border, 0x48),
                        crate::kit::alpha(self.palette.section_text, 0x8c),
                    ))
                })
                .when(self.filter_open && front && !has_custom_view, |frame| {
                    frame.child(self.render_filter_overlay())
                }),
        )
    }

    fn render_content(
        &self,
        width: f32,
        rail: Vec<AnyElement>,
        items: Vec<AnyElement>,
        custom_view: Option<AnyView>,
    ) -> Div {
        let depth = self.stack.len() - 1;
        let card = self.render_card(self.stack.len() - 1, items, custom_view);
        let base = div().flex_1().min_h(px(0.)).flex().flex_row().items_start();
        if !self.rail_is_open() {
            let slide = if self.deck_transition.snapped {
                None
            } else {
                deck::slide(self.deck_transition.step, self.deck_motion, depth, width)
            };
            let deck_ease = || Animation::new(RAIL_TRANSITION).with_easing(ease_out_quint());
            if depth == 0 {
                let card = match slide {
                    Some(slide) => card
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .with_animation(
                            ("settings-card-deck-slide", slide.step),
                            deck_ease(),
                            move |card, delta| {
                                card.left(px(slide.from + (slide.to - slide.from) * delta))
                            },
                        )
                        .into_any_element(),
                    None => card.into_any_element(),
                };
                return base.child(card);
            }
            return base.child(self.render_deck(depth, card, slide));
        }
        let entering = !self.rail_source_level();
        let progress = move |delta: f32| if entering { delta } else { 1.0 - delta };
        let step = self.rail_transition.step;
        let snapped = self.rail_transition.snapped;
        let ease = || Animation::new(RAIL_TRANSITION).with_easing(ease_in_out);
        let rail_column = div()
            .id("settings-section-rail")
            .flex_none()
            .relative()
            .flex()
            .flex_col()
            .h_full()
            .w(px(super::PANEL_RAIL_WIDTH))
            .p(px(qol_theme::SPACE_INSET))
            .children(rail);
        let scrim = div()
            .absolute()
            .inset_0()
            .bg(crate::kit::rail_scrim(self.palette.window_bg));
        let rail_column = if snapped {
            let reached = progress(1.0);
            rail_column
                .child(scrim.opacity(reached))
                .opacity(1.0 - RAIL_DIM * reached)
                .into_any_element()
        } else {
            rail_column
                .child(scrim.with_animation(
                    ("settings-rail-scrim", step),
                    ease(),
                    move |scrim, delta| scrim.opacity(progress(delta)),
                ))
                .with_animation(("settings-rail-dim", step), ease(), move |rail, delta| {
                    rail.opacity(1.0 - RAIL_DIM * progress(delta))
                })
                .into_any_element()
        };
        base.relative().child(rail_column).child(match depth {
            0 => {
                let card = card
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .bg(rgb(self.palette.window_bg))
                    .border_t(px(1.))
                    .border_r(px(1.))
                    .border_b(px(1.))
                    .border_color(self.hairline())
                    .shadow(crate::kit::float_shadow(self.palette.section_text));
                let accent = crate::kit::accent_left_edge(
                    qol_theme::RADIUS_CARD,
                    RAIL_CARD_ACCENT,
                    self.palette.row_border_selected,
                );
                if snapped {
                    let reached = progress(1.0);
                    card.child(
                        accent
                            .rounded_l(px(qol_theme::RADIUS_CARD * reached))
                            .border_l(px(RAIL_CARD_ACCENT * reached)),
                    )
                    .left(px(super::PANEL_RAIL_WIDTH - RAIL_CARD_OVERLAP * reached))
                    .rounded_l(px(qol_theme::RADIUS_CARD * reached))
                    .into_any_element()
                } else {
                    card.child(accent.with_animation(
                        ("settings-card-accent", step),
                        ease(),
                        move |edge, delta| {
                            let reached = progress(delta);
                            edge.rounded_l(px(qol_theme::RADIUS_CARD * reached))
                                .border_l(px(RAIL_CARD_ACCENT * reached))
                        },
                    ))
                    .with_animation(("settings-card-slide", step), ease(), move |card, delta| {
                        let reached = progress(delta);
                        card.left(px(super::PANEL_RAIL_WIDTH - RAIL_CARD_OVERLAP * reached))
                            .rounded_l(px(qol_theme::RADIUS_CARD * reached))
                    })
                    .into_any_element()
                }
            }
            _ => {
                let deck = self
                    .render_deck(depth, card, None)
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0();
                if snapped {
                    let reached = progress(1.0);
                    deck.left(px(super::PANEL_RAIL_WIDTH - RAIL_CARD_OVERLAP * reached))
                        .into_any_element()
                } else {
                    deck.with_animation(
                        ("settings-card-slide", step),
                        ease(),
                        move |deck, delta| {
                            let reached = progress(delta);
                            deck.left(px(super::PANEL_RAIL_WIDTH - RAIL_CARD_OVERLAP * reached))
                        },
                    )
                    .into_any_element()
                }
            }
        })
    }

    fn render_deck(&self, depth: usize, card: Div, slide: Option<DeckSlide>) -> Div {
        deck::render(self.palette, depth, card, slide, "settings-card-deck-slide")
    }

    fn panel_row_total(&self) -> usize {
        self.current_visible_rows().len()
    }

    fn hairline(&self) -> Rgba {
        rgba(self.kit.washes.hairline.packed())
    }

    /// Lays the whole path out as one trail, `qol Settings › Plugin › Card`:
    /// ancestors stay muted, the current page is bright and bold, and the
    /// separator only ever sits between two crumbs.
    fn crumb_elements(&self, trail: Vec<String>) -> Vec<Div> {
        let last = trail.len().saturating_sub(1);
        let separator = rgba(crate::kit::alpha(self.palette.status_muted, 0x70));
        let mut crumbs = Vec::with_capacity(trail.len() * 2);
        for (index, label) in trail.into_iter().enumerate() {
            if index > 0 {
                crumbs.push(
                    div()
                        .flex_none()
                        .px(px(qol_theme::SPACE_INSET))
                        .text_color(separator)
                        .child("\u{203A}"),
                );
            }
            let crumb = if index == last {
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.section_text))
            } else {
                div()
                    .max_w(px(CRUMB_MAX_WIDTH))
                    .text_color(rgb(self.palette.status_muted))
            };
            crumbs.push(crumb.truncate().child(label));
        }
        crumbs
    }

    fn render_band(&self) -> Div {
        let total = self.panel_row_total();
        let trail = self.trail();
        let subtitle_fits = self.stack.len() == 1;
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(qol_theme::SPACE_GUTTER))
            .h(px(super::PANEL_BAND_HEIGHT))
            .px(px(qol_theme::SPACE_GUTTER))
            .border_b(px(1.))
            .border_color(self.hairline())
            .bg(rgb(self.palette.rail_bg))
            .panel_drag_area()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(qol_theme::SPACE_STACK))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .text_size(px(qol_theme::TEXT_TITLE))
                            .line_height(px(BAND_TEXT_LINE_HEIGHT))
                            .children(self.crumb_elements(trail)),
                    )
                    .when(subtitle_fits, |group| {
                        group.children(self.subtitle.clone().map(|subtitle| {
                            div()
                                .truncate()
                                .text_size(px(qol_theme::TEXT_MICRO))
                                .text_color(rgb(self.palette.status_muted))
                                .child(subtitle)
                        }))
                    }),
            )
            .when(!self.current_source_is_custom(), |band| {
                band.child(self.kit.count_chip(total, plural(total, "setting")))
            })
    }

    fn render_filter_field(&self) -> Div {
        let empty = self.filter.is_empty();
        let text = if empty {
            "Filter settings".to_string()
        } else {
            self.filter.clone()
        };
        let field = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .h(px(super::PANEL_FILTER_HEIGHT))
            .px(px(qol_theme::SPACE_PAD))
            .rounded(px(qol_theme::RADIUS_WELL))
            .bg(rgba(self.kit.washes.fill_resting.packed()))
            .border(px(1.))
            .border_color(self.hairline())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(if empty {
                        self.palette.status_muted
                    } else {
                        self.palette.section_text
                    }))
                    .child(text),
            );
        if self.filter_open {
            return field
                .bg(rgb(self.palette.window_bg))
                .border_color(rgb(self.palette.row_border_selected))
                .child(
                    div()
                        .flex_none()
                        .w(px(1.5))
                        .h(px(16.))
                        .bg(rgb(self.palette.row_border_selected)),
                );
        }
        field.child(self.kit.keycap("/"))
    }

    fn render_filter_overlay(&self) -> Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .px(px(qol_theme::SPACE_PAD))
            .pt(px(qol_theme::SPACE_CELL))
            .pb(px(qol_theme::SPACE_INSET))
            .bg(rgb(self.palette.window_bg))
            .child(self.render_filter_field())
    }

    fn body_groups(&self) -> Vec<(String, Vec<usize>)> {
        if !self.filtering() {
            return (0..self.level().sections.len())
                .filter(|section| {
                    self.level().sections[*section].source == self.materialized_source
                })
                .map(|section| {
                    let visible = self.section_filtered_rows(section);
                    (self.section_title(section), visible)
                })
                .filter(|(_, rows)| !rows.is_empty())
                .collect();
        }
        let mut groups = Vec::new();
        for source in 0..self.sources.len() {
            let mut rows = Vec::new();
            for section in 0..self.level().sections.len() {
                if self.level().sections[section].source == source {
                    rows.extend(self.section_filtered_rows(section));
                }
            }
            if rows.is_empty() {
                continue;
            }
            groups.push((self.plugin_title(source), rows));
        }
        groups
    }

    fn section_title(&self, section: usize) -> String {
        self.level()
            .sections
            .get(section)
            .map(|section| section.label.clone())
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| self.panel.heading.clone())
    }

    fn trail(&self) -> Vec<String> {
        let cards = self
            .stack
            .iter()
            .skip(1)
            .map(|level| level.title.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        let plugin = (self.sources.len() > 1 && !self.filtering())
            .then(|| self.plugin_title(self.materialized_source));
        crumb_labels(&self.panel.heading, plugin, cards)
    }

    fn plugin_title(&self, source: usize) -> String {
        self.panel
            .sources
            .get(source)
            .map(|source| source.heading.clone())
            .unwrap_or_else(|| {
                self.sources
                    .get(source)
                    .map(|source| source.plugin_id.clone())
                    .unwrap_or_default()
            })
    }

    fn render_group_header(&self, title: &str, count: usize) -> impl IntoElement {
        SettingsGroupHeader::new(
            title.to_string(),
            count,
            plural(count, "setting"),
            self.palette,
        )
    }

    fn enter_hint(&self) -> Option<&'static str> {
        if self.level().list_card {
            return Some("run");
        }
        let row = self.level().rows.get(self.level().selected)?;
        match &row.control {
            RowControl::Toggle(_) => Some("flip"),
            RowControl::Action { .. } => Some("run"),
            RowControl::Select { .. } | RowControl::MultiSelect { .. } => Some("choose"),
            RowControl::Color(_) => Some("pick"),
            RowControl::List { items, .. } if !items.is_empty() => Some("open"),
            RowControl::ObjectArray(_) => Some("edit"),
            RowControl::Number { .. } | RowControl::Text(_) | RowControl::TextList(_) => {
                Some("edit")
            }
            RowControl::Gamepad { .. } => Some("next"),
            RowControl::List { .. }
            | RowControl::Status { .. }
            | RowControl::QrCode { .. }
            | RowControl::Unsupported { .. } => None,
        }
    }

    fn render_hint_bar(&self) -> Div {
        if self.current_source_is_custom() && self.body_has_focus() {
            return self
                .kit
                .hint_bar()
                .child(self.kit.hint("\u{2191}\u{2193}", "move"))
                .child(self.kit.hint("\u{21b5}", "open"))
                .child(self.kit.hint("A", "add"))
                .child(self.kit.hint("\u{232b}", "delete"))
                .when(self.custom_tool_is_shortcuts(), |bar| {
                    bar.child(self.kit.hint("R", "run"))
                })
                .child(div().flex_1())
                .child(self.kit.hint("esc", "back"));
        }
        let mut bar = self
            .kit
            .hint_bar()
            .children(
                self.enter_hint()
                    .map(|label| self.kit.hint("\u{21b5}", label)),
            )
            .child(self.kit.hint("\u{2191}\u{2193}", "move"));
        if self.filtering() {
            bar = bar
                .child(self.kit.hint("esc", "back to plugins"))
                .child(div().flex_1());
        } else {
            bar = bar
                .child(self.kit.hint("type", "search every plugin"))
                .child(div().flex_1())
                .child(self.kit.hint("esc", "close"));
        }
        bar
    }

    fn custom_tool_is_shortcuts(&self) -> bool {
        self.panel
            .sources
            .get(self.materialized_source)
            .is_some_and(|source| source.plugin_id == "__core-shortcuts")
    }

    fn render_failure_bar(&self, message: String) -> impl IntoElement {
        SettingsFeedback::new(message, self.palette.status_danger, true)
    }

    fn resize_canvas(&mut self) -> impl IntoElement {
        let dismisser = self.dismisser.clone();
        let target = self.window_height();
        canvas(
            |_, _, _| (),
            move |_bounds, _, window, cx| {
                let current = window.viewport_size().height.to_f64() as f32;
                if (target - current).abs() <= 1.0 {
                    return;
                }
                let next = size(dismisser.window_size().width, px(target));
                let handle = window.window_handle();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| {
                        dismisser.resize_window(next, window);
                    });
                });
            },
        )
        .absolute()
        .inset_0()
    }
}

fn dropdown_items(options: &[SelectOption]) -> Vec<DropdownItem> {
    options
        .iter()
        .map(|option| DropdownItem {
            label: option.label.clone(),
            accent: option.accent,
        })
        .collect()
}

fn parsed_number(edit: &str, min: Option<f64>, max: Option<f64>, step: Option<f64>) -> Option<f64> {
    let mut value = edit.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
    if let Some(min) = min {
        value = value.max(min);
    }
    if let Some(max) = max {
        value = value.min(max);
    }
    if let Some(step) = step {
        value = align_to_step(value, min, max, step);
    }
    Some(value)
}

fn align_to_step(value: f64, min: Option<f64>, max: Option<f64>, step: f64) -> f64 {
    let origin = min.unwrap_or(0.0);
    let steps = (value - origin) / step;
    let rounded = steps.round();
    if (steps - rounded).abs() <= 1e-9 {
        return value;
    }
    for n in [rounded, rounded - 1.0, rounded + 1.0] {
        let candidate = origin + n * step;
        if min.is_none_or(|min| candidate >= min) && max.is_none_or(|max| candidate <= max) {
            return round_to_step_precision(candidate, step);
        }
    }
    value
}

fn round_to_step_precision(value: f64, step: f64) -> f64 {
    let decimals = format!("{step}")
        .split('.')
        .nth(1)
        .map(|fraction| fraction.len() as i32)
        .unwrap_or(0);
    let factor = 10f64.powi(decimals);
    (value * factor).round() / factor
}

fn stepped_number(
    edit: &str,
    fallback: f64,
    min: Option<f64>,
    max: Option<f64>,
    step: f64,
    direction: f64,
) -> String {
    let current = parsed_number(edit, min, max, Some(step)).unwrap_or(fallback);
    let next = parsed_number(
        &(current + direction * step).to_string(),
        min,
        max,
        Some(step),
    )
    .unwrap_or(current);
    format_number(next)
}

fn number_preview(
    edit: Option<&str>,
    fallback: f64,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
) -> f64 {
    edit.and_then(|value| parsed_number(value, min, max, step))
        .unwrap_or(fallback)
}

fn horizontal_step_direction(key: &str) -> Option<f64> {
    match key {
        "left" => Some(-1.0),
        "right" => Some(1.0),
        _ => None,
    }
}

fn parsed_color(text: &str) -> Option<u32> {
    let hex = text.trim();
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{}", (value * 10_000.0).round() / 10_000.0)
    }
}

fn color_display(value: &str) -> String {
    if value.starts_with('#') {
        value.to_string()
    } else {
        format!("#{value}")
    }
}

fn text_or_placeholder(value: &str, placeholder: Option<&str>) -> String {
    if !value.is_empty() {
        return value.to_string();
    }
    placeholder.unwrap_or("Empty").to_string()
}

fn number_unit(field_id: &str) -> Option<&'static str> {
    if field_id.ends_with("_percent") {
        return Some("%");
    }
    if field_id.ends_with("_px") || field_id.ends_with("_pixels") {
        return Some("px");
    }
    if field_id.ends_with("_ms") {
        return Some("ms");
    }
    if field_id.ends_with("_seconds") {
        return Some("s");
    }
    None
}

fn slider_fraction(value: f64, min: Option<f64>, max: Option<f64>) -> f32 {
    let Some(min) = min else {
        return 0.0;
    };
    let Some(max) = max else {
        return 0.0;
    };
    if max <= min {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
}

fn stepped_slider_value(current: f64, direction: f64, min: f64, max: f64, step: f64) -> f64 {
    let next = round_to_step_precision(current + direction * step, step);
    next.clamp(min, max)
}

fn slider_value_from_fraction(min: f64, max: f64, step: f64, fraction: f32) -> f64 {
    let value = min + f64::from(fraction) * (max - min);
    align_to_step(value, Some(min), Some(max), step)
}

fn slider_percent_label(min: f64, max: f64, value: f64) -> String {
    let fraction = if max > min {
        (value - min) / (max - min)
    } else {
        0.0
    };
    format!("{:.0}%", fraction * 100.0)
}

fn step_list_slider(slider: &mut ListSlider, item: &ListItem, direction: f64) {
    let current = list_slider_value(&slider.spec, &slider.values, item);
    let next = stepped_slider_value(
        current,
        direction,
        slider.spec.min,
        slider.spec.max,
        slider.spec.step,
    );
    slider.values.insert(
        item.id.clone(),
        SliderHold {
            value: next,
            dispatched: None,
            until: std::time::Instant::now() + SLIDER_HOLD_DURATION,
        },
    );
}

fn list_card_slider_value(
    slider: &ListSlider,
    actions: &ListActions,
    items: &[ListItem],
    filter: &str,
    slot: usize,
) -> Option<f64> {
    let item = selected_list_item(actions, items, filter, slot)?;
    Some(list_slider_value(&slider.spec, &slider.values, item))
}

fn resolve_list_slider_dispatch(
    rows: &mut [Row],
    row_index: usize,
    item_id: &str,
) -> Option<(ResolvedRowAction, String)> {
    let RowControl::List { slider, items, .. } =
        rows.get_mut(row_index).map(|row| &mut row.control)?
    else {
        return None;
    };
    let slider = slider.as_mut()?;
    let item = items.iter().find(|item| item.id == item_id)?;
    let value = list_slider_value(&slider.spec, &slider.values, item);
    if let Some(hold) = slider.values.get_mut(item_id) {
        hold.dispatched = Some(value);
    }
    Some((
        resolve_slider_action(&slider.spec, &item.data, value),
        item.id.clone(),
    ))
}

fn binary_state_label(active: bool) -> &'static str {
    if active {
        "On"
    } else {
        "Off"
    }
}

fn action_value_label(
    active: bool,
    pending: bool,
    failed: bool,
    has_runtime_state: bool,
    has_active_action: bool,
    state_labels: &std::collections::BTreeMap<String, String>,
) -> String {
    if pending {
        return "working\u{2026}".into();
    }
    if failed {
        return "failed".into();
    }
    if !has_runtime_state {
        return "Run".into();
    }
    if let Some(label) = state_labels.get(if active { "true" } else { "false" }) {
        return label.clone();
    }
    if active && has_active_action {
        return "Stop".into();
    }
    if active {
        return "Active".into();
    }
    "Run".into()
}

fn binary_state_color(palette: SettingsPanelPalette, active: bool) -> u32 {
    if active {
        palette.state_on
    } else {
        palette.state_off
    }
}

fn status_tone_color(palette: SettingsPanelPalette, tone: StatusTone) -> u32 {
    match tone {
        StatusTone::Accent => palette.status_accent,
        StatusTone::Success => palette.status_success,
        StatusTone::Danger => palette.status_danger,
        StatusTone::Warning => palette.status_warning,
        StatusTone::Muted => palette.status_muted,
    }
}

fn list_action_affordance(primary: &str, action_count: usize) -> String {
    if action_count > 1 {
        return format!("{primary} +{}", action_count - 1);
    }
    primary.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Up,
    Down,
    Activate,
    CommitEdit,
    Backspace,
    Insert(String),
    Close,
    CancelEdit,
}

#[derive(Clone, Copy)]
struct ModChipSlot {
    row: usize,
    field: usize,
    option: usize,
}

impl ModChipSlot {
    fn id(self) -> u64 {
        ((self.row as u64) << 32) | ((self.field as u16 as u64) << 16) | self.option as u16 as u64
    }
}

fn entry_id(index: usize, entry: usize) -> u64 {
    ((index as u64) << 32) | (entry as u32) as u64
}

fn item_count_label(count: usize) -> String {
    match count {
        0 => "none".to_string(),
        1 => "1 item".to_string(),
        _ => format!("{count} items"),
    }
}

fn action_refresh_payload(
    result: &Result<Option<serde_json::Value>, String>,
    active_value_from: Option<&str>,
) -> Option<serde_json::Value> {
    result
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .filter(|value| query_flag_value(value, active_value_from).is_some())
        .cloned()
}

fn action_shows_spinner(row: &Row) -> bool {
    match &row.control {
        RowControl::Action {
            pending: true,
            error: None,
            ..
        } => true,
        RowControl::Action {
            active: true,
            error: None,
            ..
        } => row.variant.as_deref() != Some("toggle"),
        _ => false,
    }
}

fn intent(key: &str, key_char: Option<&str>, editing: bool) -> Option<Intent> {
    if editing {
        return match key {
            "enter" | "return" => Some(Intent::CommitEdit),
            "escape" => Some(Intent::CancelEdit),
            "backspace" => Some(Intent::Backspace),
            _ => key_char.map(|ch| Intent::Insert(ch.to_string())),
        };
    }
    match key {
        "up" => Some(Intent::Up),
        "down" => Some(Intent::Down),
        "enter" | "return" | "space" => Some(Intent::Activate),
        "escape" => Some(Intent::Close),
        _ => None,
    }
}

pub(super) fn row_height(row: &Row, show_section_headers: bool) -> f32 {
    let header = if show_section_headers && row.section_label.is_some() {
        super::PANEL_SECTION_HEADER_HEIGHT
    } else {
        0.0
    };
    row_body_height(row, false) + header
}

fn list_fit_updates(
    rows: &[Row],
    sections: &[RowSection],
    selected_source: usize,
    height_cap: f32,
) -> Vec<(usize, usize)> {
    let show_section_headers = false;
    let budget = height_cap - super::chrome_height(&[]);
    let mut updates = Vec::new();
    for section in sections {
        if section.source != selected_source {
            continue;
        }
        let visible = section
            .rows
            .iter()
            .copied()
            .filter(|index| super::rows::row_is_visible(rows, *index))
            .collect::<Vec<_>>();
        let mut fixed = visible.len().saturating_sub(1) as f32 * super::PANEL_COLUMN_GAP;
        let mut lists: Vec<usize> = Vec::new();
        for index in &visible {
            let row = &rows[*index];
            let header = if show_section_headers && row.section_label.is_some() {
                super::PANEL_SECTION_HEADER_HEIGHT
            } else {
                0.0
            };
            if matches!(row.control, RowControl::List { .. }) {
                fixed += header + super::PANEL_LIST_PADDING_Y + list_header_height(row);
                lists.push(*index);
            } else {
                fixed += row_height(row, show_section_headers);
            }
        }
        if lists.is_empty() {
            continue;
        }
        let per_list = ((budget - fixed) / lists.len() as f32).max(0.0);
        let fit = (per_list / (super::PANEL_LIST_ITEM_HEIGHT + super::PANEL_LIST_GAP)) as usize;
        let visible_items = fit.clamp(LIST_FIT_MIN_VISIBLE, super::rows::LIST_MAX_VISIBLE);
        for index in lists {
            if let RowControl::List { list, .. } = &rows[index].control {
                if list.max_visible != visible_items {
                    updates.push((index, visible_items));
                }
            }
        }
    }
    updates
}

fn source_window_height_for(
    rows: &[Row],
    sections: &[RowSection],
    source: usize,
    height_cap: f32,
) -> f32 {
    let sections = sections
        .iter()
        .filter(|section| section.source == source)
        .cloned()
        .collect::<Vec<_>>();
    super::panel_height(rows, &sections).clamp(
        super::chrome_height(&sections) + super::PANEL_ROW_HEIGHT,
        height_cap,
    )
}

pub(super) fn row_body_height(row: &Row, expanded: bool) -> f32 {
    if !expanded {
        return super::PANEL_ROW_HEIGHT;
    }
    if matches!(row.control, RowControl::Gamepad { .. }) {
        return super::PANEL_GAMEPAD_HEIGHT;
    }
    if matches!(row.control, RowControl::QrCode { .. }) {
        return super::PANEL_LIST_PADDING_Y
            + list_header_height(row)
            + super::PANEL_QR_CODE_HEIGHT
            + super::PANEL_QR_URL_HEIGHT
            + 2.0 * super::PANEL_LIST_GAP;
    }
    super::PANEL_ROW_HEIGHT
}

fn body_child_offset(
    sections: &[impl AsRef<[usize]>],
    headers: &[bool],
    row: usize,
) -> Option<usize> {
    let mut child = 0;
    for (section_index, section) in sections.iter().enumerate() {
        let visible = section.as_ref();
        if visible.is_empty() {
            continue;
        }
        if headers.get(section_index).copied().unwrap_or(true) {
            child += 1;
        }
        match visible.iter().position(|index| *index == row) {
            Some(position) => return Some(child + position),
            None => child += visible.len(),
        }
    }
    None
}

fn text_list_child_rows(_label: &str, key: &str, source: usize, values: &[String]) -> Vec<Row> {
    let mut rows = vec![Row {
        id: "text_list_add".into(),
        section_id: None,
        section_label: None,
        label: "+ Add".into(),
        description: None,
        placeholder: None,
        variant: None,
        config_key: key.to_string(),
        default: qol_config::contract::FieldDefault::String(String::new()),
        stream: None,
        action: None,
        visibility: None,
        source,
        control: RowControl::Text(String::new()),
    }];
    rows.extend(values.iter().enumerate().map(|(index, value)| Row {
        id: format!("text_list_item_{index}"),
        section_id: None,
        section_label: None,
        label: value.clone(),
        description: None,
        placeholder: None,
        variant: None,
        config_key: key.to_string(),
        default: qol_config::contract::FieldDefault::String(String::new()),
        stream: None,
        action: None,
        visibility: None,
        source,
        control: RowControl::Text(value.clone()),
    }));
    rows
}

fn object_array_child_rows(
    _label: &str,
    key: &str,
    source: usize,
    state: &ObjectArrayState,
) -> Vec<Row> {
    let mut rows = vec![Row {
        id: "object_array_add".into(),
        section_id: None,
        section_label: None,
        label: "+ Add".into(),
        description: None,
        placeholder: None,
        variant: None,
        config_key: key.to_string(),
        default: qol_config::contract::FieldDefault::String(String::new()),
        stream: None,
        action: None,
        visibility: None,
        source,
        control: RowControl::Text(String::new()),
    }];
    rows.extend((0..state.entries.len()).map(|index| {
        let summary = state.summary(index);
        Row {
            id: format!("object_array_item_{index}"),
            section_id: None,
            section_label: None,
            label: summary.clone(),
            description: None,
            placeholder: None,
            variant: None,
            config_key: key.to_string(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            source,
            control: RowControl::Text(summary),
        }
    }));
    rows
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChipRowPart {
    Chip(Chip),
    Arrow,
}

fn chip_row_parts(chips: &ItemChips) -> Vec<ChipRowPart> {
    let mut parts = Vec::new();
    if chips.is_directional() {
        parts.extend(chips.from.iter().cloned().map(ChipRowPart::Chip));
        if let Some(shared) = shared_key_chip(&chips.rest) {
            parts.push(ChipRowPart::Chip(shared));
        }
        parts.push(ChipRowPart::Arrow);
        parts.extend(chips.to.iter().cloned().map(ChipRowPart::Chip));
    } else {
        parts.extend(
            chips
                .from
                .iter()
                .chain(&chips.rest)
                .chain(&chips.to)
                .cloned()
                .map(ChipRowPart::Chip),
        );
    }
    parts.extend(chips.flags.iter().map(|flag| {
        ChipRowPart::Chip(Chip {
            label: flag.clone(),
            tone: ChipTone::Plain,
        })
    }));
    parts
}

fn initial_card_selection(count: usize) -> usize {
    if count == 0 {
        0
    } else {
        1
    }
}

fn object_array_card_level(
    label: &str,
    config_key: &str,
    source: usize,
    state: ObjectArrayState,
    origin_row: usize,
) -> Level {
    let selected = initial_card_selection(state.entries.len());
    let mut state = state;
    state.list.selected = selected;
    state.list.sync(state.entry_count());
    let rows = object_array_child_rows(label, config_key, source, &state);
    let section = RowSection {
        label: label.to_string(),
        description: None,
        rows: (0..rows.len()).collect(),
        source,
    };
    let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
    Level {
        rows,
        sections: vec![section],
        selected,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds,
        title: Some(label.to_string()),
        origin_row: Some(origin_row),
        object_array: Some(state),
        list_card: false,
        live_card: false,
    }
}

fn sync_object_array_level(level: &mut Level, label: &str, config_key: &str, source: usize) {
    let Some(state) = level.object_array.as_ref() else {
        return;
    };
    let selected = state.list.selected;
    let rows = object_array_child_rows(label, config_key, source, state);
    let section = RowSection {
        label: label.to_string(),
        description: None,
        rows: (0..rows.len()).collect(),
        source,
    };
    level.rows = rows;
    level.sections = vec![section];
    level.row_bounds = (0..level.rows.len())
        .map(|_| Rc::new(Cell::new(None)))
        .collect();
    level.selected = selected.min(level.rows.len() - 1);
}

fn object_array_card_sync(root_rows: &mut [Row], level: &mut Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let Some(RowControl::ObjectArray(stored)) =
        root_rows.get_mut(origin_row).map(|row| &mut row.control)
    else {
        return;
    };
    let Some(child) = level.object_array.as_ref() else {
        return;
    };
    *stored = ObjectArrayState::from_entries(
        child.key_label.clone(),
        child.schema.clone(),
        child.entries.clone(),
    );
    let Some(row) = root_rows.get(origin_row) else {
        return;
    };
    let label = row.label.clone();
    let config_key = row.config_key.clone();
    let source = row.source;
    sync_object_array_level(level, &label, &config_key, source);
}

fn list_item_value_text(item: &ListItem) -> String {
    item.error
        .clone()
        .or_else(|| item.badge.clone())
        .or_else(|| item.subtitle.clone())
        .unwrap_or_default()
}

fn list_card_child_rows(
    _label: &str,
    key: &str,
    source: usize,
    items: &[ListItem],
    visible: &[usize],
) -> Vec<Row> {
    visible
        .iter()
        .map(|item_index| {
            let item = &items[*item_index];
            Row {
                id: item.id.clone(),
                section_id: None,
                section_label: None,
                label: item.label.clone(),
                description: None,
                placeholder: None,
                variant: None,
                config_key: key.to_string(),
                default: qol_config::contract::FieldDefault::String(String::new()),
                stream: None,
                action: None,
                visibility: None,
                source,
                control: RowControl::Text(list_item_value_text(item)),
            }
        })
        .collect()
}

struct ListCardOrigin<'a> {
    label: &'a str,
    config_key: &'a str,
    source: usize,
    row: usize,
}

fn list_card_level(
    origin: ListCardOrigin,
    actions: &ListActions,
    items: &[ListItem],
    filter: &str,
    selected_slot: usize,
) -> Level {
    let ListCardOrigin {
        label,
        config_key,
        source,
        row: origin_row,
    } = origin;
    let visible = filtered_list_items(actions, items, filter);
    let rows = list_card_child_rows(label, config_key, source, items, &visible);
    let section = RowSection {
        label: label.to_string(),
        description: None,
        rows: (0..rows.len()).collect(),
        source,
    };
    let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
    let selected = selected_slot.min(rows.len().saturating_sub(1));
    Level {
        rows,
        sections: vec![section],
        selected,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds,
        title: Some(label.to_string()),
        origin_row: Some(origin_row),
        object_array: None,
        list_card: true,
        live_card: false,
    }
}

fn list_card_sync(root_rows: &mut [Row], level: &mut Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let selected = level.selected;
    let selected_id = level.rows.get(selected).map(|row| row.id.clone());
    let (label, config_key, source) = {
        let Some(parent) = root_rows.get(origin_row) else {
            return;
        };
        (
            parent.label.clone(),
            parent.config_key.clone(),
            parent.source,
        )
    };
    let Some(RowControl::List {
        actions,
        items,
        filter,
        list,
        ..
    }) = root_rows.get_mut(origin_row).map(|row| &mut row.control)
    else {
        return;
    };
    let visible = filtered_list_items(actions, items, filter);
    let visible_ids = visible
        .iter()
        .map(|slot| items[*slot].id.clone())
        .collect::<Vec<_>>();
    let next = match selected_id {
        Some(id) => visible_ids
            .iter()
            .position(|candidate| candidate == &id)
            .unwrap_or_else(|| selected.min(visible.len().saturating_sub(1))),
        None => selected.min(visible.len().saturating_sub(1)),
    };
    list.selected = next;
    list.sync(visible.len());
    let rows = list_card_child_rows(&label, &config_key, source, items, &visible);
    let section = RowSection {
        label: label.clone(),
        description: None,
        rows: (0..rows.len()).collect(),
        source,
    };
    level.rows = rows;
    level.sections = vec![section];
    level.row_bounds = (0..level.rows.len())
        .map(|_| Rc::new(Cell::new(None)))
        .collect();
    level.selected = next;
}

fn live_card_level(
    label: &str,
    description: Option<String>,
    config_key: &str,
    source: usize,
    control: RowControl,
    origin_row: usize,
) -> Level {
    let row = Row {
        id: "live_card_body".into(),
        section_id: None,
        section_label: None,
        label: label.to_string(),
        description,
        placeholder: None,
        variant: None,
        config_key: config_key.to_string(),
        default: qol_config::contract::FieldDefault::String(String::new()),
        stream: None,
        action: None,
        visibility: None,
        source,
        control,
    };
    let section = RowSection {
        label: label.to_string(),
        description: None,
        rows: vec![0],
        source,
    };
    Level {
        rows: vec![row],
        sections: vec![section],
        selected: 0,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds: vec![Rc::new(Cell::new(None))],
        title: Some(label.to_string()),
        origin_row: Some(origin_row),
        object_array: None,
        list_card: false,
        live_card: true,
    }
}

fn live_card_sync(root_rows: &mut [Row], level: &mut Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let Some(parent) = root_rows.get(origin_row) else {
        return;
    };
    let Some(row) = level.rows.first_mut() else {
        return;
    };
    match (&parent.control, &mut row.control) {
        (
            RowControl::Gamepad { query, monitor },
            RowControl::Gamepad {
                query: row_query,
                monitor: row_monitor,
            },
        ) => {
            *row_query = query.clone();
            *row_monitor = monitor.clone();
        }
        (
            RowControl::QrCode {
                query,
                value_from,
                url,
                modules,
                error,
            },
            RowControl::QrCode {
                query: row_query,
                value_from: row_value_from,
                url: row_url,
                modules: row_modules,
                error: row_error,
            },
        ) => {
            *row_query = query.clone();
            *row_value_from = value_from.clone();
            *row_url = url.clone();
            *row_modules = modules.clone();
            *row_error = error.clone();
        }
        _ => {}
    }
}

fn live_card_sync_back(root_rows: &mut [Row], level: &Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let Some(parent) = root_rows.get_mut(origin_row) else {
        return;
    };
    let Some(row) = level.rows.first() else {
        return;
    };
    if let (
        RowControl::Gamepad {
            monitor: root_monitor,
            ..
        },
        RowControl::Gamepad {
            monitor: card_monitor,
            ..
        },
    ) = (&mut parent.control, &row.control)
    {
        *root_monitor = card_monitor.clone();
    }
}

fn row_matches(row: &Row, needle: &str) -> bool {
    row.label.to_lowercase().contains(needle)
        || row
            .description
            .as_deref()
            .is_some_and(|text| text.to_lowercase().contains(needle))
}

fn bare_filter_seed(key: &str, key_char: Option<&str>) -> Option<String> {
    if matches!(
        key,
        "space"
            | "enter"
            | "return"
            | "escape"
            | "tab"
            | "backspace"
            | "up"
            | "down"
            | "left"
            | "right"
    ) {
        return None;
    }
    key_char
        .filter(|text| text.chars().count() == 1 && !text.chars().any(char::is_control))
        .map(str::to_string)
}

pub(super) fn rail_open(sources: usize, filtering: bool) -> bool {
    sources > 1 && !filtering
}

fn rail_group_breaks(groups: &[PanelSourceGroup]) -> Vec<usize> {
    groups
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| (pair[0] != pair[1]).then_some(index + 1))
        .collect()
}

fn filtered_visible_rows(
    rows: &[Row],
    sections: &[RowSection],
    source_count: usize,
    needle: Option<&str>,
    selected_source: usize,
) -> Vec<usize> {
    let filtering = needle.is_some();
    let mut out = Vec::new();
    for source in 0..source_count {
        if !filtering && source != selected_source {
            continue;
        }
        for section in sections.iter().filter(|section| section.source == source) {
            let visible = section
                .rows
                .iter()
                .copied()
                .filter(|index| super::rows::row_is_visible(rows, *index));
            match needle {
                Some(needle) => {
                    out.extend(visible.filter(|index| row_matches(&rows[*index], needle)));
                }
                None => out.extend(visible),
            }
        }
    }
    out
}

fn clamp_selected(visible: &[usize], selected: usize) -> usize {
    if visible.contains(&selected) {
        selected
    } else {
        visible.first().copied().unwrap_or(selected)
    }
}

fn push_level(stack: &mut Vec<Level>, child: Level) {
    stack.push(child);
}

fn pop_level(stack: &mut Vec<Level>) -> Option<Level> {
    if stack.len() == 1 {
        return None;
    }
    stack.pop()
}

#[derive(Debug, PartialEq)]
enum EscapeStep {
    CloseFilter,
    PopCard,
    AscendRail,
    Dismiss,
}

fn escape_step(depth: usize, filter_open: bool, rail_can_ascend: bool) -> EscapeStep {
    if filter_open {
        return EscapeStep::CloseFilter;
    }
    if depth > 0 {
        return EscapeStep::PopCard;
    }
    if rail_can_ascend {
        return EscapeStep::AscendRail;
    }
    EscapeStep::Dismiss
}

fn focus_enters_the_body(plugin_id: &str) -> bool {
    plugin_id != qol_conventions::CORE_PANEL_ID
}

fn crumb_labels(heading: &str, plugin: Option<String>, parents: Vec<String>) -> Vec<String> {
    let mut labels = vec![heading.to_string()];
    labels.extend(plugin.filter(|title| title != heading));
    labels.extend(parents);
    labels
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}

fn header_is_redundant(title: &str, labels: &[&str]) -> bool {
    if labels.len() != 1 {
        return false;
    }
    title.trim().to_lowercase() == labels[0].trim().to_lowercase()
}

fn value_cell_width(control: &RowControl) -> f32 {
    match control {
        RowControl::Toggle(_) => 92.0,
        RowControl::Number { .. } => 33.0,
        RowControl::Color(_) => 76.0,
        RowControl::Select { .. } | RowControl::MultiSelect { .. } => 96.0,
        RowControl::Text(_) => 120.0,
        RowControl::TextList(_) => 160.0,
        RowControl::Action { .. } => 90.0,
        RowControl::Status { .. } => 130.0,
        RowControl::ObjectArray(_) => 60.0,
        RowControl::List { .. } => 120.0,
        RowControl::QrCode { .. } => 260.0,
        RowControl::Unsupported { .. } => 200.0,
        RowControl::Gamepad { .. } => 200.0,
    }
}

fn list_header_height(row: &Row) -> f32 {
    super::PANEL_LIST_HEADER_HEIGHT
        + if row.description.is_some() {
            super::PANEL_LIST_DESCRIPTION_HEIGHT
        } else {
            0.0
        }
}

fn qr_side(modules: &[bool]) -> usize {
    (modules.len() as f64).sqrt() as usize
}

fn qr_module_px(modules: &[bool]) -> f32 {
    let side = qr_side(modules);
    let module = super::PANEL_QR_CODE_HEIGHT / side as f32;
    module.clamp(2.0, 4.0)
}

fn adjacent_visible_row(visible: &[usize], selected: usize, direction: isize) -> usize {
    let Some(position) = visible.iter().position(|index| *index == selected) else {
        return visible.first().copied().unwrap_or(0);
    };
    let next = if direction < 0 {
        position.saturating_sub(1)
    } else {
        (position + 1).min(visible.len() - 1)
    };
    visible[next]
}

#[cfg(test)]
mod tests {
    use super::{
        action_refresh_payload, action_shows_spinner, action_value_label, adjacent_visible_row,
        binary_state_label, clamp_selected, color_display, crumb_labels, due_query_indices,
        escape_step, focus_level, format_number, header_is_redundant, horizontal_step_direction,
        intent, list_action_affordance, list_card_slider_value, list_fit_updates,
        list_slider_value, live_card_level, live_card_sync, live_card_sync_back, number_preview,
        number_unit, parsed_color, parsed_number, pop_level, push_level, query_is_due,
        rail_group_breaks, row_body_height, selected_list_item, slider_fraction,
        slider_percent_label, slider_value_from_fraction, source_window_height_for,
        step_list_slider, stepped_number, stepped_slider_value, text_or_placeholder,
        transition_in_flight, transition_policy, ChipRowPart, EscapeStep, HeightCache, Intent,
        Level, ObjectArrayState, Row, RowControl, RowSection, SliderHold, TransitionAction,
        TransitionTracker,
    };
    use crate::gamepad::GamepadMonitor;
    use crate::phantom_nav::{NavAxis, PhantomNavGuard};
    use crate::scroll_list::ScrollList;
    use crate::settings_panel::object_array_row::{Chip, ChipTone, Entry, Item, ItemChips};
    use crate::settings_panel::rows::{rows_from_resolved, visible_row_indices};
    use crate::settings_panel::PanelSourceGroup;

    #[test]
    fn rail_separator_only_marks_the_core_to_plugin_boundary() {
        assert_eq!(
            rail_group_breaks(&[
                PanelSourceGroup::Core,
                PanelSourceGroup::Core,
                PanelSourceGroup::Core,
                PanelSourceGroup::Plugin,
                PanelSourceGroup::Plugin,
            ]),
            vec![3]
        );
        assert!(rail_group_breaks(&[PanelSourceGroup::Core]).is_empty());
    }

    fn rows(headers: &[bool]) -> Vec<Row> {
        headers
            .iter()
            .map(|header| Row {
                id: "field".into(),
                section_id: header.then(|| "section".to_string()),
                section_label: header.then(|| "Section".to_string()),
                label: "Label".into(),
                description: None,
                placeholder: None,
                variant: None,
                config_key: "key".into(),
                default: qol_config::contract::FieldDefault::String(String::new()),
                stream: None,
                action: None,
                visibility: None,
                source: 0,
                control: RowControl::Toggle(false),
            })
            .collect()
    }

    #[test]
    fn long_descriptions_stay_inside_the_fixed_row_height() {
        let icon = Row {
            id: "icon".into(),
            section_id: None,
            section_label: None,
            label: "Icon".into(),
            description: Some("Where the app icon appears over each window preview.".into()),
            placeholder: None,
            variant: None,
            config_key: "display.icon_position".into(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            source: 0,
            control: RowControl::Select {
                options: vec![],
                index: 0,
                dynamic: None,
            },
        };
        let card = Row {
            id: "card".into(),
            section_id: None,
            section_label: None,
            label: "Card".into(),
            description: Some(
                "Scale multiplier for window cards and previews. 1.0 is the compact legacy size."
                    .into(),
            ),
            placeholder: None,
            variant: None,
            config_key: "display.card_scale".into(),
            default: qol_config::contract::FieldDefault::Number(1.5),
            stream: None,
            action: None,
            visibility: None,
            source: 0,
            control: RowControl::Number {
                value: 1.5,
                min: None,
                max: None,
                step: None,
            },
        };
        let dynamic = Row {
            id: "dynamic".into(),
            section_id: None,
            section_label: None,
            label: "Dynamic".into(),
            description: Some(
                "Grow window cards to fill free space when few windows are open; shrink to fit when many are."
                    .into(),
            ),
            placeholder: None,
            variant: None,
            config_key: "display.dynamic_card_scale".into(),
            default: qol_config::contract::FieldDefault::Boolean(false),
            stream: None,
            action: None,
            visibility: None,
            source: 0,
            control: RowControl::Toggle(false),
        };
        assert_eq!(
            row_body_height(&icon, false),
            super::super::PANEL_ROW_HEIGHT
        );
        assert_eq!(
            row_body_height(&card, false),
            super::super::PANEL_ROW_HEIGHT
        );
        assert_eq!(
            row_body_height(&dynamic, false),
            super::super::PANEL_ROW_HEIGHT
        );
    }

    #[test]
    fn color_display_normalizes_the_hash_prefix_like_the_web_save() {
        let cases = [
            ("ff0000", "#ff0000"),
            ("#202322", "#202322"),
            ("#FFFFFF", "#FFFFFF"),
        ];
        for (value, expected) in cases {
            assert_eq!(color_display(value), expected, "value: {value}");
        }
    }

    #[test]
    fn format_number_matches_the_web_value_formatting() {
        let cases = [
            (1.7000000000000002, "1.7"),
            (1.5, "1.5"),
            (0.65, "0.65"),
            (1.85, "1.85"),
            (6.0, "6"),
            (1.234567, "1.2346"),
            (2.5, "2.5"),
        ];
        for (value, expected) in cases {
            assert_eq!(format_number(value), expected, "value: {value}");
        }
    }

    #[test]
    fn the_panel_filter_reads_both_the_label_and_the_description() {
        let mut row = list_row();
        row.label = "Excluded Apps".into();
        row.description = Some("Remapping is ignored in these apps.".into());

        assert!(super::row_matches(&row, "excluded"));
        assert!(super::row_matches(&row, "ignored"));
        assert!(!super::row_matches(&row, "brightness"));
    }

    fn labeled_row(label: &str, source: usize) -> Row {
        let mut row = rows(&[false]).remove(0);
        row.label = label.into();
        row.source = source;
        row
    }

    fn source_section(label: &str, source: usize, rows: Vec<usize>) -> RowSection {
        RowSection {
            label: label.into(),
            description: None,
            rows,
            source,
        }
    }

    #[test]
    fn a_bare_letter_opens_the_filter_and_seeds_it() {
        assert_eq!(super::bare_filter_seed("a", Some("a")), Some("a".into()));
        assert_eq!(super::bare_filter_seed("7", Some("7")), Some("7".into()));
        assert_eq!(super::bare_filter_seed("ä", Some("ä")), Some("ä".into()));
        for (key, ch, label) in [
            ("space", Some(" "), "space activates the row"),
            ("enter", Some("\r"), "enter activates the row"),
            ("return", Some("\r"), "return activates the row"),
            ("escape", Some("\u{1b}"), "escape closes the panel"),
            ("tab", Some("\t"), "tab moves focus"),
            ("backspace", None, "backspace is not filter text"),
            ("up", None, "arrows navigate"),
            ("down", None, "arrows navigate"),
            ("left", None, "arrows navigate"),
            ("right", None, "arrows navigate"),
        ] {
            assert_eq!(super::bare_filter_seed(key, ch), None, "{label}");
        }
    }

    #[test]
    fn a_needle_that_matches_nothing_shows_nothing_rather_than_the_whole_panel() {
        let rows = vec![labeled_row("Reconnect", 0), labeled_row("Brightness", 1)];
        let sections = vec![
            source_section("Devices", 0, vec![0]),
            source_section("Display", 1, vec![1]),
        ];
        assert!(
            super::filtered_visible_rows(&rows, &sections, 2, Some("zzz"), 0).is_empty(),
            "a needle with no hits must leave the body empty, not fall back to the unfiltered panel"
        );
        assert!(
            !super::rail_open(2, true),
            "the rail stays away while a query is on screen, hits or no hits"
        );
    }

    #[test]
    fn a_filter_needle_gathers_matching_rows_from_every_source() {
        let rows = vec![
            labeled_row("Reconnect", 0),
            labeled_row("Reconnect interval", 1),
            labeled_row("Brightness", 1),
        ];
        let sections = vec![
            source_section("Devices", 0, vec![0]),
            source_section("Display", 1, vec![1, 2]),
        ];
        assert_eq!(
            super::filtered_visible_rows(&rows, &sections, 2, Some("reconnect"), 0),
            vec![0, 1],
            "a needle matching two sources must return rows from both"
        );
        assert_eq!(
            super::filtered_visible_rows(&rows, &sections, 2, None, 0),
            vec![0],
            "without a needle only the selected source's rows are visible"
        );
    }

    #[test]
    fn filtering_suppresses_the_rail() {
        assert!(!super::rail_open(2, true));
        assert!(super::rail_open(2, false));
        assert!(!super::rail_open(1, false));
        assert!(!super::rail_open(1, true));
    }

    #[test]
    fn escape_clears_the_filter_and_lands_back_on_the_selected_source() {
        let rows = vec![
            labeled_row("Reconnect", 0),
            labeled_row("Reconnect interval", 1),
        ];
        let sections = vec![
            source_section("Devices", 0, vec![0]),
            source_section("Display", 1, vec![1]),
        ];
        let mut selected = 1;
        let filtering = super::filtered_visible_rows(&rows, &sections, 2, Some("reconnect"), 0);
        assert_eq!(filtering, vec![0, 1]);
        selected = super::clamp_selected(&filtering, selected);
        assert_eq!(selected, 1);
        let cleared = super::filtered_visible_rows(&rows, &sections, 2, None, 0);
        assert_eq!(cleared, vec![0]);
        selected = super::clamp_selected(&cleared, selected);
        assert_eq!(
            selected, 0,
            "the cursor must land on the selected source's row"
        );
        assert_eq!(rows[selected].source, 0);
    }

    #[test]
    fn a_group_header_offsets_every_row_it_precedes_in_the_scroller() {
        let sections = [vec![0usize, 1], Vec::new(), vec![2, 3, 4]];

        assert_eq!(
            super::body_child_offset(&sections, &[true, true, true], 0),
            Some(1)
        );
        assert_eq!(
            super::body_child_offset(&sections, &[true, true, true], 1),
            Some(2)
        );
        assert_eq!(
            super::body_child_offset(&sections, &[true, true, true], 2),
            Some(4)
        );
        assert_eq!(
            super::body_child_offset(&sections, &[true, true, true], 4),
            Some(6)
        );
        assert_eq!(
            super::body_child_offset(&sections, &[true, true, true], 5),
            None
        );
    }

    #[test]
    fn counts_name_their_noun_in_the_right_number() {
        assert_eq!(super::plural(1, "setting"), "setting");
        assert_eq!(super::plural(0, "setting"), "settings");
        assert_eq!(super::plural(13, "item"), "items");
    }

    fn list_row() -> Row {
        Row {
            id: "items".into(),
            section_id: None,
            section_label: None,
            label: "Items".into(),
            description: None,
            placeholder: None,
            variant: None,
            config_key: "items".into(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            source: 0,
            control: RowControl::List {
                query: "items".into(),
                filter: String::new(),
                active_query: None,
                active_value_from: None,
                active_label: None,
                active: false,
                row_label: "{name}".into(),
                row_subtitle: Some("{detail}".into()),
                actions: Box::new(super::super::rows::ListActions {
                    primary: None,
                    additional: Vec::new(),
                }),
                slider: None,
                items: Vec::new(),
                list: ScrollList::new(super::super::rows::LIST_MAX_VISIBLE),
                error: None,
            },
        }
    }

    #[test]
    fn descriptions_never_change_row_height() {
        let mut plain = rows(&[false]).remove(0);
        let plain_height = row_body_height(&plain, false);
        plain.description = Some("Helpful context".into());

        assert_eq!(plain_height, super::super::PANEL_ROW_HEIGHT);
        assert_eq!(
            row_body_height(&plain, false),
            super::super::PANEL_ROW_HEIGHT
        );

        let mut list = list_row();
        let plain_list_height = row_body_height(&list, false);
        list.description = Some("Live devices".into());
        assert_eq!(plain_list_height, super::super::PANEL_ROW_HEIGHT);
        assert_eq!(
            row_body_height(&list, false),
            super::super::PANEL_ROW_HEIGHT
        );
    }

    #[test]
    fn every_control_rests_at_the_fixed_row_height() {
        let spec = qol_config::contract::parse_spec_str(
            r#"
schema_version = 1

[field.toggle]
type = "boolean"
default = true

[field.number]
type = "number"
default = 1

[field.text]
type = "string"
default = "d"

[field.text_list]
type = "string_array"
default = []

[field.multi_select]
type = "string_array"
options = ["a", "b"]
default = ["a"]

[field.select]
type = "select"
options = ["a", "b"]
default = "a"

[field.color]
type = "color"
default = "202322"

[field.action]
type = "action"
action = "go"

[field.status]
type = "status"
query = "status_query"
value_from = "state"

[field.list]
type = "list"
query = "list_query"
row_label = "{name}"

[field.object_array]
type = "object_array"
default = []

[field.object_array.item.fields]
app = "string"

[field.gamepad]
type = "gamepad"
query = "controller_input"

[field.qr]
type = "qr_code"
query = "connection_info"
"#,
        )
        .unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved, 0);
        assert_eq!(rows.len(), 13);
        for row in &rows {
            assert_eq!(
                row_body_height(row, false),
                super::super::PANEL_ROW_HEIGHT,
                "{} must rest at the fixed row height",
                row.id
            );
        }
        let broken = qol_config::contract::parse_spec_str(
            "schema_version = 1\n\n[field.broken]\ntype = \"boolean\"\ndefault = true\n",
        )
        .unwrap();
        let resolved = qol_config::normalized::resolve_config(
            &broken,
            &serde_json::json!({ "broken": "yes" }),
        )
        .unwrap();
        let rows = rows_from_resolved(&resolved, 0);
        assert_eq!(
            row_body_height(&rows[0], false),
            super::super::PANEL_ROW_HEIGHT
        );
    }

    #[test]
    fn keyboard_navigation_skips_conditional_rows() {
        const SPEC: &str = r#"
schema_version = 1

[field.enabled]
type = "boolean"
default = false

[field.detail]
type = "number"
default = 4

[field.detail.show_when]
field = "enabled"
equals = true

[field.always]
type = "string"
default = "visible"
"#;
        let spec = qol_config::contract::parse_spec_str(SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved, 0);
        let visible = visible_row_indices(&rows);

        assert_eq!(adjacent_visible_row(&visible, 0, 1), 2);
        assert_eq!(adjacent_visible_row(&visible, 2, -1), 0);
    }

    #[test]
    fn escape_is_the_only_key_that_goes_back() {
        for key in ["left", "right", "up", "down"] {
            assert_ne!(intent(key, None, false), Some(Intent::Close), "key: {key}");
        }
        assert_eq!(intent("escape", None, false), Some(Intent::Close));
    }

    #[test]
    fn binary_runtime_and_config_states_share_on_off_labels() {
        assert_eq!(binary_state_label(true), "On");
        assert_eq!(binary_state_label(false), "Off");
    }

    #[test]
    fn action_values_distinguish_commands_from_semantic_runtime_state() {
        let no_labels = std::collections::BTreeMap::new();
        let cases = [
            (false, false, false, false, false, "Run"),
            (false, true, false, false, false, "working\u{2026}"),
            (false, false, true, false, false, "failed"),
            (false, false, false, true, true, "Run"),
            (true, false, false, true, true, "Stop"),
            (true, false, false, true, false, "Active"),
        ];
        for (active, pending, failed, runtime, reversible, expected) in cases {
            assert_eq!(
                action_value_label(active, pending, failed, runtime, reversible, &no_labels),
                expected
            );
        }

        let labels = std::collections::BTreeMap::from([
            ("false".into(), "Light".into()),
            ("true".into(), "Dark".into()),
        ]);
        assert_eq!(
            action_value_label(false, false, false, true, false, &labels),
            "Light"
        );
        assert_eq!(
            action_value_label(true, false, false, true, false, &labels),
            "Dark"
        );
    }

    #[test]
    fn action_refresh_uses_only_payloads_that_answer_the_active_query() {
        let cases = [
            (
                Ok(Some(serde_json::json!({"dark": true}))),
                Some("dark"),
                Some(serde_json::json!({"dark": true})),
            ),
            (
                Ok(Some(serde_json::json!({"dark": false}))),
                Some("dark"),
                Some(serde_json::json!({"dark": false})),
            ),
            (
                Ok(Some(serde_json::json!({"scheme": "dark"}))),
                Some("dark"),
                None,
            ),
            (
                Ok(Some(serde_json::json!({"dark": "yes"}))),
                Some("dark"),
                None,
            ),
            (
                Ok(Some(serde_json::json!(true))),
                None,
                Some(serde_json::json!(true)),
            ),
            (Ok(None), Some("dark"), None),
            (Err("failed".into()), Some("dark"), None),
        ];
        for (result, path, expected) in cases {
            assert_eq!(action_refresh_payload(&result, path), expected);
        }
    }

    #[test]
    fn every_navigation_level_reads_from_one_focus_rule() {
        use super::PanelFocus::{Body, Sources};
        let cases = [
            (true, 4, Sources),
            (true, 2, Sources),
            (true, 1, Body),
            (true, 0, Body),
            (false, 4, Body),
            (false, 1, Body),
        ];
        for (source_menu, sources, expected) in cases {
            assert_eq!(
                focus_level(source_menu, sources),
                expected,
                "source_menu={source_menu} sources={sources}"
            );
        }
    }

    #[test]
    fn toggle_actions_show_state_without_a_permanent_spinner() {
        let mut row = rows(&[false]).remove(0);
        row.variant = Some("toggle".into());
        row.control = RowControl::Action {
            action: "enable_adapter".into(),
            active_action: Some("disable_adapter".into()),
            active_label: Some("Bluetooth".into()),
            active_query: Some("adapter_status".into()),
            active_value_from: Some("powered".into()),
            state_labels: std::collections::BTreeMap::new(),
            active: true,
            pending: false,
            error: None,
        };
        assert!(!action_shows_spinner(&row));

        let RowControl::Action { pending, .. } = &mut row.control else {
            unreachable!();
        };
        *pending = true;
        assert!(action_shows_spinner(&row));
    }

    #[test]
    fn list_action_affordance_exposes_additional_action_count() {
        let cases = [
            ("Connect", 1, "Connect"),
            ("Disconnect", 2, "Disconnect +1"),
            ("Pair", 6, "Pair +5"),
        ];
        for (primary, count, expected) in cases {
            assert_eq!(list_action_affordance(primary, count), expected);
        }
    }

    #[test]
    fn parsed_number_parses_clamps_and_rejects() {
        let cases = [
            ("18", None, None, None, Some(18.0)),
            (" 23.5 ", None, None, None, Some(23.5)),
            ("-4", Some(0.0), Some(51.0), None, Some(0.0)),
            ("99", Some(0.0), Some(51.0), None, Some(51.0)),
            ("abc", None, None, None, None),
            ("", None, None, None, None),
            ("inf", None, None, None, None),
        ];
        for (edit, min, max, step, expected) in cases {
            assert_eq!(
                parsed_number(edit, min, max, step),
                expected,
                "edit: {edit:?}"
            );
        }
    }

    #[test]
    fn typed_numbers_align_to_the_contract_step() {
        let cases = [
            ("0.649", Some(0.1), Some(1.0), Some(0.01), Some(0.65)),
            ("0.8", Some(0.1), Some(1.0), Some(0.05), Some(0.8)),
            ("0.7", Some(0.1), Some(1.0), Some(0.05), Some(0.7)),
            ("1250", Some(100.0), Some(4000.0), Some(100.0), Some(1300.0)),
            ("1200", Some(100.0), Some(4000.0), Some(100.0), Some(1200.0)),
            ("99", Some(0.0), Some(51.0), Some(2.0), Some(50.0)),
            ("2.34", None, None, Some(1.0), Some(2.0)),
            ("2.5", None, None, Some(1.0), Some(3.0)),
            ("0.33", Some(0.0), Some(1.0), Some(0.1), Some(0.3)),
            ("0.1", Some(0.1), Some(1.0), Some(0.01), Some(0.1)),
        ];
        for (edit, min, max, step, expected) in cases {
            assert_eq!(
                parsed_number(edit, min, max, step),
                expected,
                "edit: {edit:?} step: {step:?}"
            );
        }
    }

    #[test]
    fn activated_numbers_step_and_clamp_without_persisting_partial_text() {
        let cases = [
            ("4", 4.0, Some(0.0), Some(10.0), 2.0, 1.0, "6"),
            ("4", 4.0, Some(0.0), Some(10.0), 2.0, -1.0, "2"),
            ("10", 10.0, Some(0.0), Some(10.0), 2.0, 1.0, "10"),
            ("invalid", 4.0, None, None, 0.5, 1.0, "4.5"),
        ];
        for (edit, fallback, min, max, step, direction, expected) in cases {
            assert_eq!(
                stepped_number(edit, fallback, min, max, step, direction),
                expected,
                "edit: {edit:?} direction: {direction}"
            );
        }
    }

    #[test]
    fn activated_numbers_use_horizontal_arrows_for_nudging() {
        let cases = [
            ("left", Some(-1.0)),
            ("right", Some(1.0)),
            ("up", None),
            ("down", None),
            ("enter", None),
        ];
        for (key, expected) in cases {
            assert_eq!(horizontal_step_direction(key), expected, "key: {key}");
        }
    }

    #[test]
    fn active_number_edits_drive_the_slider_preview() {
        let cases = [
            (Some("6"), 4.0, Some(0.0), Some(10.0), None, 6.0),
            (Some("20"), 4.0, Some(0.0), Some(10.0), None, 10.0),
            (Some("invalid"), 4.0, Some(0.0), Some(10.0), None, 4.0),
            (None, 4.0, Some(0.0), Some(10.0), None, 4.0),
        ];
        for (edit, fallback, min, max, step, expected) in cases {
            assert_eq!(
                number_preview(edit, fallback, min, max, step),
                expected,
                "edit: {edit:?}"
            );
        }
    }

    #[test]
    fn slider_fraction_clamps_and_requires_a_valid_range() {
        let cases = [
            (50.0, Some(0.0), Some(100.0), 0.5),
            (-1.0, Some(0.0), Some(100.0), 0.0),
            (120.0, Some(0.0), Some(100.0), 1.0),
            (4.0, None, Some(8.0), 0.0),
            (4.0, Some(8.0), Some(8.0), 0.0),
        ];
        for (value, min, max, expected) in cases {
            assert_eq!(
                slider_fraction(value, min, max),
                expected,
                "value={value} min={min:?} max={max:?}"
            );
        }
    }

    #[test]
    fn row_slider_steps_align_and_clamp_to_the_contract_range() {
        let cases = [
            (50.0, 1.0, 0.0, 100.0, 10.0, 60.0),
            (50.0, -1.0, 0.0, 100.0, 10.0, 40.0),
            (95.0, 1.0, 0.0, 100.0, 10.0, 100.0),
            (5.0, -1.0, 0.0, 100.0, 10.0, 0.0),
            (0.0, -1.0, 0.0, 100.0, 10.0, 0.0),
            (0.25, 1.0, 0.0, 1.0, 0.1, 0.4),
            (0.99, 1.0, 0.0, 1.0, 0.1, 1.0),
        ];
        for (current, direction, min, max, step, expected) in cases {
            assert_eq!(
                stepped_slider_value(current, direction, min, max, step),
                expected,
                "current={current} direction={direction} range={min}..{max} step={step}"
            );
        }
    }

    #[test]
    fn row_slider_click_fractions_map_to_aligned_values_and_percents() {
        let value_cases = [
            (0.0, 100.0, 10.0, 0.5, 50.0),
            (0.0, 100.0, 10.0, 0.33, 30.0),
            (0.0, 100.0, 10.0, 0.67, 70.0),
            (0.0, 100.0, 10.0, 0.0, 0.0),
            (0.0, 100.0, 10.0, 1.0, 100.0),
            (5.0, 250.0, 5.0, 0.4, 105.0),
            (5.0, 250.0, 5.0, 0.0, 5.0),
        ];
        for (min, max, step, fraction, expected) in value_cases {
            assert_eq!(
                slider_value_from_fraction(min, max, step, fraction),
                expected,
                "min={min} max={max} step={step} fraction={fraction}"
            );
        }

        let percent_cases = [
            (0.0, 100.0, 42.0, "42%"),
            (0.0, 100.0, 0.0, "0%"),
            (0.0, 100.0, 100.0, "100%"),
            (5.0, 250.0, 105.0, "41%"),
            (5.0, 250.0, 5.0, "0%"),
            (0.0, 1.0, 0.25, "25%"),
        ];
        for (min, max, value, expected) in percent_cases {
            assert_eq!(
                slider_percent_label(min, max, value),
                expected,
                "min={min} max={max} value={value}"
            );
        }
    }

    #[test]
    fn compact_values_reuse_contract_placeholders_and_field_units() {
        let placeholder_cases = [
            ("value", Some("hint"), "value"),
            ("", Some("hint"), "hint"),
            ("", None, "Empty"),
        ];
        for (value, placeholder, expected) in placeholder_cases {
            assert_eq!(text_or_placeholder(value, placeholder), expected);
        }

        let unit_cases = [
            ("width_percent", Some("%")),
            ("padding_px", Some("px")),
            ("preview_pixels", Some("px")),
            ("onset_ms", Some("ms")),
            ("retry_seconds", Some("s")),
            ("count", None),
        ];
        for (field, expected) in unit_cases {
            assert_eq!(number_unit(field), expected, "field: {field}");
        }
    }

    #[test]
    fn parsed_color_accepts_six_digit_hex_with_optional_hash() {
        let cases = [
            ("#202322", Some(0x202322)),
            ("202322", Some(0x202322)),
            (" #ffffff ", Some(0xffffff)),
            ("AABBCC", Some(0xaabbcc)),
            ("#fff", None),
            ("#2023221", None),
            ("20232g", None),
            ("", None),
        ];
        for (text, expected) in cases {
            assert_eq!(parsed_color(text), expected, "text: {text:?}");
        }
    }

    #[test]
    fn right_then_left_within_phantom_window_steps_slider_once() {
        let mut guard = PhantomNavGuard::new();
        let (min, max, step) = (0.0, 100.0, 10.0);
        let mut value = 50.0;

        assert!(!guard.swallow(NavAxis::Horizontal, 1.0));
        value = stepped_slider_value(value, 1.0, min, max, step);
        assert_eq!(value, 60.0);

        assert!(
            guard.swallow(NavAxis::Horizontal, -1.0),
            "a left arriving within the phantom window must be swallowed"
        );
        assert_eq!(
            value, 60.0,
            "the slider holds at one step after a real right and a phantom left"
        );
    }

    #[test]
    fn down_then_up_within_phantom_window_moves_list_selection_once() {
        let mut guard = PhantomNavGuard::new();
        let mut list = ScrollList::new(5);

        assert!(!guard.swallow(NavAxis::Vertical, 1.0));
        list.move_down(7);
        list.sync(7);
        assert_eq!(list.selected, 1);

        let swallowed = guard.swallow(NavAxis::Vertical, -1.0);
        assert!(
            swallowed,
            "an up arriving within the phantom window must be swallowed"
        );
        if !swallowed {
            list.move_up();
        }
        list.sync(7);
        assert_eq!(
            list.selected, 1,
            "list selection holds against the phantom up"
        );
    }

    #[test]
    fn theme_override_values_reads_native_and_accent() {
        let values = serde_json::json!({
            "native_theme": "dark",
            "accent": "teal",
            "other": "field",
        });
        let (native, accent) = super::theme_override_values(&values);
        assert_eq!(native, Some("dark"));
        assert_eq!(accent, Some("teal"));
    }

    #[test]
    fn theme_override_values_treats_missing_fields_as_none() {
        let values = serde_json::json!({ "other": "field" });
        let (native, accent) = super::theme_override_values(&values);
        assert_eq!(native, None);
        assert_eq!(accent, None);
    }

    #[test]
    fn inactive_inputs_require_enter_before_control_keys_take_effect() {
        let cases = [
            ("up", None, false, Some(Intent::Up)),
            ("down", None, false, Some(Intent::Down)),
            ("left", None, false, None),
            ("right", None, false, None),
            ("space", None, false, Some(Intent::Activate)),
            ("5", Some("5"), false, None),
            ("-", Some("-"), false, None),
            (".", Some("."), false, None),
            ("a", Some("a"), false, None),
            ("enter", None, false, Some(Intent::Activate)),
            ("return", None, false, Some(Intent::Activate)),
            ("escape", None, false, Some(Intent::Close)),
            ("enter", None, true, Some(Intent::CommitEdit)),
            ("return", None, true, Some(Intent::CommitEdit)),
            ("escape", None, true, Some(Intent::CancelEdit)),
            ("backspace", None, true, Some(Intent::Backspace)),
            ("a", Some("a"), true, Some(Intent::Insert("a".into()))),
        ];
        for (key, ch, editing, expected) in cases {
            assert_eq!(
                intent(key, ch, editing),
                expected,
                "key {key} editing {editing}"
            );
        }
    }

    #[test]
    fn only_enter_and_space_activate_a_row() {
        assert_eq!(intent("space", None, false), Some(Intent::Activate));
        assert_eq!(intent("enter", None, false), Some(Intent::Activate));
        assert_eq!(intent("right", None, false), None);
        assert_eq!(intent("left", None, false), None);
        assert_eq!(
            intent("space", Some(" "), true),
            Some(Intent::Insert(" ".into()))
        );
    }

    #[test]
    fn header_is_redundant_matches_a_single_identical_row_label() {
        assert!(header_is_redundant("Character Rules", &["Character Rules"]));
    }

    #[test]
    fn header_is_redundant_ignores_case_and_padding() {
        assert!(header_is_redundant(
            " character RULES ",
            &["Character Rules"]
        ));
        assert!(header_is_redundant(
            "Character Rules",
            &["  character rules  "]
        ));
    }

    #[test]
    fn header_is_redundant_is_false_for_a_differing_label() {
        assert!(!header_is_redundant("Character Rules", &["Key Remapping"]));
    }

    #[test]
    fn header_is_redundant_is_false_with_more_than_one_row() {
        assert!(!header_is_redundant(
            "Character Rules",
            &["Character Rules", "Key Remapping"]
        ));
    }

    #[test]
    fn header_is_redundant_is_false_without_rows() {
        assert!(!header_is_redundant("Character Rules", &[]));
    }

    fn level(selected: usize) -> Level {
        Level {
            rows: Vec::new(),
            sections: Vec::new(),
            selected,
            active_section: None,
            selected_section: 0,
            body_scroll: crate::scroll_list::SelectionScroll::new(),
            active_control: None,
            row_bounds: Vec::new(),
            title: None,
            origin_row: None,
            object_array: None,
            list_card: false,
            live_card: false,
        }
    }

    #[test]
    fn pop_restores_the_parent_level_untouched() {
        let mut parent = level(3);
        parent.rows = rows(&[false, false]);
        parent.sections = vec![source_section("Devices", 0, vec![0, 1])];
        let parent_sections = parent.sections.clone();
        let child = level(7);
        let mut stack = vec![parent, child];
        let popped = pop_level(&mut stack).unwrap();
        assert_eq!(popped.selected, 7);
        assert!(popped.sections.is_empty());
        let root = stack.last().unwrap();
        assert_eq!(root.selected, 3);
        assert_eq!(root.rows.len(), 2);
        assert_eq!(root.sections, parent_sections);
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn the_root_level_never_pops() {
        let mut stack = vec![level(3)];
        assert!(pop_level(&mut stack).is_none());
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].selected, 3);
    }

    #[test]
    fn push_then_pop_is_identity() {
        let mut stack = vec![level(3)];
        push_level(&mut stack, level(9));
        let popped = pop_level(&mut stack).unwrap();
        assert_eq!(popped.selected, 9);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].selected, 3);
    }

    fn gamepad_row() -> Row {
        Row {
            id: "input".into(),
            section_id: None,
            section_label: None,
            label: "Controller Input".into(),
            description: None,
            placeholder: None,
            variant: None,
            config_key: "input".into(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            source: 0,
            control: RowControl::Gamepad {
                query: "controller_input".into(),
                monitor: GamepadMonitor::default(),
            },
        }
    }

    fn qr_row() -> Row {
        Row {
            id: "pair".into(),
            section_id: None,
            section_label: None,
            label: "Pair".into(),
            description: None,
            placeholder: None,
            variant: None,
            config_key: "connection_info".into(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            source: 0,
            control: RowControl::QrCode {
                query: "connection_info".into(),
                value_from: Some("url".into()),
                url: Some("https://example.com/pair".into()),
                modules: vec![true, false, true, false],
                error: None,
            },
        }
    }

    #[test]
    fn a_live_card_level_names_the_row_and_carries_its_control() {
        let level = live_card_level(
            "Controller Input",
            Some("The active controller".into()),
            "input",
            1,
            RowControl::Gamepad {
                query: "controller_input".into(),
                monitor: GamepadMonitor::default(),
            },
            3,
        );
        assert_eq!(level.title.as_deref(), Some("Controller Input"));
        assert_eq!(level.origin_row, Some(3));
        assert!(level.live_card);
        assert!(!level.list_card);
        assert_eq!(level.selected, 0);
        assert_eq!(level.sections.len(), 1);
        assert_eq!(level.sections[0].rows, vec![0]);
        assert_eq!(level.rows.len(), 1);
        assert_eq!(level.rows[0].label, "Controller Input");
        assert_eq!(
            level.rows[0].description.as_deref(),
            Some("The active controller")
        );
        let RowControl::Gamepad { query, .. } = &level.rows[0].control else {
            panic!("expected gamepad control on the card");
        };
        assert_eq!(query, "controller_input");
    }

    #[test]
    fn live_card_sync_mirrors_root_gamepad_state_into_the_card() {
        let mut root = vec![gamepad_row()];
        let RowControl::Gamepad { monitor, .. } = &mut root[0].control else {
            panic!("expected gamepad monitor");
        };
        monitor.apply_query(Ok(serde_json::json!({
            "available": true,
            "items": [
                {
                    "name": "alpha",
                    "state": {"mapping": "standard", "buttons": [], "axes": []}
                },
                {
                    "name": "beta",
                    "state": {"mapping": "standard", "buttons": [], "axes": []}
                },
            ],
        })));
        let mut card = live_card_level(
            "Controller Input",
            None,
            "input",
            0,
            RowControl::Gamepad {
                query: "controller_input".into(),
                monitor: GamepadMonitor::default(),
            },
            0,
        );
        live_card_sync(&mut root, &mut card);
        let RowControl::Gamepad {
            monitor: card_monitor,
            ..
        } = &card.rows[0].control
        else {
            panic!("expected card gamepad monitor");
        };
        assert_eq!(
            card_monitor
                .selected()
                .map(|controller| controller.name.as_str()),
            Some("alpha")
        );
        let RowControl::Gamepad {
            monitor: root_monitor,
            ..
        } = &mut root[0].control
        else {
            panic!("expected root gamepad monitor");
        };
        root_monitor.select_next();
        live_card_sync(&mut root, &mut card);
        let RowControl::Gamepad {
            monitor: card_monitor,
            ..
        } = &card.rows[0].control
        else {
            panic!("expected card gamepad monitor");
        };
        assert_eq!(
            card_monitor
                .selected()
                .map(|controller| controller.name.as_str()),
            Some("beta")
        );
    }

    #[test]
    fn live_card_sync_mirrors_root_qr_state_into_the_card() {
        let mut root = vec![qr_row()];
        let mut card = live_card_level(
            "Pair",
            None,
            "connection_info",
            0,
            RowControl::QrCode {
                query: "connection_info".into(),
                value_from: None,
                url: None,
                modules: Vec::new(),
                error: Some("stale".into()),
            },
            0,
        );
        live_card_sync(&mut root, &mut card);
        let RowControl::QrCode {
            value_from,
            url,
            modules,
            error,
            ..
        } = &card.rows[0].control
        else {
            panic!("expected card qr code row");
        };
        assert_eq!(value_from.as_deref(), Some("url"));
        assert_eq!(url.as_deref(), Some("https://example.com/pair"));
        assert_eq!(modules, &vec![true, false, true, false]);
        assert!(error.is_none());
    }

    #[test]
    fn live_card_sync_back_returns_the_cycled_monitor_to_the_root() {
        let mut root = vec![gamepad_row()];
        let RowControl::Gamepad { monitor, .. } = &mut root[0].control else {
            panic!("expected gamepad monitor");
        };
        monitor.apply_query(Ok(serde_json::json!({
            "available": true,
            "items": [
                {
                    "name": "alpha",
                    "state": {"mapping": "standard", "buttons": [], "axes": []}
                },
                {
                    "name": "beta",
                    "state": {"mapping": "standard", "buttons": [], "axes": []}
                },
            ],
        })));
        let mut card = live_card_level(
            "Controller Input",
            None,
            "input",
            0,
            RowControl::Gamepad {
                query: "controller_input".into(),
                monitor: GamepadMonitor::default(),
            },
            0,
        );
        live_card_sync(&mut root, &mut card);
        let RowControl::Gamepad {
            monitor: card_monitor,
            ..
        } = &mut card.rows[0].control
        else {
            panic!("expected card gamepad monitor");
        };
        card_monitor.select_next();
        live_card_sync_back(&mut root, &card);
        let RowControl::Gamepad {
            monitor: root_monitor,
            ..
        } = &root[0].control
        else {
            panic!("expected root gamepad monitor");
        };
        assert_eq!(
            root_monitor
                .selected()
                .map(|controller| controller.name.as_str()),
            Some("beta")
        );
    }

    #[test]
    fn text_list_child_rows_builds_the_add_row_first_then_one_row_per_value() {
        let values = vec![
            "com.kitty.app".to_string(),
            "com.apple.Terminal".to_string(),
            "com.googlecode.iterm2".to_string(),
        ];
        let rows = super::text_list_child_rows("Excluded apps", "excluded_apps", 2, &values);
        assert_eq!(rows.len(), 4);
        let add = &rows[0];
        assert_eq!(add.label, "+ Add");
        assert!(matches!(&add.control, RowControl::Text(stored) if stored.is_empty()));
        for (index, value) in values.iter().enumerate() {
            let row = &rows[index + 1];
            assert_eq!(row.label, *value);
            assert!(matches!(&row.control, RowControl::Text(stored) if stored == value));
            assert_eq!(row.config_key, "excluded_apps");
            assert_eq!(row.source, 2);
        }
        let round_trip: Vec<String> = rows
            .iter()
            .filter_map(|row| match &row.control {
                RowControl::Text(value) => Some(value.clone()),
                _ => None,
            })
            .filter(|value| !value.is_empty())
            .collect();
        assert_eq!(round_trip, values);
    }

    #[test]
    fn deleting_the_first_real_text_list_item_removes_its_value() {
        let values = vec![
            "com.kitty.app".to_string(),
            "com.apple.Terminal".to_string(),
        ];
        let mut rows = super::text_list_child_rows("Excluded apps", "excluded_apps", 0, &values);
        let RowControl::Text(first) = &mut rows[1].control else {
            unreachable!();
        };
        *first = String::new();
        let kept: Vec<String> = rows
            .iter()
            .filter_map(|row| match &row.control {
                RowControl::Text(value) => Some(value.clone()),
                _ => None,
            })
            .filter(|value| !value.is_empty())
            .collect();
        assert_eq!(kept, vec!["com.apple.Terminal".to_string()]);
    }

    fn object_array_state() -> ObjectArrayState {
        ObjectArrayState::from_entries(
            None,
            vec![(
                "app".to_string(),
                qol_config::object_array::ItemFieldKind::Text,
            )],
            vec![
                Entry {
                    key: None,
                    fields: Item::from_iter([(
                        "app".to_string(),
                        qol_config::contract::FieldDefault::String("idea".into()),
                    )]),
                },
                Entry {
                    key: None,
                    fields: Item::from_iter([(
                        "app".to_string(),
                        qol_config::contract::FieldDefault::String("zed".into()),
                    )]),
                },
            ],
        )
    }

    #[test]
    fn object_array_card_rows_build_the_add_row_first_then_one_row_per_entry() {
        let state = object_array_state();
        let rows = super::object_array_child_rows("Rules", "rules", 1, &state);
        assert_eq!(rows.len(), 3);
        let add = &rows[0];
        assert_eq!(add.label, "+ Add");
        assert!(matches!(&add.control, RowControl::Text(stored) if stored.is_empty()));
        for (index, row) in rows.iter().skip(1).enumerate() {
            assert_eq!(row.label, state.summary(index));
            assert!(matches!(
                &row.control,
                RowControl::Text(stored) if stored == &state.summary(index)
            ));
            assert_eq!(row.config_key, "rules");
            assert_eq!(row.source, 1);
        }
    }

    #[test]
    fn an_empty_object_array_card_selects_the_add_row() {
        let empty = ObjectArrayState::from_entries(
            None,
            vec![(
                "app".to_string(),
                qol_config::object_array::ItemFieldKind::Text,
            )],
            Vec::new(),
        );
        let level = super::object_array_card_level("Rules", "rules", 0, empty, 3);
        assert_eq!(level.selected, 0);
        assert_eq!(level.object_array.as_ref().unwrap().list.selected, 0);

        let level = super::object_array_card_level("Rules", "rules", 0, object_array_state(), 3);
        assert_eq!(level.selected, 1);
        assert_eq!(level.object_array.as_ref().unwrap().list.selected, 1);
    }

    #[test]
    fn a_fresh_card_selects_the_first_item_or_the_add_row() {
        assert_eq!(super::initial_card_selection(0), 0);
        assert_eq!(super::initial_card_selection(3), 1);
    }

    #[test]
    fn removing_an_entry_rebuilds_the_card_rows_and_clamps_selection() {
        let state = object_array_state();
        let mut level = super::object_array_card_level("Rules", "rules", 0, state, 3);
        assert_eq!(level.selected, 1);
        level.selected = 2;
        let child = level.object_array.as_mut().unwrap();
        child.list.selected = 2;
        assert!(child.remove_selected());
        super::sync_object_array_level(&mut level, "Rules", "rules", 0);
        let child = level.object_array.as_ref().unwrap();
        assert_eq!(level.rows.len(), 2);
        assert_eq!(level.rows[0].label, "+ Add");
        assert_eq!(level.rows[1].label, child.summary(0));
        assert_eq!(level.selected, 1);
    }

    #[test]
    fn a_synced_root_row_holds_the_child_entries_in_order() {
        let mut root = level(0);
        root.rows = rows(&[false, false]);
        root.rows[1].control = RowControl::ObjectArray(ObjectArrayState::from_entries(
            None,
            vec![(
                "app".to_string(),
                qol_config::object_array::ItemFieldKind::Text,
            )],
            Vec::new(),
        ));
        let mut child =
            super::object_array_card_level("Rules", "rules", 0, object_array_state(), 1);
        super::object_array_card_sync(&mut root.rows, &mut child);
        let RowControl::ObjectArray(stored) = &root.rows[1].control else {
            panic!("root row holds an object array");
        };
        let entries = child.object_array.as_ref().unwrap().entries.clone();
        assert_eq!(stored.entries, entries);
    }

    #[test]
    fn an_entry_summary_spells_out_the_well_display() {
        let state = ObjectArrayState::from_entries(
            None,
            vec![
                (
                    "from_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "to_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "keys".to_string(),
                    qol_config::object_array::ItemFieldKind::StringArray,
                ),
            ],
            vec![Entry {
                key: None,
                fields: Item::from_iter([
                    (
                        "from_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["ctrl".into()]),
                    ),
                    (
                        "to_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["cmd".into()]),
                    ),
                    (
                        "keys".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["c".into()]),
                    ),
                ]),
            }],
        );
        assert_eq!(state.summary(0), "ctrl + c \u{2192} cmd + c");
    }

    #[test]
    fn an_entry_summary_compresses_shared_keys_like_the_well() {
        let state = ObjectArrayState::from_entries(
            None,
            vec![
                (
                    "from_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "to_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "keys".to_string(),
                    qol_config::object_array::ItemFieldKind::StringArray,
                ),
            ],
            vec![Entry {
                key: None,
                fields: Item::from_iter([
                    (
                        "from_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["ctrl".into()]),
                    ),
                    (
                        "to_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["cmd".into()]),
                    ),
                    (
                        "keys".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec![
                            "c".into(),
                            "v".into(),
                        ]),
                    ),
                ]),
            }],
        );
        assert_eq!(state.summary(0), "ctrl + 2 keys \u{2192} cmd + 2 keys");
    }

    #[test]
    fn a_directional_chip_row_renders_the_same_chips_item_chips_produces() {
        let state = ObjectArrayState::from_entries(
            None,
            vec![
                (
                    "from_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "to_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "keys".to_string(),
                    qol_config::object_array::ItemFieldKind::StringArray,
                ),
                (
                    "global".to_string(),
                    qol_config::object_array::ItemFieldKind::Boolean,
                ),
            ],
            vec![Entry {
                key: None,
                fields: Item::from_iter([
                    (
                        "from_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["ctrl".into()]),
                    ),
                    (
                        "to_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["cmd".into()]),
                    ),
                    (
                        "keys".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec![
                            "c".into(),
                            "v".into(),
                        ]),
                    ),
                    (
                        "global".to_string(),
                        qol_config::contract::FieldDefault::Boolean(true),
                    ),
                ]),
            }],
        );
        let chips = state.chips(0);
        assert!(chips.is_directional());
        assert_eq!(
            super::chip_row_parts(&chips),
            vec![
                ChipRowPart::Chip(Chip {
                    label: "ctrl".into(),
                    tone: ChipTone::Modifier
                }),
                ChipRowPart::Chip(Chip {
                    label: "2 keys".into(),
                    tone: ChipTone::Key
                }),
                ChipRowPart::Arrow,
                ChipRowPart::Chip(Chip {
                    label: "cmd".into(),
                    tone: ChipTone::Modifier
                }),
                ChipRowPart::Chip(Chip {
                    label: "global".into(),
                    tone: ChipTone::Plain
                }),
            ]
        );
    }

    #[test]
    fn a_non_directional_chip_row_renders_in_order_without_an_arrow() {
        let chips = ItemChips {
            from: vec![Chip {
                label: "idea".into(),
                tone: ChipTone::Key,
            }],
            rest: Vec::new(),
            to: Vec::new(),
            flags: vec!["wired".into()],
        };
        assert_eq!(
            super::chip_row_parts(&chips),
            vec![
                ChipRowPart::Chip(Chip {
                    label: "idea".into(),
                    tone: ChipTone::Key
                }),
                ChipRowPart::Chip(Chip {
                    label: "wired".into(),
                    tone: ChipTone::Plain
                }),
            ]
        );
    }

    fn listed_row(items: Vec<super::super::rows::ListItem>) -> Row {
        let mut row = list_row();
        let RowControl::List { items: stored, .. } = &mut row.control else {
            unreachable!();
        };
        *stored = items;
        row
    }

    fn list_item(id: &str, name: &str) -> super::super::rows::ListItem {
        super::super::rows::ListItem {
            id: id.into(),
            label: name.into(),
            subtitle: None,
            accent: None,
            badge: None,
            badge_tone: None,
            data: serde_json::Value::Null,
            pending: false,
            error: None,
        }
    }

    fn list_card_fixture() -> (Vec<Row>, Level) {
        let mut root = level(0);
        root.rows = vec![listed_row(vec![
            list_item("a", "Alpha"),
            list_item("b", "Beta"),
            list_item("c", "Gamma"),
        ])];
        let RowControl::List {
            actions,
            items,
            filter,
            list,
            ..
        } = &root.rows[0].control
        else {
            unreachable!();
        };
        let child = super::list_card_level(
            super::ListCardOrigin {
                label: "Devices",
                config_key: "devices",
                source: 0,
                row: 0,
            },
            actions,
            items,
            filter,
            list.selected,
        );
        (root.rows, child)
    }

    #[test]
    fn list_card_child_rows_builds_one_row_per_item() {
        let items = vec![
            super::super::rows::ListItem {
                id: "aa:bb".into(),
                label: "Headphones".into(),
                subtitle: Some("Sony WH-1000XM4".into()),
                accent: None,
                badge: Some("Connected".into()),
                badge_tone: None,
                data: serde_json::Value::Null,
                pending: false,
                error: None,
            },
            list_item("cc:dd", "Keyboard"),
        ];
        let rows = super::list_card_child_rows("Devices", "devices", 2, &items, &[0, 1]);
        assert_eq!(rows.len(), 2);
        for (slot, row) in rows.iter().enumerate() {
            assert_eq!(row.label, items[slot].label);
            assert_eq!(row.id, items[slot].id);
            assert_eq!(row.config_key, "devices");
            assert_eq!(row.source, 2);
        }
        assert!(matches!(
            &rows[0].control,
            RowControl::Text(value) if value == "Connected"
        ));
        assert!(matches!(
            &rows[1].control,
            RowControl::Text(value) if value.is_empty()
        ));
    }

    #[test]
    fn a_list_card_rebuild_keeps_the_selection_on_the_same_item() {
        let (mut rows, mut child) = list_card_fixture();
        child.selected = 1;
        super::super::rows::apply_runtime_query(
            &mut rows,
            "items",
            Ok(serde_json::json!({"items": [
                {"id": "c", "name": "Gamma"},
                {"id": "a", "name": "Alpha"},
                {"id": "b", "name": "Beta"},
            ]})),
            &|_, _| false,
        );
        super::list_card_sync(&mut rows, &mut child);
        assert_eq!(child.rows.len(), 3);
        assert_eq!(child.selected, 2);
        assert_eq!(child.rows[child.selected].id, "b");
        let RowControl::List {
            list: root_list, ..
        } = &rows[0].control
        else {
            unreachable!();
        };
        assert_eq!(root_list.selected, 2);
    }

    #[test]
    fn a_list_card_rebuild_clamps_when_the_selected_item_disappears() {
        let (mut rows, mut child) = list_card_fixture();
        child.selected = 1;
        super::super::rows::apply_runtime_query(
            &mut rows,
            "items",
            Ok(serde_json::json!({"items": [
                {"id": "a", "name": "Alpha"},
                {"id": "c", "name": "Gamma"},
            ]})),
            &|_, _| false,
        );
        super::list_card_sync(&mut rows, &mut child);
        assert_eq!(child.rows.len(), 2);
        assert_eq!(child.selected, 1);
        assert_eq!(child.rows[child.selected].id, "c");
        super::super::rows::apply_runtime_query(
            &mut rows,
            "items",
            Ok(serde_json::json!({"items": []})),
            &|_, _| false,
        );
        super::list_card_sync(&mut rows, &mut child);
        assert_eq!(child.rows.len(), 0);
        assert_eq!(child.selected, 0);
        let RowControl::List {
            list: root_list, ..
        } = &rows[0].control
        else {
            unreachable!();
        };
        assert_eq!(root_list.selected, 0);
    }

    fn list_item_with_value(id: &str, name: &str, level: f64) -> super::super::rows::ListItem {
        let mut item = list_item(id, name);
        item.data = serde_json::json!({ "level": level });
        item
    }

    fn slider_list_row() -> Row {
        let mut row = listed_row(vec![
            list_item_with_value("a", "Alpha", 0.3),
            list_item_with_value("b", "Beta", 0.8),
        ]);
        let RowControl::List { slider, .. } = &mut row.control else {
            unreachable!();
        };
        *slider = Some(Box::new(super::super::rows::ListSlider {
            spec: qol_config::contract::RowSliderSpec {
                value_from: "level".into(),
                min: 0.0,
                max: 1.0,
                step: 0.1,
                action: "set_level".into(),
                input: None,
            },
            values: std::collections::HashMap::new(),
        }));
        row
    }

    fn slider_card_fixture() -> (Vec<Row>, Level) {
        let mut root = level(0);
        root.rows = vec![slider_list_row()];
        let RowControl::List {
            actions,
            items,
            filter,
            list,
            ..
        } = &root.rows[0].control
        else {
            unreachable!();
        };
        let child = super::list_card_level(
            super::ListCardOrigin {
                label: "Devices",
                config_key: "devices",
                source: 0,
                row: 0,
            },
            actions,
            items,
            filter,
            list.selected,
        );
        (root.rows, child)
    }

    #[test]
    fn a_card_slider_row_exposes_the_same_value_list_slider_value_returns() {
        let (mut rows, mut child) = slider_card_fixture();
        child.selected = 1;
        let RowControl::List {
            actions,
            slider,
            items,
            filter,
            ..
        } = &mut rows[0].control
        else {
            unreachable!();
        };
        slider.as_mut().unwrap().values.insert(
            "b".into(),
            SliderHold {
                value: 0.55,
                dispatched: Some(0.55),
                until: std::time::Instant::now() + super::SLIDER_HOLD_DURATION,
            },
        );
        let slider = slider.as_deref().unwrap();
        let held = list_slider_value(&slider.spec, &slider.values, &items[1]);
        assert_eq!(
            held, 0.55,
            "the hold wins over the item data while it lasts"
        );
        assert_eq!(
            list_card_slider_value(slider, actions, items, filter, child.selected),
            Some(held),
            "the card row for the selected slot shows the root list value for its item"
        );
        assert_eq!(
            list_card_slider_value(slider, actions, items, filter, 0),
            Some(list_slider_value(&slider.spec, &slider.values, &items[0])),
        );
    }

    #[test]
    fn card_slider_stepping_lands_on_the_item_the_well_would_step() {
        let (mut rows, mut child) = slider_card_fixture();
        let RowControl::List {
            actions,
            slider,
            items,
            filter,
            list,
            ..
        } = &mut rows[0].control
        else {
            unreachable!();
        };
        child.selected = 1;
        list.selected = child.selected;
        let card_item = selected_list_item(actions, items, filter, child.selected).unwrap();
        let well_item = selected_list_item(actions, items, filter, list.selected).unwrap();
        assert_eq!(card_item.id, "b");
        assert_eq!(well_item.id, card_item.id);
        let slider = slider.as_mut().unwrap();
        step_list_slider(slider, card_item, 1.0);
        let hold = slider
            .values
            .get(&well_item.id)
            .expect("the step records the hold under the id the well would step");
        assert_eq!(hold.value, 0.9);
    }

    #[test]
    fn pushing_a_text_list_child_popping_leaves_the_root_untouched() {
        let mut root = level(2);
        root.rows = rows(&[false, false, false]);
        root.sections = vec![source_section("Devices", 0, vec![0, 1, 2])];
        let root_sections = root.sections.clone();
        let values = vec![
            "com.kitty.app".to_string(),
            "com.apple.Terminal".to_string(),
            "com.googlecode.iterm2".to_string(),
        ];
        let child = Level {
            rows: super::text_list_child_rows("Excluded apps", "excluded_apps", 0, &values),
            sections: vec![source_section("Excluded apps", 0, (0..4).collect())],
            selected: 0,
            active_section: None,
            selected_section: 0,
            body_scroll: crate::scroll_list::SelectionScroll::new(),
            active_control: None,
            row_bounds: Vec::new(),
            title: Some("Excluded apps".into()),
            origin_row: Some(1),
            object_array: None,
            list_card: false,
            live_card: false,
        };
        let mut stack = vec![root];
        push_level(&mut stack, child);
        assert_eq!(stack.len(), 2);
        let popped = pop_level(&mut stack).unwrap();
        assert_eq!(popped.title.as_deref(), Some("Excluded apps"));
        assert_eq!(popped.origin_row, Some(1));
        assert_eq!(popped.rows.len(), 4);
        assert_eq!(stack.len(), 1);
        let root = stack.last().unwrap();
        assert_eq!(root.rows.len(), 3);
        assert_eq!(root.selected, 2);
        assert_eq!(root.sections, root_sections);
    }

    #[test]
    fn escape_prefers_the_filter_over_everything() {
        assert_eq!(escape_step(2, true, true), EscapeStep::CloseFilter);
    }

    #[test]
    fn escape_pops_one_card_before_touching_the_rail() {
        assert_eq!(escape_step(1, false, true), EscapeStep::PopCard);
        assert_eq!(escape_step(2, false, false), EscapeStep::PopCard);
    }

    #[test]
    fn escape_at_the_root_ascends_then_dismisses() {
        assert_eq!(escape_step(0, false, true), EscapeStep::AscendRail);
        assert_eq!(escape_step(0, false, false), EscapeStep::Dismiss);
    }

    #[test]
    fn popping_reclamps_the_parent_cursor() {
        let mut parent = level(9);
        parent.rows = rows(&[false, false]);
        parent.sections = vec![source_section("Devices", 0, vec![0, 1])];
        let mut stack = vec![parent, level(7)];
        pop_level(&mut stack).unwrap();
        let root = stack.last().unwrap();
        let visible = super::filtered_visible_rows(&root.rows, &root.sections, 1, None, 0);
        let selected = clamp_selected(&visible, root.selected);
        assert_eq!(selected, 0);
        assert!(visible.contains(&selected));
    }

    #[test]
    fn a_dropped_group_header_does_not_shift_the_scroll_target() {
        let sections = vec![vec![0usize, 1], vec![2, 3]];
        assert_eq!(
            super::body_child_offset(&sections, &[false, true], 3),
            Some(4)
        );
        assert_eq!(
            super::body_child_offset(&sections, &[true, true], 3),
            Some(5)
        );
    }

    #[test]
    fn the_trail_follows_from_the_window_down_to_the_open_card() {
        assert_eq!(
            crumb_labels(
                "qol settings",
                Some("Key remap".to_string()),
                vec!["Excluded apps".to_string()]
            ),
            vec!["qol settings", "Key remap", "Excluded apps"]
        );
    }

    #[test]
    fn a_single_plugin_panel_does_not_repeat_its_own_name_in_the_trail() {
        assert_eq!(
            crumb_labels("Key remap", Some("Key remap".to_string()), Vec::new()),
            vec!["Key remap"]
        );
    }

    #[test]
    fn transition_policy_decides_animate_snap_or_nothing() {
        let cases = [
            (false, false, None),
            (true, false, None),
            (false, true, Some(TransitionAction::Animate)),
            (true, true, Some(TransitionAction::Snap)),
        ];
        for (in_flight, state_changed, expected) in cases {
            assert_eq!(
                transition_policy(in_flight, state_changed),
                expected,
                "in_flight: {in_flight} state_changed: {state_changed}"
            );
        }
    }

    #[test]
    fn frame_paced_100ms_query_with_16ms_requests_keeps_its_interval() {
        let epoch = std::time::Instant::now();
        let at = |ms: u64| epoch + std::time::Duration::from_millis(ms);
        let interval = std::time::Duration::from_millis(100);
        let mut due = vec![None];
        let mut runs = 0;
        let mut tick = 0u64;
        while tick <= 1000 {
            if !due_query_indices(&due, at(tick)).is_empty() {
                runs += 1;
                due[0] = Some(at(tick) + interval);
            }
            tick += 16;
        }
        assert!(
            (9..=11).contains(&runs),
            "a 100ms query asked every 16ms must run ~10x/sec, ran {runs}"
        );
    }

    #[test]
    fn frame_paced_queries_never_bypass_the_due_gate() {
        let epoch = std::time::Instant::now();
        let at = |ms: u64| epoch + std::time::Duration::from_millis(ms);
        let intervals = [
            std::time::Duration::from_millis(8),
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis(1000),
        ];
        let mut due = vec![None; 3];
        let mut runs = [0usize; 3];
        let mut tick = 0u64;
        while tick <= 1000 {
            for index in due_query_indices(&due, at(tick)) {
                runs[index] += 1;
                due[index] = Some(at(tick) + intervals[index]);
            }
            tick += 16;
        }
        assert!((57..=66).contains(&runs[0]), "8ms query ran {}x", runs[0]);
        assert!((9..=11).contains(&runs[1]), "100ms query ran {}x", runs[1]);
        assert_eq!(runs[2], 1, "1000ms query ran {}x", runs[2]);
    }

    #[test]
    fn first_request_with_no_due_time_always_runs() {
        let now = std::time::Instant::now();
        assert!(query_is_due(None, now));
        assert_eq!(due_query_indices(&[None], now), vec![0]);
    }

    #[test]
    fn exactly_due_runs_and_one_millisecond_early_does_not() {
        let epoch = std::time::Instant::now();
        let at = |ms: u64| epoch + std::time::Duration::from_millis(ms);
        assert!(query_is_due(Some(at(100)), at(100)));
        assert!(query_is_due(Some(at(100)), at(101)));
        assert!(!query_is_due(Some(at(100)), at(99)));
        assert_eq!(
            due_query_indices(&[Some(at(100))], at(99)),
            Vec::<usize>::new()
        );
        assert_eq!(due_query_indices(&[Some(at(100))], at(100)), vec![0]);
    }

    #[test]
    fn transition_in_flight_window_uses_the_rail_duration() {
        let epoch = std::time::Instant::now();
        let at = |ms: u64| epoch + std::time::Duration::from_millis(ms);
        assert!(!transition_in_flight(None, at(0)));
        assert!(transition_in_flight(Some(epoch), at(0)));
        assert!(transition_in_flight(Some(epoch), at(179)));
        assert!(!transition_in_flight(Some(epoch), at(180)));
        assert!(!transition_in_flight(Some(epoch), at(181)));
    }

    #[test]
    fn transition_tracker_bumps_once_per_burst_and_snaps_while_in_flight() {
        let epoch = std::time::Instant::now();
        let at = |ms: u64| epoch + std::time::Duration::from_millis(ms);
        let mut tracker = TransitionTracker::default();

        tracker.state_changed(true, at(0));
        assert_eq!(tracker.step, 1, "first state change animates");
        assert!(!tracker.snapped);
        assert_eq!(tracker.started, Some(at(0)));

        tracker.state_changed(true, at(60));
        assert_eq!(
            tracker.step, 1,
            "in-flight change must not bump the counter"
        );
        assert!(tracker.snapped);
        assert_eq!(tracker.started, Some(at(0)));

        tracker.state_changed(true, at(120));
        assert_eq!(tracker.step, 1, "in-flight change must not restart");
        assert!(tracker.snapped);

        tracker.state_changed(true, at(300));
        assert_eq!(tracker.step, 2, "change after the window animates again");
        assert!(!tracker.snapped);
        assert_eq!(tracker.started, Some(at(300)));
    }

    #[test]
    fn transition_tracker_ignores_unchanged_state() {
        let mut tracker = TransitionTracker::default();
        tracker.state_changed(false, std::time::Instant::now());
        assert_eq!(tracker.step, 0);
        assert!(!tracker.snapped);
        assert_eq!(tracker.started, None);
    }

    #[test]
    fn height_cache_recomputes_only_when_the_revision_changes() {
        let mut cache = HeightCache::default();
        let computes = std::cell::Cell::new(0);
        let compute = || {
            computes.set(computes.get() + 1);
            128.0
        };
        assert_eq!(cache.value(0, compute), 128.0);
        assert_eq!(cache.value(0, compute), 128.0);
        assert_eq!(computes.get(), 1, "same revision serves the cached height");
        assert_eq!(cache.value(1, compute), 128.0);
        assert_eq!(computes.get(), 2, "new revision recomputes");
        assert_eq!(cache.value(1, compute), 128.0);
        assert_eq!(
            computes.get(),
            2,
            "same revision stays cached after recompute"
        );
    }

    #[test]
    fn list_fit_updates_report_only_unfitted_lists() {
        let mut rows = vec![list_row()];
        let sections = vec![RowSection {
            label: "Section".into(),
            description: None,
            rows: vec![0],
            source: 0,
        }];
        assert_eq!(
            list_fit_updates(&rows, &sections, 0, 720.0),
            Vec::new(),
            "a list already at the fit target is left untouched"
        );
        let RowControl::List { list, .. } = &mut rows[0].control else {
            unreachable!();
        };
        list.max_visible = 3;
        let updates = list_fit_updates(&rows, &sections, 0, 720.0);
        assert_eq!(updates, vec![(0, super::super::rows::LIST_MAX_VISIBLE)]);
        for (index, visible_items) in updates {
            if let RowControl::List { list, .. } = &mut rows[index].control {
                list.max_visible = visible_items;
            }
        }
        assert_eq!(
            list_fit_updates(&rows, &sections, 0, 720.0),
            Vec::new(),
            "applying the updates settles the list"
        );
    }

    #[test]
    fn source_window_height_for_clamps_to_the_height_cap() {
        let rows = vec![list_row(), list_row()];
        let sections = vec![RowSection {
            label: "Section".into(),
            description: None,
            rows: vec![0, 1],
            source: 0,
        }];
        let uncapped = source_window_height_for(&rows, &sections, 0, 720.0);
        assert_eq!(
            source_window_height_for(&rows, &sections, 0, uncapped - 40.0),
            uncapped - 40.0,
            "a cap below the natural height is applied"
        );
        assert!(uncapped <= 720.0);
        let without_sections = source_window_height_for(&rows, &sections, 1, 720.0);
        assert!(
            without_sections > 0.0 && without_sections < uncapped,
            "a source without sections still contributes its floor height"
        );
    }
}

#[cfg(test)]
mod query_state_tests {
    use super::{rollup_query_state, RowQueryState};
    use std::time::{Duration, Instant};

    const GRACE: Duration = Duration::from_millis(300);

    fn rollup(states: &[Option<RowQueryState>], now: Instant) -> RowQueryState {
        rollup_query_state(states.iter().map(Option::as_ref), GRACE, now)
    }

    /// A healthy plugin answers well inside the grace period, so its rows must
    /// never flash a spinner on the way to their value.
    #[test]
    fn a_query_inside_its_grace_period_shows_no_indicator() {
        let now = Instant::now();
        let fresh = RowQueryState::Loading { since: now };
        assert_eq!(rollup(&[Some(fresh)], now), RowQueryState::Idle);
    }

    /// Past the grace period the row has to admit it is waiting, otherwise a
    /// wedged daemon is indistinguishable from a working one.
    #[test]
    fn a_query_past_its_grace_period_reports_loading() {
        let now = Instant::now();
        let stale = RowQueryState::Loading {
            since: now - GRACE - Duration::from_millis(1),
        };
        assert!(matches!(
            rollup(&[Some(stale)], now),
            RowQueryState::Loading { .. }
        ));
    }

    /// A row backed by several queries is only as good as its worst one.
    #[test]
    fn the_worst_query_decides_what_the_row_shows() {
        let now = Instant::now();
        let waiting = RowQueryState::Loading {
            since: now - GRACE * 2,
        };
        let cases = [
            (
                vec![Some(RowQueryState::Ready), Some(waiting.clone())],
                "loading",
            ),
            (
                vec![
                    Some(RowQueryState::Ready),
                    Some(waiting.clone()),
                    Some(RowQueryState::Unavailable("dead".into())),
                ],
                "unavailable",
            ),
            (vec![Some(RowQueryState::Ready), None], "ready"),
            (vec![None, None], "idle"),
        ];
        for (states, expected) in cases {
            let actual = match rollup(&states, now) {
                RowQueryState::Idle => "idle",
                RowQueryState::Loading { .. } => "loading",
                RowQueryState::Ready => "ready",
                RowQueryState::Unavailable(_) => "unavailable",
            };
            assert_eq!(actual, expected, "states: {states:?}");
        }
    }
}
