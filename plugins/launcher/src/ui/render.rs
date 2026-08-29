use gpui::prelude::FluentBuilder;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::*;

use super::layout::{
    window_height_for, window_height_for_detail, window_height_for_rows, window_height_for_trail,
    FLOW_ROW_HEIGHT, HEADER_HEIGHT, MAX_VISIBLE, ROW_HEIGHT, WINDOW_WIDTH,
};
#[cfg(debug_assertions)]
use super::trace;
use super::view;
use super::LauncherView;

#[cfg(debug_assertions)]
static LAST_RENDER_US: AtomicU64 = AtomicU64::new(0);
#[cfg(debug_assertions)]
static RENDER_COUNT: AtomicU64 = AtomicU64::new(0);

impl Focusable for LauncherView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.dismiss_requested {
            let from = self.dismiss_requested_from;
            self.dismiss_requested = false;
            self.dismiss_requested_from = "requested";
            self.hide_to_ghost(from, window);
        }

        if self.dismiss_sub.is_none() {
            self.dismiss_sub = Some(qol_gpui::ghost::track_dismiss(
                "launcher",
                &self.focus_handle,
                window,
                |this: &Self| this.blur_guard_until,
                |this: &Self| this.is_showing,
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
                        target_height: window_height_for_rows(0),
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
                .id("launcher")
                .track_focus(&self.focus_handle)
                .w(px(WINDOW_WIDTH))
                .h(px(window_height_for_rows(0)))
                .overflow_hidden()
                .rounded(px(qol_gpui::theme::RADIUS_WINDOW))
                .bg(view::bg_color());
        }

        if self.is_showing {
            self.ensure_click_away_monitor(cx);
            self.sync_entries_from_shared();
            if !self.entry_watch_running {
                self.start_entry_watch(cx);
            }
        }

        #[cfg(debug_assertions)]
        let t0 = std::time::Instant::now();
        let flow_active = self.state.flow.is_some();
        if !flow_active {
            self.store
                .ensure_filtered(&self.state.query, self.state.mode, self.state.fuzziness);
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
        self.apply_focus_gravity_if_idle(result_count, visible, nav_cues.decayed_momentum, cx);
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
        let target_height = if detail_ready {
            window_height_for_detail()
        } else if flow_active {
            if result_count > 0 {
                window_height_for_trail()
            } else {
                window_height_for(0, FLOW_ROW_HEIGHT)
            }
        } else {
            window_height_for_rows(visible)
        };
        let results_height = target_height - HEADER_HEIGHT - qol_gpui::theme::HEIGHT_HINT_BAR;

        #[cfg(debug_assertions)]
        let t1 = std::time::Instant::now();
        #[cfg(debug_assertions)]
        let selected_name = self
            .store
            .get(self.state.scroll_list.selected)
            .map(|scored| self.store.name(scored))
            .unwrap_or("")
            .to_string();
        let rows = if flow_active {
            Vec::new()
        } else {
            self.build_visible_rows(scroll_offset, visible)
        };
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
                    self.state.query
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
        div()
            .id("launcher")
            .track_focus(&self.focus_handle)
            .w(px(WINDOW_WIDTH))
            .h(px(target_height))
            .overflow_hidden()
            .rounded(px(qol_gpui::theme::RADIUS_WINDOW))
            .shadow(qol_gpui::kit::float_shadow(view::palette().text_selected))
            .flex()
            .flex_col()
            .bg(view::bg_color())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.is_showing {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" | "esc" if this.state.flow.is_some() => {
                        this.handle_key(event, window, cx);
                    }
                    "escape" | "esc" => {
                        this.hide_to_ghost("key", window);
                    }
                    _ => this.handle_key(event, window, cx),
                }
            }))
            .child(view::search_bar(
                &self.state.query,
                self.state.launch_error.as_deref(),
                self.state.cursor,
                self.state.selected_range(),
                self.state.scroll_list.selected,
                result_count,
                flow_pending,
                flow_prompt.as_deref().unwrap_or("Type to search\u{2026}"),
            ))
            .when(result_count > 0, |root| {
                if flow_active {
                    if detail_ready {
                        match self
                            .state
                            .flow
                            .as_ref()
                            .and_then(|session| session.rows.get(self.state.scroll_list.selected))
                        {
                            Some(row) => root.child(
                                div()
                                    .id("launcher-results")
                                    .h(px(results_height))
                                    .w_full()
                                    .overflow_hidden()
                                    .bg(view::bg_color())
                                    .child(view::detail_body(&kit, row, results_height)),
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
                                    .child(view::trail_body(&kit, &session.rows, focus)),
                            ),
                            None => root,
                        }
                    }
                } else {
                    root.child(
                        div()
                            .id("launcher-results")
                            .h(px(results_height))
                            .w_full()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .py(px(super::layout::LIST_PAD_Y))
                            .gap(px(super::layout::ROW_GAP))
                            .bg(view::bg_color())
                            .on_scroll_wheel(cx.listener(
                                move |this: &mut Self,
                                      event: &ScrollWheelEvent,
                                      _window,
                                      cx: &mut Context<Self>| {
                                    let rows =
                                        qol_gpui::scroll_list::wheel_rows(&event.delta, ROW_HEIGHT);
                                    this.state.scroll_list.wheel_by(rows, result_count);
                                    cx.notify();
                                },
                            ))
                            .children(rows),
                    )
                }
            })
            .child(if detail_ready {
                view::hint_bar_detail()
            } else {
                match flow_entry.as_ref() {
                    Some(entry) => view::hint_bar_flow(entry),
                    None => view::hint_bar(self.state.mode),
                }
            })
    }
}

impl LauncherView {
    fn apply_focus_gravity_if_idle(
        &mut self,
        result_count: usize,
        visible: usize,
        decayed_momentum: u8,
        cx: &mut Context<Self>,
    ) {
        if decayed_momentum != 0 {
            return;
        }
        if !self.state.should_focus_gravity() {
            return;
        }

        let target = self.state.focus_gravity_target(result_count, visible);
        let Some(next_offset) = Self::step_toward(self.state.scroll_list.scroll_offset, target)
        else {
            return;
        };

        self.state.scroll_list.scroll_offset = next_offset;
        cx.notify();
    }

    fn step_toward(current: usize, target: usize) -> Option<usize> {
        if current < target {
            return Some(current + 1);
        }
        if current > target {
            return Some(current - 1);
        }
        None
    }

    fn build_visible_rows(&self, scroll_offset: usize, visible: usize) -> Vec<Div> {
        let mut rows = Vec::with_capacity(visible);
        let selected = self.state.scroll_list.selected;
        for (i, scored) in self
            .store
            .results()
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(MAX_VISIBLE)
        {
            rows.push(view::result_row(
                scored,
                self.store.name(scored),
                i == selected,
                ROW_HEIGHT,
            ));
        }
        rows
    }
}
