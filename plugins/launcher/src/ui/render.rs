use std::path::PathBuf;
use std::time::Instant;

use gpui::prelude::FluentBuilder;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{Motion, TextStyle};
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::*;

use crate::flow::FlowVerdict;

use super::about::{self, About};
use super::details::Want;
use super::feedback::{self, Copied, Cue, Feedback, Placed};
use super::files::{self, FileCard, Panel};
use super::layout::{
    content_width, header_window_height, list_gap, window_height_for, window_height_for_detail,
    window_height_for_trail, window_height_with_panel, APP_CARD_MIN, APP_GAP, FILES_PANEL_MIN,
    FILE_CARD_MAX, FILE_INSET, FLOW_ROW_HEIGHT, HEADER_HEIGHT, LIST_PAD_Y, PANEL_GAP, WINDOW_WIDTH,
};
use super::sizing;
use super::state::PanelItem;
#[cfg(debug_assertions)]
use super::trace;
use super::transitions::Prop;
use super::view;
use super::LauncherView;
use crate::discovery::search::{ResultItem, SearchMode};

#[cfg(debug_assertions)]
static LAST_RENDER_US: AtomicU64 = AtomicU64::new(0);
#[cfg(debug_assertions)]
static RENDER_COUNT: AtomicU64 = AtomicU64::new(0);

static REGISTER_NATIVE_DISPLAY: std::sync::Once = std::sync::Once::new();
const FEEDBACK_LIFE: std::time::Duration = std::time::Duration::from_secs(2);
const GHOST_INK_MIX: f32 = 0.6;
const GHOST_EDGE: f32 = 1.5;
const GHOST_EDGE_ALPHA: u16 = 550;
const GHOST_FILL_ALPHA: u16 = 180;
const GHOST_OUTSET: f32 = 2.0;
const WINDOW_KEY: &str = "launcher:window";
const PANEL_KEY: &str = "launcher:panel";
const LIST_KEY: &str = "launcher:list";

impl Focusable for LauncherView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        qol_gpui::kit::enter_window(qol_gpui::kit::WindowLook::Live);
        REGISTER_NATIVE_DISPLAY.call_once(|| {
            qol_gpui::popup_window::register_native_display(window);
        });

        if self.dismiss_requested {
            let from = self.dismiss_requested_from;
            self.dismiss_requested = false;
            self.dismiss_requested_from = "requested";
            self.hide_to_ghost(from, window);
        }

        if self.dismiss_sub.is_none() {
            self.dismiss_sub = Some(qol_gpui::ghost::track_dismiss_held(
                "launcher",
                &self.focus_handle,
                window,
                |this: &Self| this.blur_guard.guard_until(),
                |this: &Self| this.is_showing,
                |_| qol_gpui::popup_window::input_held(),
                cx,
                |this, window, _cx| {
                    this.hide_to_ghost("blur", window);
                },
            ));
            if !self.is_showing {
                qol_gpui::popup_window::hide_invisible(&self.window_title);
            }
        }

        #[cfg(debug_assertions)]
        let (render_start, gap_us) = {
            let render_start = std::time::Instant::now();
            let now_abs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros() as u64;
            let prev = LAST_RENDER_US.swap(now_abs, Ordering::Relaxed);
            let gap_us = if prev > 0 {
                now_abs.saturating_sub(prev)
            } else {
                0
            };
            (render_start, gap_us)
        };

        if !self.is_showing {
            #[cfg(debug_assertions)]
            {
                let total_us = render_start.elapsed().as_micros();
                trace::render(
                    self,
                    window,
                    trace::RenderSample {
                        result_count: self.store.result_count(),
                        visible_rows: 0,
                        scroll_offset: 0,
                        hidden_above: 0,
                        hidden_below: 0,
                        results_height: 0.0,
                        target_height: header_window_height(),
                        selected_name: String::new(),
                        resize: None,
                        total_us,
                        filter_us: 0,
                        rows_us: 0,
                        gap_us,
                    },
                );
            }

            return div()
                .text(TextStyle::Value)
                .id("launcher")
                .track_focus(&self.focus_handle)
                .w(px(WINDOW_WIDTH))
                .h(px(header_window_height()))
                .overflow_hidden()
                .bg(view::bg_color());
        }

        if self.state.feedback.as_ref().is_some_and(|feedback| {
            !feedback.closing() && feedback.started.elapsed() > FEEDBACK_LIFE
        }) {
            self.state.feedback = None;
        }

        if self.is_showing {
            self.ensure_click_away_monitor(cx);
            self.refresh_recent_files(cx);
            self.sync_entries_from_shared();
            if !self.entry_watch_running {
                self.start_entry_watch(cx);
            }
        }

        #[cfg(debug_assertions)]
        let t0 = std::time::Instant::now();
        let now = Instant::now();
        let flow_active = self.state.flow.is_some();
        if !flow_active {
            self.store.ensure_filtered(
                self.state.query.text(),
                self.state.mode,
                self.state.fuzziness,
            );
        }
        #[cfg(debug_assertions)]
        let filter_us = t0.elapsed().as_micros();

        let result_count = if flow_active {
            self.state.flow_result_count()
        } else {
            self.store.result_count()
        };
        self.state.sync_result_window(result_count);
        let trail_focus = self.state.flow_trail_focus();
        let visible_range = self.state.scroll_list.visible_range(result_count);
        let visible = visible_range.len();
        let scroll_offset = visible_range.start;
        let nav_cues = self.state.nav_cues();
        #[cfg(debug_assertions)]
        let hidden_above = visible_range.start;
        #[cfg(debug_assertions)]
        let hidden_below = result_count.saturating_sub(visible_range.end);
        if nav_cues.decayed_momentum > 0 {
            self.ensure_trail_decay_tick(cx);
        }
        self.state.take_edge_hit();
        let detail = self.state.flow_detail_open();
        let detail_ready =
            detail
                && self.state.flow.as_ref().is_some_and(|session| {
                    session.rows.get(self.state.scroll_list.selected).is_some()
                });
        let flow_verdict = self.state.flow_verdict();
        let files_mode = !flow_active && self.state.mode == SearchMode::Files;
        let width = content_width(flow_active);
        #[cfg(debug_assertions)]
        let t1 = std::time::Instant::now();
        let VisibleRows {
            rows,
            heights: row_heights,
            panel,
            panel_height,
            wants,
            placed,
            overlay,
            shown: list_shown,
        } = if flow_active {
            VisibleRows::default()
        } else {
            self.build_visible_rows(scroll_offset, visible, now, window, cx)
        };
        if !flow_active {
            self.last_layout = placed;
        }
        self.load_details(wants, cx);
        let content_height = if detail_ready {
            window_height_for_detail()
        } else if flow_active {
            if result_count > 0 {
                window_height_for_trail(
                    matches!(flow_verdict, FlowVerdict::Vague | FlowVerdict::Checking),
                    self.state
                        .flow
                        .as_ref()
                        .and_then(|session| {
                            view::answer_lead(
                                matches!(flow_verdict, FlowVerdict::Vague | FlowVerdict::Checking),
                                &session.rows,
                            )
                        })
                        .map_or(qol_gpui::trail::motion::ROW_H, |_| view::CARD_HEIGHT),
                )
            } else if flow_verdict == FlowVerdict::NoMemory {
                window_height_for(1, FLOW_ROW_HEIGHT)
            } else {
                window_height_for(0, FLOW_ROW_HEIGHT)
            }
        } else {
            self.transitions.value(
                WINDOW_KEY,
                Prop::Height,
                window_height_with_panel(&row_heights, list_gap(self.state.mode), panel_height),
                Motion::SETTLE,
                now,
            )
        };
        if flow_active {
            self.transitions.retain(|key| key != WINDOW_KEY);
            self.frame = None;
        }
        if self.transitions.moving(now) {
            window.request_animation_frame();
        }
        let target_height = match self.menu_kind {
            Some(super::menu::MenuKind::Help) => {
                content_height.max(HEADER_HEIGHT + self.menu_height())
            }
            Some(super::menu::MenuKind::Options) => {
                content_height.max(HEADER_HEIGHT + self.menu_height() + 8.0)
            }
            None => content_height,
        };
        let results_height = content_height
            - header_window_height()
            - if flow_active {
                qol_gpui::theme::HEIGHT_HINT_BAR
            } else {
                0.0
            };

        #[cfg(debug_assertions)]
        let selected_name = self
            .store
            .get(self.state.scroll_list.selected)
            .map(|scored| self.store.name(scored))
            .unwrap_or("")
            .to_string();
        let kit = qol_gpui::kit::kit();
        #[cfg(debug_assertions)]
        {
            let rows_us = t1.elapsed().as_micros();
            let total_us = render_start.elapsed().as_micros();
            trace::render(
                self,
                window,
                trace::RenderSample {
                    result_count,
                    visible_rows: visible,
                    scroll_offset,
                    hidden_above,
                    hidden_below,
                    results_height,
                    target_height,
                    selected_name,
                    resize: None,
                    total_us,
                    filter_us,
                    rows_us,
                    gap_us,
                },
            );
            let n = RENDER_COUNT.fetch_add(1, Ordering::Relaxed);
            if n.is_multiple_of(10) {
                eprintln!(
                    "[render #{n}] total={total_us}us filter={filter_us}us rows={rows_us}us gap={gap_us}us visible={visible} results={result_count} q={:?}",
                    self.state.query.text()
                );
            }
        }
        let flow_prompt = self
            .state
            .flow
            .as_ref()
            .map(|session| session.entry.prompt.clone());
        let flow_entry = self
            .state
            .flow
            .as_ref()
            .map(|session| session.entry.clone());
        let flow_pending = self
            .state
            .flow
            .as_ref()
            .is_some_and(|session| session.pending);
        let menu = self.menu_kind.map(|_| self.menu_overlay(cx));
        let launcher =
            kit.window()
                .text(TextStyle::Value)
                .id("launcher")
                .track_focus(&self.focus_handle)
                .w(px(width))
                .h(px(target_height))
                .flex()
                .flex_col()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if !this.is_showing {
                        return;
                    }
                    this.handle_key(event, window, cx);
                }))
                .child(view::search_bar(
                    &self.state.query,
                    self.state.launch_error.as_deref(),
                    view::SearchBarStatus {
                        mode: (!flow_active).then_some(self.state.mode),
                        pending: flow_pending,
                        list_focused: self.state.list_focused,
                        panel_active: files_mode && self.state.list_focused,
                        feedback: self.state.feedback.as_ref(),
                    },
                    flow_prompt
                        .as_deref()
                        .unwrap_or(view::search_placeholder(self.state.mode)),
                    width,
                    window,
                    cx,
                ))
                .when(result_count > 0, |root| {
                    if flow_active {
                        if detail_ready {
                            match self.state.flow.as_ref().and_then(|session| {
                                session.rows.get(self.state.scroll_list.selected)
                            }) {
                                Some(row) => root.child(
                                    div()
                                        .id("launcher-results")
                                        .h(px(results_height))
                                        .w_full()
                                        .overflow_hidden()
                                        .bg(view::bg_color())
                                        .child(view::detail_body(
                                            &kit,
                                            row,
                                            results_height,
                                            &self.detail_scroll,
                                        )),
                                ),
                                None => root,
                            }
                        } else {
                            match self.state.flow.as_ref().zip(trail_focus) {
                            Some((session, focus)) => root.child(
                                div()
                                    .id("launcher-results")
                                    .h(px(results_height))
                                    .w_full()
                                    .overflow_hidden()
                                    .bg(view::bg_color())
                                    .on_scroll_wheel(cx.listener(
                                        move |this: &mut Self,
                                              event: &ScrollWheelEvent,
                                              _window,
                                              cx: &mut Context<Self>| {
                                            let rows = qol_gpui::scroll_list::wheel_rows(
                                                &event.delta,
                                                qol_gpui::trail::motion::ROW_H,
                                            );
                                            for _ in 0..rows.max(0) as usize {
                                                this.state.scroll_list.move_down(result_count);
                                            }
                                            for _ in 0..(-rows).max(0) as usize {
                                                this.state.scroll_list.move_up();
                                            }
                                            cx.notify();
                                        },
                                    ))
                                    .child(view::trail_body(
                                        &kit,
                                        &session.rows,
                                        focus,
                                        flow_verdict,
                                    )),
                            ),
                            None => root,
                        }
                        }
                    } else {
                        root.child(self.results_view(
                            rows,
                            overlay,
                            panel,
                            list_shown,
                            results_height,
                            files_mode,
                            scroll_offset,
                            visible,
                            result_count,
                            cx,
                        ))
                    }
                })
                .when(
                    flow_active && result_count == 0 && flow_verdict == FlowVerdict::NoMemory,
                    |root| {
                        root.child(
                            div()
                                .id("launcher-results")
                                .h(px(results_height))
                                .w_full()
                                .overflow_hidden()
                                .bg(view::bg_color())
                                .child(view::flow_empty_state(&kit)),
                        )
                    },
                )
                .when(flow_active, |root| {
                    root.child(if detail_ready {
                        view::hint_bar_detail()
                    } else {
                        view::hint_bar_flow(flow_entry.as_ref().expect("active flow has an entry"))
                    })
                })
                .when_some(menu, |root, menu| root.child(menu));
        div()
            .id("launcher-frame")
            .w(px(WINDOW_WIDTH))
            .flex()
            .justify_center()
            .child(launcher)
    }
}

#[derive(Default)]
struct VisibleRows {
    rows: Vec<AnyElement>,
    heights: Vec<f32>,
    panel: Option<Div>,
    panel_height: f32,
    wants: Vec<Want>,
    placed: Vec<Placed>,
    overlay: Option<AnyElement>,
    shown: f32,
}

#[derive(Clone)]
pub(super) enum Shot {
    App(PathBuf),
    File(PathBuf),
    Other(Option<String>),
}

#[derive(Clone)]
pub(super) struct Shown {
    key: String,
    name: String,
    shot: Shot,
}

pub(super) struct Frame {
    rows: Vec<Shown>,
    offset: usize,
    query: String,
    mode: SearchMode,
}

struct Plan<'a> {
    index: usize,
    shown: Shown,
    entry: Option<&'a crate::discovery::AppEntry>,
    chosen: bool,
    strength: f32,
    height: f32,
    top: f32,
}

struct Look<'a> {
    index: Option<usize>,
    chosen: bool,
    lit: f32,
    cue: f32,
    strength: f32,
    feedback: Option<&'a Feedback>,
}

impl LauncherView {
    #[allow(clippy::too_many_arguments)]
    fn results_view(
        &self,
        rows: Vec<AnyElement>,
        overlay: Option<AnyElement>,
        panel: Option<Div>,
        shown: f32,
        results_height: f32,
        files_mode: bool,
        scroll_offset: usize,
        visible: usize,
        result_count: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let kit = qol_gpui::kit::kit();
        let cue = kit.scroll_cue(
            qol_gpui::scrollbar::ScrollSource::Window {
                first: scroll_offset,
                shown: visible,
                total: result_count,
            },
            kit.grounds.pane,
        );
        let (wheel_row, beside) = if files_mode {
            (FILE_CARD_MAX, PANEL_GAP)
        } else {
            (APP_CARD_MIN, APP_GAP)
        };
        div()
            .id("launcher-results")
            .h(px(results_height))
            .w_full()
            .overflow_hidden()
            .relative()
            .bg(view::bg_color())
            .on_scroll_wheel(cx.listener(
                move |this: &mut Self,
                      event: &ScrollWheelEvent,
                      _window,
                      cx: &mut Context<Self>| {
                    let rows = qol_gpui::scroll_list::wheel_rows(&event.delta, wheel_row);
                    this.state.scroll_list.wheel_by(rows, result_count);
                    cx.notify();
                },
            ))
            .flex()
            .gap(px(beside))
            .px(px(FILE_INSET))
            .py(px(LIST_PAD_Y))
            .when(shown < 1.0, |results| results.opacity(shown))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .relative()
                    .children(overlay)
                    .children(rows)
                    .child(cue),
            )
            .children(panel)
    }

    fn build_visible_rows(
        &mut self,
        scroll_offset: usize,
        visible: usize,
        now: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> VisibleRows {
        let selected = self.state.scroll_list.selected;
        let query = self.state.query.text().to_owned();
        let typed = !query.trim().is_empty();
        let mode = self.state.mode;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let gap = list_gap(mode);
        let previous = self.frame.take();
        let switched = previous.as_ref().is_some_and(|frame| frame.mode != mode);
        let scrolled = previous
            .as_ref()
            .filter(|frame| {
                frame.mode == mode && frame.query == query && frame.offset != scroll_offset
            })
            .map(|frame| frame.offset > scroll_offset);
        if switched {
            self.transitions.clear();
            self.leaving.clear();
        }
        let arriving = previous.is_some() && !switched;
        let feedback = self.state.feedback.clone();
        let feedback = feedback.as_ref();
        let slide = if feedback.is_some_and(|feedback| {
            matches!(feedback.cue, Cue::Moved { .. })
                && feedback.started.elapsed() < feedback::BADGE_HOLD
        }) {
            Motion::TRAVEL
        } else {
            Motion::SETTLE
        };

        let results = self.store.results();
        let top_use = results
            .first()
            .map_or(0, |result| result.frecency_bonus + result.manual_boost);
        let mut out = VisibleRows {
            rows: Vec::with_capacity(visible),
            heights: Vec::with_capacity(visible),
            shown: 1.0,
            ..VisibleRows::default()
        };
        let mut plans = Vec::with_capacity(visible);
        let mut end = 0.0;
        for (index, scored) in results.iter().enumerate().skip(scroll_offset).take(visible) {
            let name = self.store.name(scored);
            let strength = |fallback: f32| {
                if typed {
                    sizing::match_strength(name, &query)
                } else {
                    fallback
                }
            };
            let (shot, entry, strength, height) = match self.store.item(scored) {
                Some(ResultItem::File(entry)) => {
                    if self.details.file(&entry.path).is_none() {
                        out.wants.push(Want::File(entry.path.clone()));
                    }
                    let strength = strength(sizing::rank_strength(index));
                    (
                        Shot::File(entry.path.clone()),
                        None,
                        strength,
                        sizing::file_card(strength).height,
                    )
                }
                item => {
                    let strength = strength(sizing::usage_strength(
                        scored.frecency_bonus + scored.manual_boost,
                        top_use,
                    ));
                    let (shot, entry) = match item {
                        Some(ResultItem::App(entry)) => {
                            if self.details.app(&entry.path).is_none() {
                                out.wants.push(Want::App(entry.clone()));
                            }
                            (Shot::App(entry.path.clone()), Some(entry))
                        }
                        Some(ResultItem::Flow(entry)) => {
                            (Shot::Other(Some(entry.prompt.clone())), None)
                        }
                        _ => (Shot::Other(None), None),
                    };
                    (shot, entry, strength, sizing::app_card(strength).height)
                }
            };
            let key = match &shot {
                Shot::App(path) => format!("app:{}", path.display()),
                Shot::File(path) => format!("file:{}", path.display()),
                Shot::Other(_) => format!("row:{name}"),
            };
            plans.push(Plan {
                index,
                shown: Shown {
                    key,
                    name: name.to_owned(),
                    shot,
                },
                entry,
                chosen: index == selected,
                strength,
                height,
                top: end,
            });
            end += height + gap;
        }

        if let Some(previous) = previous.as_ref().filter(|_| scrolled.is_none() && arriving) {
            for row in &previous.rows {
                let live = plans.iter().any(|plan| plan.shown.key == row.key);
                if !live && !self.leaving.iter().any(|gone| gone.key == row.key) {
                    self.leaving.push(row.clone());
                }
            }
        }
        self.leaving
            .retain(|row| !plans.iter().any(|plan| plan.shown.key == row.key));

        let mut leaving = Vec::with_capacity(self.leaving.len());
        let mut faded = Vec::new();
        for row in &self.leaving {
            let key = row.key.as_str();
            let top = self
                .transitions
                .value(key, Prop::Top, end, Motion::SETTLE, now);
            let shown = self
                .transitions
                .value(key, Prop::Shown, 0.0, Motion::QUICK, now);
            let lit = self
                .transitions
                .value(key, Prop::Lit, 0.0, Motion::QUICK, now);
            let strength = self
                .transitions
                .current(key, Prop::Strength, now)
                .unwrap_or(0.0);
            let cue = self
                .transitions
                .value(key, Prop::Cue, 0.0, Motion::QUICK, now);
            if shown <= 0.0 {
                faded.push(row.key.clone());
                continue;
            }
            let (element, height) = self.card(
                row,
                Look {
                    index: None,
                    chosen: false,
                    lit,
                    cue,
                    strength,
                    feedback: None,
                },
                home.as_deref(),
                cx,
            );
            leaving.push(placed_row(element, (top, height, shown), &row.name, None));
        }
        self.leaving.retain(|row| !faded.contains(&row.key));

        let in_results = self.state.list_focused;
        for plan in &plans {
            let key = plan.shown.key.as_str();
            let name = plan.shown.name.as_str();
            let strength =
                self.transitions
                    .value(key, Prop::Strength, plan.strength, Motion::SETTLE, now);
            let lit = self.transitions.value(
                key,
                Prop::Lit,
                if plan.chosen && in_results { 1.0 } else { 0.0 },
                Motion::QUICK,
                now,
            );
            let cue = self.transitions.value(
                key,
                Prop::Cue,
                if plan.chosen && !in_results { 1.0 } else { 0.0 },
                Motion::QUICK,
                now,
            );
            let (top, shown) = if arriving && !self.transitions.knows(key, Prop::Top) {
                let from = match scrolled {
                    Some(true) => -(plan.height + gap),
                    _ => end,
                };
                (
                    self.transitions
                        .enter(key, Prop::Top, (from, plan.top), slide, now),
                    self.transitions
                        .enter(key, Prop::Shown, (0.0, 1.0), Motion::QUICK, now),
                )
            } else {
                (
                    self.transitions.value(key, Prop::Top, plan.top, slide, now),
                    self.transitions
                        .value(key, Prop::Shown, 1.0, Motion::QUICK, now),
                )
            };
            if plan.chosen {
                let tint = self.transitions.value(
                    PANEL_KEY,
                    Prop::Lit,
                    if in_results { 1.0 } else { 0.0 },
                    Motion::QUICK,
                    now,
                );
                let fill = view::panel_fill(tint);
                let (panel, height) = self.panel(plan, fill, feedback, home.as_deref(), window, cx);
                out.panel = Some(panel);
                out.panel_height = height;
            }
            out.heights.push(plan.height);
            out.placed.push(Placed {
                name: name.to_owned(),
                top: plan.top,
                height: plan.height,
            });
            let (element, height) = self.card(
                &plan.shown,
                Look {
                    index: Some(plan.index),
                    chosen: plan.chosen && in_results,
                    lit,
                    cue,
                    strength,
                    feedback,
                },
                home.as_deref(),
                cx,
            );
            out.rows
                .push(placed_row(element, (top, height, shown), name, feedback));
        }

        out.panel = out.panel.take().map(|panel| {
            let opacity = if !self.transitions.knows(PANEL_KEY, Prop::Shown) && previous.is_some() {
                self.transitions
                    .enter(PANEL_KEY, Prop::Shown, (0.0, 1.0), Motion::QUICK, now)
            } else {
                self.transitions
                    .value(PANEL_KEY, Prop::Shown, 1.0, Motion::QUICK, now)
            };
            panel.opacity(opacity)
        });
        out.shown = if switched {
            self.transitions
                .enter(LIST_KEY, Prop::Shown, (0.0, 1.0), Motion::QUICK, now)
        } else {
            self.transitions
                .value(LIST_KEY, Prop::Shown, 1.0, Motion::QUICK, now)
        };

        let has_panel = out.panel.is_some();
        let keep: Vec<&str> = plans
            .iter()
            .map(|plan| plan.shown.key.as_str())
            .chain(self.leaving.iter().map(|row| row.key.as_str()))
            .collect();
        self.transitions.retain(|key| {
            keep.contains(&key)
                || key == WINDOW_KEY
                || key == LIST_KEY
                || (key == PANEL_KEY && has_panel)
        });
        leaving.append(&mut out.rows);
        out.rows = leaving;
        out.overlay = feedback.and_then(|feedback| ghost(feedback, &out.placed));
        self.frame = Some(Frame {
            rows: plans.into_iter().map(|plan| plan.shown).collect(),
            offset: scroll_offset,
            query,
            mode,
        });
        out
    }

    fn card(
        &self,
        shown: &Shown,
        look: Look<'_>,
        home: Option<&std::path::Path>,
        cx: &mut Context<Self>,
    ) -> (AnyElement, f32) {
        match &shown.shot {
            Shot::File(path) => {
                let size = sizing::file_card(look.strength);
                let card = files::file_card(
                    FileCard {
                        index: look.index,
                        name: &shown.name,
                        path,
                        details: self.details.file(path),
                        lit: look.lit,
                        cue: look.cue,
                        size,
                        feedback: look.feedback,
                    },
                    home,
                    cx,
                );
                (card.into_any_element(), size.height)
            }
            shot => {
                let size = sizing::app_card(look.strength);
                let details = match shot {
                    Shot::App(path) => self.details.app(path),
                    _ => None,
                };
                let summary = match shot {
                    Shot::Other(summary) => summary.as_deref(),
                    _ => details.and_then(|details| details.description.as_deref()),
                };
                let card = view::app_card(view::AppCard {
                    name: &shown.name,
                    summary,
                    art: details.and_then(|details| details.icon.as_deref()),
                    chosen: look.chosen,
                    lit: look.lit,
                    cue: look.cue,
                    size,
                    feedback: look.feedback,
                });
                (card.into_any_element(), size.height)
            }
        }
    }

    fn panel(
        &self,
        plan: &Plan<'_>,
        fill: u32,
        feedback: Option<&Feedback>,
        home: Option<&std::path::Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Div, f32) {
        let name = plan.shown.name.as_str();
        match &plan.shown.shot {
            Shot::File(path) => (
                files::panel(Panel {
                    name,
                    icon: self
                        .details
                        .file(path)
                        .and_then(|details| details.icon.as_deref()),
                    choice: self.state.panel,
                    done: copied_item(feedback, name),
                    active: self.state.list_focused,
                    fill,
                }),
                FILES_PANEL_MIN,
            ),
            _ => {
                let details = plan.entry.and_then(|entry| self.details.app(&entry.path));
                let panel = about::panel(
                    About {
                        name,
                        icon: details.and_then(|details| details.icon.as_deref()),
                        about: details.map(|details| &details.about),
                        home,
                        copied: copied_item(feedback, name) == Some(PanelItem::CopyPath),
                        fill,
                    },
                    window,
                    cx,
                );
                (panel.element, panel.height)
            }
        }
    }
}

fn copied_item(feedback: Option<&Feedback>, name: &str) -> Option<PanelItem> {
    match feedback.filter(|feedback| feedback.is_about(name))?.cue {
        Cue::Copied {
            what: Copied::Path, ..
        } => Some(PanelItem::CopyPath),
        Cue::Copied {
            what: Copied::Name, ..
        } => Some(PanelItem::CopyName),
        _ => None,
    }
}

fn placed_row(
    card: AnyElement,
    (top, height, shown): (f32, f32, f32),
    name: &str,
    feedback: Option<&Feedback>,
) -> AnyElement {
    let row = div()
        .absolute()
        .top(px(top))
        .left_0()
        .right_0()
        .h(px(height))
        .when(shown < 1.0, |row| row.opacity(shown))
        .child(card);
    match feedback.filter(|feedback| feedback.cue.nudges() && feedback.is_about(name)) {
        Some(feedback) => row
            .with_animation(
                feedback.id("launcher-nudge"),
                feedback::nudge_animation(),
                |row, delta| {
                    let nudge = feedback::nudge(delta);
                    row.left(px(nudge)).right(px(-nudge))
                },
            )
            .into_any_element(),
        None => row.into_any_element(),
    }
}

fn ghost(feedback: &Feedback, placed: &[Placed]) -> Option<AnyElement> {
    let Cue::Moved { name, .. } = &feedback.cue else {
        return None;
    };
    let was = feedback.before.iter().find(|placed| &placed.name == name)?;
    let now = placed.iter().find(|placed| &placed.name == name)?;
    if (was.top - now.top).abs() <= 0.5 {
        return None;
    }
    let kit = qol_gpui::kit::kit();
    let edge = qol_color::mix_rgb(kit.palette.accent, kit.grounds.pane.ink, GHOST_INK_MIX);
    Some(
        div()
            .absolute()
            .top(px(was.top))
            .left(px(-GHOST_OUTSET))
            .right_0()
            .h(px(was.height))
            .rounded(px(qol_gpui::theme::RADIUS_CONTROL))
            .border(px(GHOST_EDGE))
            .border_color(rgba(
                qol_gpui::theme::css_rgba_milli(edge, GHOST_EDGE_ALPHA).packed(),
            ))
            .bg(rgba(
                qol_gpui::theme::css_rgba_milli(kit.grounds.band.bg, GHOST_FILL_ALPHA).packed(),
            ))
            .with_animation(
                feedback.id("launcher-ghost"),
                feedback::ghost_animation(),
                |ghost, delta| ghost.opacity(feedback::ghost(delta)),
            )
            .into_any_element(),
    )
}
