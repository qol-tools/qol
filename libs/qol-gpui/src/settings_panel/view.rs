use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_config::contract::ResolvedRowAction;
use qol_config::object_array::pretty_label;

use super::object_array_row::{
    ChipTone, DraftField, DraftValue, ItemChips, ObjectArrayOutcome, ObjectArrayState,
};
use super::persistence::save_values;
use super::rows::{
    apply_runtime_query, begin_list_item_action, list_item_actions, merged_config,
    primary_list_item_action, query_flag_value, runtime_query_names, section_label_for, Row,
    RowControl, RowSection, SelectOption,
};
use super::{SettingsPanel, SettingsRuntime};
use crate::color_wheel::{ColorWheel, ColorWheelPopup, WheelCallbacks, WheelStyle};
use crate::dropdown::{Dropdown, DropdownEvent, DropdownItem, DropdownStyle};
use crate::gamepad::{gamepad_panel, GamepadPalette};
use crate::spinner::Spinner;
use crate::status_indicator::{StatusIndicator, StatusTone};
use crate::surface::{PanelDragArea, SurfaceDismisser};
use crate::theme::{settings_panel_runtime, SettingsPanelPalette};

type SampledQueryResults =
    std::sync::Arc<std::sync::Mutex<Vec<(String, Result<serde_json::Value, String>)>>>;

const FRAME_PACED_QUERY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

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
    runtime: SettingsRuntime,
    runtime_queries: Vec<String>,
    rows: Vec<Row>,
    sections: Vec<RowSection>,
    active_section: Option<usize>,
    selected_section: usize,
    values: serde_json::Value,
    path: PathBuf,
    selected: usize,
    scroll_offset: usize,
    body_max: f32,
    height_cap: f32,
    active_control: Option<ActiveControl>,
    row_bounds: Vec<Rc<Cell<Option<Bounds<Pixels>>>>>,
    wheel_generation: u64,
    runtime_poll_generation: u64,
    frame_paced_samples: Option<SampledQueryResults>,
    applied_query_payloads: std::collections::HashMap<String, Result<serde_json::Value, String>>,
    frame_pump_armed: bool,
    motion_tick: Option<std::time::Instant>,
    sample_signal: Option<std::sync::Arc<SampleSignal>>,
    sampler_stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    dismisser: SurfaceDismisser,
    palette: SettingsPanelPalette,
    focus_handle: FocusHandle,
}

pub(super) struct SettingsPanelState {
    pub(super) rows: Vec<Row>,
    pub(super) sections: Vec<RowSection>,
    pub(super) values: serde_json::Value,
    pub(super) path: PathBuf,
    pub(super) body_max: f32,
    pub(super) height_cap: f32,
    pub(super) runtime: SettingsRuntime,
}

enum ActiveControl {
    Edit(String),
    Dropdown(Dropdown),
    List,
    ListActions(ListActionMenu),
    ObjectArray,
    Wheel(WheelControl),
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
        cx: &mut Context<Self>,
    ) -> Self {
        let row_bounds = (0..state.rows.len())
            .map(|_| Rc::new(Cell::new(None)))
            .collect();
        let runtime_queries = runtime_query_names(&state.rows);
        cx.on_release(|view, cx| view.close_wheel_popup(cx))
            .detach();
        let mut view = Self {
            panel,
            runtime: state.runtime,
            runtime_queries,
            rows: state.rows,
            active_section: initial_active_section(state.sections.len()),
            sections: state.sections,
            selected_section: 0,
            values: state.values,
            path: state.path,
            selected: 0,
            scroll_offset: 0,
            body_max: state.body_max,
            height_cap: state.height_cap,
            active_control: None,
            row_bounds,
            wheel_generation: 0,
            runtime_poll_generation: 0,
            frame_paced_samples: None,
            applied_query_payloads: std::collections::HashMap::new(),
            frame_pump_armed: false,
            motion_tick: None,
            sample_signal: None,
            sampler_stop: None,
            dismisser,
            palette: settings_panel_runtime(),
            focus_handle: cx.focus_handle(),
        };
        view.selected = view.current_visible_rows().into_iter().next().unwrap_or(0);
        view.resume_runtime_poll(cx);
        view
    }

    fn current_visible_rows(&self) -> Vec<usize> {
        let Some(section) = self
            .active_section
            .and_then(|index| self.sections.get(index))
        else {
            return Vec::new();
        };
        section
            .rows
            .iter()
            .copied()
            .filter(|index| super::rows::row_is_visible(&self.rows, *index))
            .collect()
    }

    fn section_menu_is_open(&self) -> bool {
        self.sections.len() > 1 && self.active_section.is_none()
    }

    fn open_selected_section(&mut self) {
        if self.sections.is_empty() {
            return;
        }
        self.active_section = Some(self.selected_section.min(self.sections.len() - 1));
        self.selected = self.current_visible_rows().into_iter().next().unwrap_or(0);
        self.scroll_offset = self.selected;
        self.active_control = None;
    }

    fn open_section_menu(&mut self) {
        let Some(active) = self.active_section else {
            return;
        };
        self.selected_section = active;
        self.active_section = None;
        self.active_control = None;
        self.scroll_offset = 0;
    }

    fn on_section_menu_key(&mut self, key: &str, cx: &mut Context<Self>) {
        match key {
            "up" => self.selected_section = self.selected_section.saturating_sub(1),
            "down" => {
                self.selected_section =
                    (self.selected_section + 1).min(self.sections.len().saturating_sub(1));
            }
            "enter" | "return" | "space" | "right" => self.open_selected_section(),
            "escape" | "left" => {
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
                                    apply_runtime_query(&mut this.rows, &query, result);
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
        for row in &mut self.rows {
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
            apply_runtime_query(&mut self.rows, &query, result);
            changed = true;
        }
        changed
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
            for (index, query) in queries.iter().enumerate() {
                let frame_paced = intervals[index] <= FRAME_PACED_QUERY_INTERVAL;
                if !frame_paced && due[index].is_some_and(|due| due > started) {
                    continue;
                }
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
        latest: SampledQueryResults,
        executor: BackgroundExecutor,
    ) {
        let intervals = queries
            .iter()
            .map(|query| runtime.query_interval(query))
            .collect::<Vec<_>>();
        let mut due = vec![std::time::Instant::now(); queries.len()];
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
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
        if self.section_menu_is_open() {
            self.on_section_menu_key(key, cx);
            return;
        }
        if let Some(ActiveControl::Wheel(wheel)) = &self.active_control {
            let popup = wheel.popup;
            let _ = popup.update(cx, |popup, popup_window, popup_cx| {
                popup.handle_key(key, event.keystroke.modifiers.shift, popup_window, popup_cx);
            });
            return;
        }
        if matches!(self.active_control, Some(ActiveControl::ListActions(_))) {
            self.on_list_actions_key(key, cx);
            return;
        }
        if matches!(self.active_control, Some(ActiveControl::Dropdown(_))) {
            self.on_dropdown_key(key, cx);
            return;
        }
        if matches!(self.active_control, Some(ActiveControl::List)) {
            self.on_list_key(key, cx);
            return;
        }
        if matches!(self.active_control, Some(ActiveControl::ObjectArray)) {
            self.on_object_array_key(key, key_char, cx);
            return;
        }
        let editing = matches!(self.active_control, Some(ActiveControl::Edit(_)));
        if editing {
            if horizontal_step_direction(key)
                .is_some_and(|direction| self.step_number_edit(direction))
            {
                cx.notify();
                return;
            }
            if matches!(key, "up" | "down") {
                self.commit_edit();
            }
        }
        let Some(intent) = intent(key, key_char, editing) else {
            return;
        };
        match intent {
            Intent::Up => {
                self.selected =
                    adjacent_visible_row(&self.current_visible_rows(), self.selected, -1);
                self.sync_scroll();
            }
            Intent::Down => {
                self.selected =
                    adjacent_visible_row(&self.current_visible_rows(), self.selected, 1);
                self.sync_scroll();
            }
            Intent::Left => {
                if self.sections.len() > 1 {
                    self.open_section_menu();
                    cx.notify();
                }
            }
            Intent::Activate => self.activate(window, cx),
            Intent::CommitEdit => self.commit_edit(),
            Intent::Backspace => {
                if let Some(ActiveControl::Edit(edit)) = self.active_control.as_mut() {
                    edit.pop();
                }
            }
            Intent::Insert(ch) => {
                if let Some(ActiveControl::Edit(edit)) = self.active_control.as_mut() {
                    edit.push_str(&ch);
                }
            }
            Intent::CancelEdit => self.active_control = None,
            Intent::Close => {
                if self.sections.len() > 1 {
                    self.open_section_menu();
                    cx.notify();
                    return;
                }
                self.pause_runtime_poll();
                self.dismisser.dismiss(cx);
                return;
            }
        }
        cx.notify();
    }

    fn open_color_wheel(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let RowControl::Color(value) = &row.control else {
            return;
        };
        let Some(anchor) = self.row_bounds.get(index).and_then(|bounds| bounds.get()) else {
            return;
        };
        let value = value.clone();
        self.close_wheel_popup(cx);
        self.selected = index;
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
        self.active_control = Some(ActiveControl::Wheel(WheelControl {
            generation,
            row: index,
            value: preview,
            popup,
        }));
        cx.notify();
    }

    fn preview_wheel(&mut self, generation: u64, value: String, cx: &mut Context<Self>) {
        let Some(ActiveControl::Wheel(wheel)) = self.active_control.as_mut() else {
            return;
        };
        if wheel.generation != generation {
            return;
        }
        wheel.value = value;
        cx.notify();
    }

    fn commit_wheel(&mut self, generation: u64, value: String, cx: &mut Context<Self>) {
        let Some(ActiveControl::Wheel(wheel)) = self.active_control.as_ref() else {
            return;
        };
        if wheel.generation != generation {
            return;
        }
        let row_index = wheel.row;
        self.active_control = None;
        let Some(row) = self.rows.get_mut(row_index) else {
            return;
        };
        let RowControl::Color(row_value) = &mut row.control else {
            return;
        };
        *row_value = value;
        self.persist();
        cx.notify();
    }

    fn close_wheel_popup(&mut self, cx: &mut App) {
        if !matches!(self.active_control, Some(ActiveControl::Wheel(_))) {
            return;
        }
        let Some(ActiveControl::Wheel(wheel)) = self.active_control.take() else {
            return;
        };
        let _ = wheel
            .popup
            .update(cx, |_, window, _| window.remove_window());
    }

    fn on_dropdown_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(ActiveControl::Dropdown(dropdown)) = self.active_control.as_mut() else {
            return;
        };
        let Some(event) = dropdown.handle_key(key) else {
            return;
        };
        match event {
            DropdownEvent::Moved => {}
            DropdownEvent::Pick(_) => self.pick_dropdown(),
            DropdownEvent::Close => self.active_control = None,
        }
        cx.notify();
    }

    fn on_list_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(intent) = list_intent(key) else {
            return;
        };
        if intent == ListIntent::Activate {
            self.dispatch_list_action(cx);
            return;
        }
        if intent == ListIntent::Actions {
            self.open_list_actions();
            cx.notify();
            return;
        }
        let Some(row) = self.rows.get_mut(self.selected) else {
            self.active_control = None;
            return;
        };
        let RowControl::List { items, list, .. } = &mut row.control else {
            self.active_control = None;
            return;
        };
        match intent {
            ListIntent::Up => list.move_up(),
            ListIntent::Down => list.move_down(items.len()),
            ListIntent::Close => self.active_control = None,
            ListIntent::Activate | ListIntent::Actions => return,
        }
        list.sync(items.len());
        cx.notify();
    }

    fn on_object_array_key(&mut self, key: &str, key_char: Option<&str>, cx: &mut Context<Self>) {
        let outcome = match self.rows.get_mut(self.selected).map(|row| &mut row.control) {
            Some(RowControl::ObjectArray(state)) => state.handle_key(key, key_char),
            _ => ObjectArrayOutcome::Close,
        };
        match outcome {
            ObjectArrayOutcome::Ignored => return,
            ObjectArrayOutcome::Handled => {}
            ObjectArrayOutcome::Persist => self.persist(),
            ObjectArrayOutcome::Close => self.active_control = None,
        }
        self.sync_scroll();
        cx.notify();
    }

    fn on_list_actions_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(ActiveControl::ListActions(menu)) = self.active_control.as_mut() else {
            return;
        };
        let Some(event) = menu.dropdown.handle_key(key) else {
            return;
        };
        match event {
            DropdownEvent::Moved => {}
            DropdownEvent::Pick(selected) => self.dispatch_list_menu_action(selected, cx),
            DropdownEvent::Close => self.active_control = Some(ActiveControl::List),
        }
        cx.notify();
    }

    fn pick_dropdown(&mut self) {
        let Some(ActiveControl::Dropdown(dropdown)) = self.active_control.as_ref() else {
            return;
        };
        self.pick_dropdown_option(dropdown.selected());
    }

    fn pick_dropdown_option(&mut self, pick: usize) {
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Select { options, index, .. } => {
                if pick < options.len() {
                    *index = pick;
                    self.persist();
                }
                self.active_control = None;
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
            | RowControl::Unsupported { .. } => self.active_control = None,
        }
    }

    fn toggle(&mut self) {
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        if let RowControl::Toggle(value) = &mut row.control {
            *value = !*value;
            self.persist();
        }
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        match &row.control {
            RowControl::Toggle(_) => self.toggle(),
            RowControl::Select { options, index, .. } => {
                self.active_control = Some(ActiveControl::Dropdown(Dropdown::open(
                    options.len(),
                    *index,
                )));
            }
            RowControl::MultiSelect { options, .. } => {
                self.active_control =
                    Some(ActiveControl::Dropdown(Dropdown::open(options.len(), 0)));
            }
            RowControl::Color(_) => self.open_color_wheel(self.selected, window, cx),
            RowControl::Action { .. } => self.dispatch_action(cx),
            RowControl::Status { .. } => {}
            RowControl::List { items, .. } if !items.is_empty() => {
                self.active_control = Some(ActiveControl::List)
            }
            RowControl::List { .. } => {}
            RowControl::ObjectArray(_) => self.active_control = Some(ActiveControl::ObjectArray),
            RowControl::Gamepad { .. } => self.select_next_gamepad(),
            RowControl::QrCode { .. } => {}
            RowControl::Unsupported { .. } => {}
            RowControl::Number { .. } | RowControl::Text(_) | RowControl::TextList(_) => {
                self.begin_edit()
            }
        }
    }

    fn select_next_gamepad(&mut self) {
        let Some(RowControl::Gamepad { monitor, .. }) =
            self.rows.get_mut(self.selected).map(|row| &mut row.control)
        else {
            return;
        };
        monitor.select_next();
    }

    fn dispatch_action(&mut self, cx: &mut Context<Self>) {
        let Some(RowControl::Action {
            action,
            active_action,
            active_query,
            active_value_from,
            active,
            pending,
            error,
            ..
        }) = self.rows.get_mut(self.selected).map(|row| &mut row.control)
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
        let row_index = self.selected;
        let refresh_query = active_query.clone();
        let refresh_value_from = active_value_from.clone();
        #[cfg(debug_assertions)]
        let plugin_id = self.panel.plugin_id.clone();
        #[cfg(debug_assertions)]
        let dispatched_action = action.clone();
        let rearm_poll_generation = refresh_query.is_some().then(|| {
            self.pause_runtime_poll();
            self.runtime_poll_generation
        });
        let runtime = self.runtime.clone();
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
                    if let Some(RowControl::Action { pending, error, .. }) =
                        this.rows.get_mut(row_index).map(|row| &mut row.control)
                    {
                        *pending = false;
                        *error = result.err();
                    }
                    if let Some((query, result)) = refreshed {
                        apply_runtime_query(&mut this.rows, &query, result);
                    }
                    #[cfg(debug_assertions)]
                    if let Some(RowControl::Action { active, error, .. }) =
                        this.rows.get(row_index).map(|row| &row.control)
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

    fn dispatch_list_action(&mut self, cx: &mut Context<Self>) {
        let row_index = self.selected;
        let Some(RowControl::List {
            actions,
            items,
            list,
            ..
        }) = self.rows.get(row_index).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) = items.get(list.selected) else {
            return;
        };
        let Some(action) = primary_list_item_action(actions, item) else {
            return;
        };
        let item_id = item.id.clone();
        self.dispatch_resolved_list_action(row_index, &item_id, action, cx);
    }

    fn select_list_item(&mut self, row_index: usize, item_index: usize) {
        let Some(RowControl::List { items, list, .. }) =
            self.rows.get_mut(row_index).map(|row| &mut row.control)
        else {
            return;
        };
        if item_index >= items.len() {
            return;
        }
        self.selected = row_index;
        list.selected = item_index;
        list.sync(items.len());
        self.active_control = Some(ActiveControl::List);
        self.sync_scroll();
    }

    fn open_list_actions(&mut self) {
        let row_index = self.selected;
        let Some(RowControl::List {
            actions,
            items,
            list,
            ..
        }) = self.rows.get(row_index).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) = items.get(list.selected).filter(|item| !item.pending) else {
            return;
        };
        let resolved = list_item_actions(actions, item);
        if resolved.len() < 2 {
            return;
        }
        self.active_control = Some(ActiveControl::ListActions(ListActionMenu {
            row: row_index,
            item_id: item.id.clone(),
            dropdown: Dropdown::open(resolved.len(), 0),
            actions: resolved,
        }));
    }

    fn dispatch_list_menu_action(&mut self, selected: usize, cx: &mut Context<Self>) {
        let Some(ActiveControl::ListActions(menu)) = self.active_control.take() else {
            return;
        };
        let row_index = menu.row;
        let item_id = menu.item_id;
        let Some(action) = menu.actions.into_iter().nth(selected) else {
            self.active_control = Some(ActiveControl::List);
            return;
        };
        self.active_control = Some(ActiveControl::List);
        self.dispatch_resolved_list_action(row_index, &item_id, action, cx);
    }

    fn dispatch_resolved_list_action(
        &mut self,
        row_index: usize,
        item_id: &str,
        action: ResolvedRowAction,
        cx: &mut Context<Self>,
    ) {
        let Some(RowControl::List { actions, items, .. }) =
            self.rows.get_mut(row_index).map(|row| &mut row.control)
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
        let runtime = self.runtime.clone();
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
        let Some(RowControl::List { items, .. }) =
            self.rows.get_mut(row_index).map(|row| &mut row.control)
        else {
            return;
        };
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        item.pending = false;
        item.error = Some(error);
    }

    fn begin_edit(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let edit = match &row.control {
            RowControl::Text(value) => value.clone(),
            RowControl::TextList(values) => values.join(", "),
            RowControl::Number { value, .. } => format_number(*value),
            RowControl::Toggle(_)
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
        self.active_control = Some(ActiveControl::Edit(edit));
    }

    fn step_number_edit(&mut self, direction: f64) -> bool {
        let Some(ActiveControl::Edit(edit)) = self.active_control.as_mut() else {
            return false;
        };
        let Some(RowControl::Number {
            value,
            min,
            max,
            step,
        }) = self.rows.get(self.selected).map(|row| &row.control)
        else {
            return false;
        };
        *edit = stepped_number(edit, *value, *min, *max, (*step).unwrap_or(1.0), direction);
        true
    }

    fn commit_edit(&mut self) {
        let Some(ActiveControl::Edit(edit)) = self.active_control.take() else {
            return;
        };
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        match &mut row.control {
            RowControl::Text(value) => *value = edit,
            RowControl::TextList(values) => {
                *values = edit
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .filter(|part| !part.is_empty())
                    .collect();
            }
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
            }
            RowControl::Toggle(_)
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
        self.persist();
    }

    fn persist(&mut self) {
        self.values = merged_config(&self.values, &self.rows);
        save_values(&self.panel.plugin_id, &self.path, &self.values);
    }

    fn sync_scroll(&mut self) {
        self.scroll_offset = scroll_offset_for(
            &self.rows,
            &self.current_visible_rows(),
            self.selected,
            self.scroll_offset,
            self.body_max,
            self.sections.len() <= 1,
        );
    }

    fn display_value(&self, index: usize) -> String {
        if index == self.selected {
            match &self.active_control {
                Some(ActiveControl::Edit(edit)) => return format!("{edit}_"),
                Some(ActiveControl::Wheel(wheel)) => return wheel.value.clone(),
                Some(ActiveControl::Dropdown(_))
                | Some(ActiveControl::List)
                | Some(ActiveControl::ListActions(_))
                | Some(ActiveControl::ObjectArray)
                | None => {}
            }
        }
        match &self.rows[index].control {
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
                text_or_placeholder(value, self.rows[index].placeholder.as_deref())
            }
            RowControl::TextList(values) => {
                text_or_placeholder(&values.join(", "), self.rows[index].placeholder.as_deref())
            }
            RowControl::Color(value) => value.clone(),
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
                    label.clone().unwrap_or_else(|| "loading...".into())
                }
            }
            RowControl::List { items, error, .. } => {
                if error.is_some() {
                    "unavailable".into()
                } else {
                    format!("{} found", items.len())
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
        if index == self.selected && matches!(self.active_control, Some(ActiveControl::Edit(_))) {
            return self.palette.label_text;
        }
        match &self.rows[index].control {
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
        DropdownStyle {
            bg: self.palette.dropdown_bg,
            bg_selected: self.palette.row_bg_selected,
            border: self.palette.row_border_selected,
            text: self.palette.label_text,
            text_selected: self.palette.section_text,
        }
    }

    fn wheel_style(&self) -> WheelStyle {
        WheelStyle {
            bg: self.palette.dropdown_bg,
            border: self.palette.row_border_selected,
            thumb_border: self.palette.section_text,
        }
    }

    fn render_value_cell(&self, index: usize) -> Div {
        match &self.rows[index].control {
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
                if self.rows[index].variant.as_deref() == Some("toggle") =>
            {
                return self.render_toggle_value(*active);
            }
            RowControl::Action { .. } => return self.render_action_value(index),
            RowControl::Unsupported { reason, .. } => {
                return div()
                    .text_xs()
                    .text_color(rgb(self.palette.status_muted))
                    .child(format!("Unsupported: {reason}"));
            }
            RowControl::Text(_)
            | RowControl::TextList(_)
            | RowControl::Color(_)
            | RowControl::Status { .. }
            | RowControl::List { .. }
            | RowControl::ObjectArray(_)
            | RowControl::Gamepad { .. }
            | RowControl::QrCode { .. } => {}
        }
        let mut cell = div().flex().flex_row().items_center().gap_2();
        if let RowControl::Status { tone, .. } = self.rows[index].control {
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
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(self.palette.panel_border))
                    .bg(rgb(color)),
            );
        }
        if let Some(accent) = self.option_accent(index) {
            cell = cell.child(div().w_2().h_2().rounded_full().bg(rgb(accent)));
        }
        cell.child(
            div()
                .text_sm()
                .text_color(rgb(self.value_color(index)))
                .child(self.display_value(index)),
        )
    }

    fn render_toggle_value(&self, active: bool) -> Div {
        let label_color = if active {
            self.palette.state_on
        } else {
            self.palette.label_text
        };
        let track_color = if active {
            self.palette.state_on
        } else {
            self.palette.dropdown_bg
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(label_color))
                    .child(if active { "On" } else { "Off" }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .when(active, |track| track.justify_end())
                    .when(!active, |track| track.justify_start())
                    .w(px(30.))
                    .h(px(16.))
                    .p(px(2.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(if active {
                        self.palette.state_on
                    } else {
                        self.palette.panel_border
                    }))
                    .bg(rgb(track_color))
                    .child(
                        div()
                            .w(px(10.))
                            .h(px(10.))
                            .rounded_full()
                            .bg(rgb(if active {
                                self.palette.window_bg
                            } else {
                                self.palette.label_text
                            })),
                    ),
            )
    }

    fn render_select_value(&self, index: usize) -> Div {
        let mut value = div().flex().flex_row().items_center().gap_2();
        if let Some(accent) = self.option_accent(index) {
            value = value.child(div().w_2().h_2().rounded_full().bg(rgb(accent)));
        }
        value
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(self.palette.panel_border))
            .bg(rgb(self.palette.dropdown_bg))
            .text_sm()
            .text_color(rgb(self.palette.label_text))
            .child(self.display_value(index))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.status_muted))
                    .child("▾"),
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
        let mut cell = div().flex().flex_row().items_center().gap_2();
        if self.rows[index].variant.as_deref() == Some("slider") {
            let edit = if index == self.selected {
                match &self.active_control {
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
                .gap_1()
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(rgb(self.palette.panel_border))
                .bg(rgb(self.palette.dropdown_bg))
                .text_sm()
                .text_color(rgb(self.palette.label_text))
                .child(self.display_value(index))
                .children(number_unit(&self.rows[index].id).map(|unit| {
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.status_muted))
                        .child(unit)
                })),
        )
    }

    fn render_action_value(&self, index: usize) -> Div {
        let variant = self.rows[index].variant.as_deref();
        let (background, border, text) = match variant {
            Some("ghost") => (
                rgba(self.palette.transparent_rgba),
                rgb(self.palette.panel_border),
                self.palette.label_text,
            ),
            Some("danger") => (
                rgba(self.palette.transparent_rgba),
                rgb(self.palette.state_off),
                self.palette.state_off,
            ),
            Some("primary") | None | Some(_) => (
                rgb(self.palette.row_bg_selected),
                rgb(self.palette.row_border_selected),
                self.palette.section_text,
            ),
        };
        let mut control = div().flex().flex_row().items_center().gap_1();
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
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(background)
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(text))
            .child(self.display_value(index))
    }

    fn action_is_busy(&self, index: usize) -> bool {
        action_shows_spinner(&self.rows[index])
    }

    fn swatch_color(&self, index: usize) -> Option<u32> {
        let RowControl::Color(value) = &self.rows[index].control else {
            return None;
        };
        let text = if index == self.selected {
            match &self.active_control {
                Some(ActiveControl::Edit(edit)) => edit,
                Some(ActiveControl::Wheel(wheel)) => return parsed_color(&wheel.value),
                Some(ActiveControl::Dropdown(_))
                | Some(ActiveControl::List)
                | Some(ActiveControl::ListActions(_))
                | Some(ActiveControl::ObjectArray)
                | None => value,
            }
        } else {
            value
        };
        parsed_color(text)
    }

    fn option_accent(&self, index: usize) -> Option<u32> {
        let RowControl::Select { options, index, .. } = &self.rows[index].control else {
            return None;
        };
        options.get(*index)?.accent
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let row = &self.rows[index];
        let mut container = div().flex().flex_col().gap_1();
        if self.sections.len() <= 1 {
            if let Some(section) = section_label_for(&self.rows, index) {
                container = container.child(
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.section_text))
                        .child(section.to_string()),
                );
            }
        }
        if matches!(row.control, RowControl::List { .. }) {
            return container.child(self.render_list(index, cx));
        }
        if matches!(row.control, RowControl::ObjectArray(_)) {
            return container.child(self.render_object_array(index, cx));
        }
        if matches!(row.control, RowControl::QrCode { .. }) {
            return container.child(self.render_qr_code(index));
        }
        if matches!(row.control, RowControl::Gamepad { .. }) {
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
        let label_group = div()
            .flex()
            .min_w_0()
            .flex_1()
            .flex_col()
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.section_text))
                    .child(label),
            )
            .when_some(row.description.clone(), |group, description| {
                group.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(self.palette.label_text))
                        .child(description),
                )
            });
        let selected = index == self.selected;
        let mut line = div()
            .id(("settings-row", index))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_3()
            .h(px(row_body_height(row)))
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(if selected {
                rgb(self.palette.row_border_selected)
            } else {
                rgba(self.palette.transparent_rgba)
            })
            .bg(if selected {
                rgb(self.palette.row_bg_selected)
            } else {
                rgba(self.palette.transparent_rgba)
            })
            .child(label_group)
            .child(self.render_value_cell(index));
        let row_bounds = Rc::clone(&self.row_bounds[index]);
        line = line.relative().child(
            canvas(
                move |bounds, _, _| row_bounds.set(Some(bounds)),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        if !matches!(
            row.control,
            RowControl::Status { .. }
                | RowControl::List { .. }
                | RowControl::Unsupported { .. }
                | RowControl::QrCode { .. }
        ) {
            line = line.cursor(CursorStyle::PointingHand).on_click(cx.listener(
                move |this, event: &ClickEvent, window, cx| {
                    if !event.standard_click() {
                        return;
                    }
                    this.selected = index;
                    this.activate(window, cx);
                    cx.notify();
                },
            ));
        }
        if selected {
            if let Some(ActiveControl::Dropdown(dropdown)) = &self.active_control {
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
                    line = line.child(dropdown.render_items_clickable(
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
                    ));
                }
            }
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
                    .px_2()
                    .text_xs()
                    .text_color(rgb(self.palette.state_off))
                    .child(error.clone()),
            );
        }
        container
    }

    fn render_gamepad(&self, index: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let row = &self.rows[index];
        let RowControl::Gamepad { monitor, .. } = &row.control else {
            return div().id(("settings-gamepad-empty", index));
        };
        let selected = index == self.selected;
        let palette = GamepadPalette {
            surface: self.palette.window_bg,
            raised: self.palette.dropdown_bg,
            border: self.palette.panel_border,
            text: self.palette.section_text,
            text_muted: self.palette.label_text,
            accent: self.palette.row_border_selected,
            success: self.palette.status_success,
            warning: self.palette.status_warning,
            danger: self.palette.status_danger,
        };
        div()
            .id(("settings-gamepad", index))
            .h(px(super::PANEL_GAMEPAD_HEIGHT))
            .rounded_lg()
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
                let already_selected = this.selected == index;
                this.selected = index;
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

    fn render_section_menu_item(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let section = &self.sections[index];
        let selected = index == self.selected_section;
        let mut item = div()
            .id(("settings-section", index))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if selected {
                self.palette.row_border_selected
            } else {
                self.palette.panel_border
            }))
            .bg(rgb(if selected {
                self.palette.row_bg_selected
            } else {
                self.palette.window_bg
            }))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(self.palette.section_text))
                            .child(section.label.clone()),
                    )
                    .when_some(section.description.clone(), |content, description| {
                        content.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(self.palette.label_text))
                                .child(description),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child("›"),
            )
            .cursor(CursorStyle::PointingHand);
        item = item.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            this.selected_section = index;
            this.open_selected_section();
            cx.notify();
        }));
        item
    }

    fn detail_heading(&self) -> Option<String> {
        if self.sections.len() <= 1 {
            return None;
        }
        self.active_section
            .and_then(|index| self.sections.get(index))
            .map(|section| format!("‹ {}", section.label))
    }

    fn detail_description(&self) -> Option<String> {
        self.active_section
            .and_then(|index| self.sections.get(index))
            .and_then(|section| section.description.clone())
    }

    fn render_list(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let row = &self.rows[index];
        let RowControl::List {
            active_label,
            active: runtime_active,
            empty_message,
            items,
            list,
            error,
            ..
        } = &row.control
        else {
            return div();
        };
        let list_active = index == self.selected
            && matches!(
                self.active_control,
                Some(ActiveControl::List) | Some(ActiveControl::ListActions(_))
            );
        let mut header_status = div().flex().items_center().gap_2();
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
            .gap_1()
            .h(px(row_body_height(row)))
            .overflow_hidden()
            .px_2()
            .py_1()
            .rounded_md();
        if index == self.selected {
            container = container
                .bg(rgb(self.palette.row_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.row_border_selected));
        }
        container = container.child(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .h(px(list_header_height(row)))
                .justify_between()
                .gap_3()
                .text_sm()
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
                                    .text_xs()
                                    .text_color(rgb(self.palette.label_text))
                                    .child(description),
                            )
                        }),
                )
                .child(header_status),
        );
        if let Some(error) = error {
            return container.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.state_off))
                    .child(error.clone()),
            );
        }
        if items.is_empty() {
            return container.child(
                div()
                    .py_2()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child(empty_message.clone()),
            );
        }
        for item_index in list.visible_range(items.len()) {
            container = container.child(self.render_list_item(index, item_index, list_active, cx));
        }
        container
    }

    fn render_qr_code(&self, index: usize) -> Div {
        let row = &self.rows[index];
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
            .gap_1()
            .h(px(row_body_height(row)))
            .overflow_hidden()
            .px_2()
            .py_1()
            .rounded_md();
        if index == self.selected {
            container = container
                .bg(rgb(self.palette.row_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.row_border_selected));
        }
        container = container.child(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .h(px(list_header_height(row)))
                .gap_3()
                .text_sm()
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
                                    .text_xs()
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
                    .unwrap_or_else(|| "Waiting...".into());
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(px(super::PANEL_QR_CODE_HEIGHT))
                    .text_sm()
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
                    .text_xs()
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
        let row = &self.rows[index];
        let RowControl::ObjectArray(state) = &row.control else {
            return div();
        };
        let mut container = self.render_block_frame(index, row, self.display_value(index));
        let Some(draft) = state.draft.as_ref() else {
            for entry in state.item_window() {
                container = container.child(self.render_object_array_entry(index, entry, cx));
            }
            let add = state.entries.len();
            return container.child(self.render_object_array_entry(index, add, cx));
        };
        for (field_index, field) in draft.fields.iter().enumerate() {
            container = container.child(self.render_draft_field(index, field_index, field, cx));
        }
        container.child(self.render_draft_save(index, draft.save_entry_selected(), cx))
    }

    fn render_block_frame(&self, index: usize, row: &Row, value: String) -> Div {
        let mut container = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .h(px(row_body_height(row)))
            .overflow_hidden()
            .px_2()
            .py_1()
            .rounded_md();
        if index == self.selected {
            container = container
                .bg(rgb(self.palette.row_bg_selected))
                .border_1()
                .border_color(rgb(self.palette.row_border_selected));
        }
        container.child(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .h(px(list_header_height(row)))
                .justify_between()
                .gap_3()
                .text_sm()
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
                                    .text_xs()
                                    .text_color(rgb(self.palette.label_text))
                                    .child(description),
                            )
                        }),
                )
                .child(div().text_color(rgb(self.value_color(index))).child(value)),
        )
    }

    fn object_array_is_active(&self, index: usize) -> bool {
        index == self.selected && matches!(self.active_control, Some(ActiveControl::ObjectArray))
    }

    fn render_object_array_entry(
        &self,
        index: usize,
        entry: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let RowControl::ObjectArray(state) = &self.rows[index].control else {
            return div().id(("settings-object-empty", entry_id(index, entry)));
        };
        let active = self.object_array_is_active(index);
        let selected = active && entry == state.list.selected;
        let stored = entry < state.entries.len();
        let body = if stored {
            self.render_chip_row(state.chips(entry))
        } else {
            div().flex().flex_row().items_center().child(
                div()
                    .text_sm()
                    .text_color(rgb(self.palette.state_on))
                    .child("+ Add"),
            )
        };
        self.object_array_line(index, entry, selected)
            .child(body)
            .when(stored, |line| {
                line.child(self.render_entry_remove(index, entry, cx))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if !event.standard_click() {
                    return;
                }
                this.select_object_array_entry(index, entry, window, cx);
                cx.notify();
            }))
    }

    fn render_entry_remove(
        &self,
        index: usize,
        entry: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("settings-object-remove", entry_id(index, entry)))
            .flex_none()
            .px_1p5()
            .rounded_sm()
            .text_xs()
            .text_color(rgb(self.palette.status_muted))
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.text_color(rgb(self.palette.state_off)))
            .child("\u{00d7}")
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                cx.stop_propagation();
                this.remove_object_array_entry(index, entry);
                cx.notify();
            }))
    }

    fn remove_object_array_entry(&mut self, index: usize, entry: usize) {
        let Some(RowControl::ObjectArray(state)) =
            self.rows.get_mut(index).map(|row| &mut row.control)
        else {
            return;
        };
        state.list.selected = entry;
        if state.remove_selected() {
            self.persist();
        }
        self.sync_scroll();
    }

    fn object_array_line(&self, index: usize, entry: usize, selected: bool) -> Stateful<Div> {
        let mut line = div()
            .id(("settings-object-entry", entry_id(index, entry)))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .flex_none()
            .h(px(super::PANEL_OBJECT_ROW_HEIGHT))
            .overflow_hidden()
            .px_2()
            .rounded_sm()
            .cursor(CursorStyle::PointingHand);
        if selected {
            line = line
                .bg(rgb(self.palette.dropdown_bg))
                .border_l_2()
                .border_color(rgb(self.palette.row_border_selected));
        }
        line
    }

    fn render_chip_row(&self, chips: ItemChips) -> Div {
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .flex_1()
            .min_w_0()
            .overflow_hidden();
        for chip in &chips.from {
            row = row.child(self.render_chip(chip.label.clone(), chip.tone));
        }
        for chip in &chips.rest {
            row = row.child(self.render_chip(chip.label.clone(), chip.tone));
        }
        if chips.is_directional() {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.label_text))
                    .child("\u{2192}"),
            );
        }
        for chip in &chips.to {
            row = row.child(self.render_chip(chip.label.clone(), chip.tone));
        }
        for flag in &chips.flags {
            row = row.child(self.render_chip(flag.clone(), ChipTone::Plain));
        }
        row
    }

    fn render_chip(&self, label: String, tone: ChipTone) -> Div {
        let (text, background) = match tone {
            ChipTone::Modifier => (self.palette.state_on, self.palette.dropdown_bg),
            ChipTone::Key => (self.palette.section_text, self.palette.dropdown_bg),
            ChipTone::Plain => (self.palette.label_text, self.palette.transparent_rgba),
        };
        div()
            .flex_none()
            .px_1p5()
            .rounded_sm()
            .border_1()
            .border_color(rgb(self.palette.panel_border))
            .bg(match tone {
                ChipTone::Plain => rgba(background),
                _ => rgb(background),
            })
            .text_xs()
            .text_color(rgb(text))
            .child(label)
    }

    fn render_draft_field(
        &self,
        index: usize,
        field_index: usize,
        field: &DraftField,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let RowControl::ObjectArray(state) = &self.rows[index].control else {
            return div().id(("settings-draft-empty", entry_id(index, field_index)));
        };
        let selected = state
            .draft
            .as_ref()
            .is_some_and(|draft| draft.selected == field_index);
        self.object_array_line(index, field_index, selected)
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(rgb(self.palette.label_text))
                    .child(pretty_label(&field.key)),
            )
            .child(self.render_draft_value(field, selected))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.select_draft_field(index, field_index);
                cx.notify();
            }))
    }

    fn render_draft_value(&self, field: &DraftField, selected: bool) -> Div {
        let mut value = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap_1()
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
                value = value.child(self.render_mod_chip(option.clone(), on, focused));
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
                .text_sm()
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

    fn render_mod_chip(&self, label: String, on: bool, focused: bool) -> Div {
        div()
            .flex_none()
            .px_1p5()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if focused {
                self.palette.row_border_selected
            } else {
                self.palette.panel_border
            }))
            .bg(if on {
                rgb(self.palette.dropdown_bg)
            } else {
                rgba(self.palette.transparent_rgba)
            })
            .text_xs()
            .text_color(rgb(if on {
                self.palette.state_on
            } else {
                self.palette.status_muted
            }))
            .child(label)
    }

    fn render_draft_save(
        &self,
        index: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let entry = usize::MAX;
        self.object_array_line(index, entry, selected)
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(self.palette.state_on))
                    .child("Save"),
            )
            .child(
                div()
                    .text_xs()
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
    }

    fn select_object_array_entry(
        &mut self,
        index: usize,
        entry: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reactivate = self.selected != index;
        self.selected = index;
        if reactivate {
            self.activate(window, cx);
        }
        let already_selected = !reactivate && self.object_array_entry_selected(index, entry);
        let Some(RowControl::ObjectArray(state)) =
            self.rows.get_mut(index).map(|row| &mut row.control)
        else {
            return;
        };
        state.list.selected = entry;
        state.list.sync(state.entry_count());
        if already_selected {
            state.open_draft();
        }
        self.active_control = Some(ActiveControl::ObjectArray);
        self.sync_scroll();
    }

    fn object_array_entry_selected(&self, index: usize, entry: usize) -> bool {
        let Some(RowControl::ObjectArray(state)) = self.rows.get(index).map(|row| &row.control)
        else {
            return false;
        };
        self.object_array_is_active(index) && state.list.selected == entry
    }

    fn select_draft_field(&mut self, index: usize, field_index: usize) {
        let Some(RowControl::ObjectArray(state)) =
            self.rows.get_mut(index).map(|row| &mut row.control)
        else {
            return;
        };
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        draft.selected = field_index;
    }

    fn commit_object_array_draft(&mut self, index: usize) {
        let Some(RowControl::ObjectArray(state)) =
            self.rows.get_mut(index).map(|row| &mut row.control)
        else {
            return;
        };
        if state.commit_draft() {
            self.persist();
        }
        self.sync_scroll();
    }

    fn render_list_item(
        &self,
        index: usize,
        item_index: usize,
        list_active: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let RowControl::List {
            actions,
            items,
            list,
            ..
        } = &self.rows[index].control
        else {
            return div().id((
                "settings-list-item-empty",
                ((index as u64) << 32) | item_index as u64,
            ));
        };
        let item = &items[item_index];
        let selected = list_active && item_index == list.selected;
        let action = primary_list_item_action(actions, item);
        let title = self.render_list_item_title(index, item_index, selected, item, action, cx);
        let accent = item
            .accent
            .map(|tone| rgb(status_tone_color(self.palette, tone)))
            .unwrap_or_else(|| rgba(self.palette.transparent_rgba));
        let mut item_row = div()
            .id((
                "settings-list-item",
                ((index as u64) << 32) | item_index as u64,
            ))
            .flex()
            .flex_col()
            .flex_none()
            .h(px(super::PANEL_LIST_ITEM_HEIGHT))
            .overflow_hidden()
            .border_l_2()
            .border_color(accent)
            .px_2()
            .py_1()
            .rounded_sm()
            .cursor(CursorStyle::PointingHand)
            .child(title);
        item_row = item_row.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            this.select_list_item(index, item_index);
            cx.notify();
        }));
        if selected {
            item_row = item_row.bg(rgb(self.palette.dropdown_bg));
        }
        let Some(detail) = item.error.as_ref().or(item.subtitle.as_ref()) else {
            return item_row;
        };
        item_row.child(
            div()
                .text_xs()
                .text_color(rgb(if item.error.is_some() {
                    self.palette.state_off
                } else {
                    self.palette.label_text
                }))
                .child(detail.clone()),
        )
    }

    fn render_list_item_title(
        &self,
        index: usize,
        item_index: usize,
        selected: bool,
        item: &super::rows::ListItem,
        action: Option<qol_config::contract::ResolvedRowAction>,
        cx: &mut Context<Self>,
    ) -> Div {
        let badge = item.badge.as_ref().map(|label| {
            StatusIndicator::new(
                (
                    "settings-list-item-status",
                    ((index as u64) << 32) | item_index as u64,
                ),
                label.clone(),
                rgb(status_tone_color(self.palette, item.effective_badge_tone())),
            )
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_sm()
                    .text_color(rgb(if selected {
                        self.palette.section_text
                    } else {
                        self.palette.label_text
                    }))
                    .child(item.label.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .children(badge)
                    .child(self.render_list_action(index, item_index, selected, item, action, cx)),
            )
    }

    fn render_list_action(
        &self,
        index: usize,
        item_index: usize,
        selected: bool,
        item: &super::rows::ListItem,
        action: Option<qol_config::contract::ResolvedRowAction>,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut cell = div().flex().flex_none().flex_row().items_center().gap_2();
        if item.pending {
            cell = cell.child(Spinner::new(
                (
                    "settings-list-action-spinner",
                    ((index as u64) << 32) | item_index as u64,
                ),
                rgb(self.palette.state_on),
            ));
        }
        let Some(action) = action else {
            return cell;
        };
        let action_count = match &self.rows[index].control {
            RowControl::List { actions, .. } => list_item_actions(actions, item).len(),
            _ => 0,
        };
        let label = list_action_affordance(&action.label, action_count);
        let mut affordance = div()
            .id((
                "settings-list-action",
                ((index as u64) << 32) | item_index as u64,
            ))
            .px_1()
            .rounded_sm()
            .border_1()
            .border_color(if selected {
                rgb(self.palette.row_border_selected)
            } else {
                rgb(self.palette.panel_border)
            })
            .text_xs()
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
                    this.select_list_item(index, item_index);
                    if action_count > 1 {
                        this.open_list_actions();
                        cx.notify();
                        return;
                    }
                    this.dispatch_list_action(cx);
                    cx.notify();
                }));
        }
        if let Some(ActiveControl::ListActions(menu)) = &self.active_control {
            if menu.row == index && menu.item_id == item.id {
                let labels = menu
                    .actions
                    .iter()
                    .map(|action| action.label.clone())
                    .collect::<Vec<_>>();
                let view = cx.weak_entity();
                affordance = affordance.child(menu.dropdown.render_clickable(
                    format!("settings-list-actions-{index}-{item_index}"),
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
                ));
            }
        }
        cell.child(affordance)
    }
}

impl Focusable for SettingsPanelView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.frame_pump_armed {
            self.pump_frame_paced_samples(window, cx);
        }
        let items: Vec<AnyElement> = if self.section_menu_is_open() {
            (0..self.sections.len())
                .map(|index| self.render_section_menu_item(index, cx).into_any_element())
                .collect()
        } else {
            visible_row_window(
                &self.rows,
                &self.current_visible_rows(),
                self.scroll_offset,
                self.body_max,
                self.sections.len() <= 1,
            )
            .into_iter()
            .map(|index| self.render_row(index, cx).into_any_element())
            .collect()
        };
        let detail_heading = self.detail_heading();
        let detail_description = self.detail_description();
        div()
            .id("settings-panel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(rgb(self.palette.panel_border))
            .bg(rgb(self.palette.window_bg))
            .child(
                div().w_full().pt_4().px_4().pb_1().panel_drag_area().child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(self.palette.section_text))
                                .child(self.panel.heading.clone()),
                        )
                        .when_some(detail_heading, |header, detail| {
                            header.child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(self.palette.row_border_selected))
                                    .child(detail),
                            )
                        })
                        .when_some(detail_description, |header, description| {
                            header.child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(self.palette.label_text))
                                    .child(description),
                            )
                        }),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .pb_4()
                    .children(items)
                    .child(self.measure_canvas()),
            )
    }
}

impl SettingsPanelView {
    fn measure_canvas(&self) -> impl IntoElement {
        let dismisser = self.dismisser.clone();
        let floor = super::PANEL_CHROME_HEIGHT + super::PANEL_ROW_HEIGHT;
        let cap = self.height_cap;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let target = (bounds.bottom().to_f64() as f32).clamp(floor, cap);
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
        format!("{value}")
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
        return "working...".into();
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
    Left,
    Activate,
    CommitEdit,
    Backspace,
    Insert(String),
    Close,
    CancelEdit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListIntent {
    Up,
    Down,
    Activate,
    Actions,
    Close,
}

fn list_intent(key: &str) -> Option<ListIntent> {
    match key {
        "up" => Some(ListIntent::Up),
        "down" => Some(ListIntent::Down),
        "enter" | "return" => Some(ListIntent::Activate),
        "space" | "right" => Some(ListIntent::Actions),
        "escape" | "left" => Some(ListIntent::Close),
        _ => None,
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
        "left" => Some(Intent::Left),
        "enter" | "return" => Some(Intent::Activate),
        "escape" => Some(Intent::Close),
        _ => None,
    }
}

pub(super) fn row_height(row: &Row) -> f32 {
    let header = if row.section_label.is_some() {
        super::PANEL_SECTION_HEADER_HEIGHT
    } else {
        0.0
    };
    row_body_height(row) + header
}

fn visible_row_height(rows: &[Row], index: usize, show_section_headers: bool) -> f32 {
    let header = if show_section_headers && section_label_for(rows, index).is_some() {
        super::PANEL_SECTION_HEADER_HEIGHT
    } else {
        0.0
    };
    row_body_height(&rows[index]) + header
}

pub(super) fn row_body_height(row: &Row) -> f32 {
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
    if let RowControl::ObjectArray(state) = &row.control {
        return object_array_body_height(row, state);
    }
    if matches!(row.control, RowControl::List { .. }) {
        return super::PANEL_LIST_HEIGHT
            + if row.description.is_some() {
                super::PANEL_LIST_DESCRIPTION_HEIGHT
            } else {
                0.0
            };
    }
    if row.description.is_some() {
        return super::PANEL_DESCRIBED_ROW_HEIGHT;
    }
    super::PANEL_ROW_HEIGHT
}

fn object_array_body_height(row: &Row, state: &ObjectArrayState) -> f32 {
    let entries = match state.draft.as_ref() {
        Some(draft) => draft.entry_count(),
        None => state.item_window().len() + 1,
    };
    super::PANEL_LIST_PADDING_Y
        + list_header_height(row)
        + entries as f32 * (super::PANEL_OBJECT_ROW_HEIGHT + super::PANEL_LIST_GAP)
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

fn visible_row_window(
    rows: &[Row],
    visible: &[usize],
    offset: usize,
    body_max: f32,
    show_section_headers: bool,
) -> Vec<usize> {
    let start = visible
        .iter()
        .position(|index| *index >= offset)
        .unwrap_or(visible.len());
    let mut used = 0.0;
    let mut window = Vec::new();
    for index in visible.iter().copied().skip(start) {
        used += visible_row_height(rows, index, show_section_headers);
        if used > body_max && !window.is_empty() {
            break;
        }
        window.push(index);
    }
    window
}

fn scroll_offset_for(
    rows: &[Row],
    visible: &[usize],
    selected: usize,
    offset: usize,
    body_max: f32,
    show_section_headers: bool,
) -> usize {
    let Some(selected_position) = visible.iter().position(|index| *index == selected) else {
        return visible.first().copied().unwrap_or(0);
    };
    let mut offset_position = visible
        .iter()
        .position(|index| *index >= offset)
        .unwrap_or(selected_position);
    if selected_position < offset_position {
        return selected;
    }
    while !visible_row_window(
        rows,
        visible,
        visible[offset_position],
        body_max,
        show_section_headers,
    )
    .contains(&selected)
    {
        offset_position += 1;
    }
    visible[offset_position]
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

fn initial_active_section(section_count: usize) -> Option<usize> {
    if section_count == 1 {
        return Some(0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        action_refresh_payload, action_shows_spinner, action_value_label, adjacent_visible_row,
        binary_state_label, horizontal_step_direction, initial_active_section, intent,
        list_action_affordance, list_intent, number_preview, number_unit, parsed_color,
        parsed_number, row_body_height, scroll_offset_for, slider_fraction, stepped_number,
        text_or_placeholder, visible_row_window, Intent, ListIntent, Row, RowControl,
    };
    use crate::scroll_list::ScrollList;
    use crate::settings_panel::rows::{rows_from_resolved, visible_row_indices};

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
                visibility: None,
                control: RowControl::Toggle(false),
            })
            .collect()
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
            visibility: None,
            control: RowControl::List {
                query: "items".into(),
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
                empty_message: "No items".into(),
                items: Vec::new(),
                list: ScrollList::new(super::super::rows::LIST_MAX_VISIBLE),
                error: None,
            },
        }
    }

    #[test]
    fn visible_range_windows_rows_by_height_budget() {
        let rows = rows(&[true, false, false, false]);
        let two_plain_rows = 2.0 * super::super::PANEL_ROW_HEIGHT;
        let cases = [
            (0, two_plain_rows, 0..1),
            (1, two_plain_rows, 1..3),
            (3, two_plain_rows, 3..4),
            (0, 1000.0, 0..4),
            (0, 1.0, 0..1),
        ];
        let visible = (0..rows.len()).collect::<Vec<_>>();
        for (offset, budget, expected) in cases {
            assert_eq!(
                visible_row_window(&rows, &visible, offset, budget, true),
                expected.collect::<Vec<_>>(),
                "offset {offset} budget {budget}"
            );
        }
    }

    #[test]
    fn visible_range_reserves_the_full_runtime_list_height() {
        let mut panel_rows = rows(&[true]);
        panel_rows.push(list_row());
        panel_rows.extend(rows(&[true, false, false, false]));
        let visible = (0..panel_rows.len()).collect::<Vec<_>>();

        assert_eq!(
            visible_row_window(&panel_rows, &visible, 0, 480.0, true),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn scroll_offset_keeps_selection_visible_in_both_directions() {
        let rows = rows(&[false, false, false, false]);
        let two_rows = 2.0 * super::super::PANEL_ROW_HEIGHT;
        let cases = [(0, 0, 0), (1, 0, 0), (2, 0, 1), (3, 1, 2), (0, 2, 0)];
        let visible = (0..rows.len()).collect::<Vec<_>>();
        for (selected, offset, expected) in cases {
            assert_eq!(
                scroll_offset_for(&rows, &visible, selected, offset, two_rows, true),
                expected,
                "selected {selected} offset {offset}"
            );
        }
    }

    #[test]
    fn descriptions_expand_only_the_rows_that_render_them() {
        let mut plain = rows(&[false]).remove(0);
        let plain_height = row_body_height(&plain);
        plain.description = Some("Helpful context".into());

        assert_eq!(plain_height, super::super::PANEL_ROW_HEIGHT);
        assert_eq!(
            row_body_height(&plain),
            super::super::PANEL_DESCRIBED_ROW_HEIGHT
        );

        let mut list = list_row();
        let plain_list_height = row_body_height(&list);
        list.description = Some("Live devices".into());
        assert_eq!(plain_list_height, super::super::PANEL_LIST_HEIGHT);
        assert_eq!(
            row_body_height(&list),
            super::super::PANEL_LIST_HEIGHT + super::super::PANEL_LIST_DESCRIPTION_HEIGHT
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
        let rows = rows_from_resolved(&resolved);
        let visible = visible_row_indices(&rows);

        assert_eq!(adjacent_visible_row(&visible, 0, 1), 2);
        assert_eq!(adjacent_visible_row(&visible, 2, -1), 0);
    }

    #[test]
    fn list_intent_activates_enter_instead_of_closing_navigation() {
        let cases = [
            ("up", Some(ListIntent::Up)),
            ("down", Some(ListIntent::Down)),
            ("enter", Some(ListIntent::Activate)),
            ("return", Some(ListIntent::Activate)),
            ("space", Some(ListIntent::Actions)),
            ("right", Some(ListIntent::Actions)),
            ("escape", Some(ListIntent::Close)),
            ("left", Some(ListIntent::Close)),
            ("tab", None),
        ];
        for (key, expected) in cases {
            assert_eq!(list_intent(key), expected, "key: {key}");
        }
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
            (false, true, false, false, false, "working..."),
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
    fn multiple_contract_sections_open_in_the_shared_section_menu() {
        let cases = [(0, None), (1, Some(0)), (2, None), (8, None)];
        for (count, expected) in cases {
            assert_eq!(initial_active_section(count), expected, "count: {count}");
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
    fn inactive_inputs_require_enter_before_control_keys_take_effect() {
        let cases = [
            ("up", None, false, Some(Intent::Up)),
            ("down", None, false, Some(Intent::Down)),
            ("left", None, false, Some(Intent::Left)),
            ("right", None, false, None),
            ("space", None, false, None),
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
}
