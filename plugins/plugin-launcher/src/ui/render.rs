#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::*;

use super::layout::{resize_for_visible_rows, MAX_VISIBLE, ROW_HEIGHT};
use super::state::{EdgeHit, NavDirection};
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
            self.dismiss_requested = false;
            self.set_showing(false);
            qol_gpui::ghost::dismiss_to_ghost(super::LAUNCHER_WINDOW_TITLE, &self.window_title);
        }

        if self.dismiss_sub.is_none() {
            self.dismiss_sub = Some(qol_gpui::ghost::track_dismiss(
                &self.focus_handle,
                window,
                |this: &Self| this.blur_guard_until,
                |this: &Self| this.is_showing,
                cx,
                |this, _window, _cx| {
                    this.set_showing(false);
                    qol_gpui::ghost::dismiss_to_ghost(
                        super::LAUNCHER_WINDOW_TITLE,
                        &this.window_title,
                    );
                },
            ));
            if !self.is_showing {
                #[cfg(target_os = "linux")]
                qol_gpui::popup_window::hide_window_invisible(&self.window_title);
                #[cfg(not(target_os = "linux"))]
                qol_gpui::popup_window::hide_window_by_title(&self.window_title);
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

        if self.is_showing {
            self.sync_entries_from_shared();
            if !self.entry_watch_running {
                self.start_entry_watch(cx);
            }
        }

        #[cfg(debug_assertions)]
        let t0 = std::time::Instant::now();
        self.store
            .ensure_filtered(&self.state.query, self.state.mode, self.state.fuzziness);
        #[cfg(debug_assertions)]
        let filter_us = t0.elapsed().as_micros();

        let result_count = self.store.result_count();
        self.state.sync_result_window(result_count);
        let visible = result_count.min(MAX_VISIBLE);
        let nav_cues = self.state.nav_cues();
        self.apply_focus_gravity_if_idle(result_count, visible, nav_cues.decayed_momentum, cx);
        let scroll_offset = self.state.scroll_offset;
        let hidden_above = scroll_offset;
        let hidden_below = result_count.saturating_sub(scroll_offset + visible);
        if nav_cues.decayed_momentum > 0 {
            self.ensure_trail_decay_tick(cx);
        }
        let previous_selected = nav_cues.previous_selected;
        let edge_hit = self.state.take_edge_hit();
        let momentum_signed = nav_cues.momentum_signed;
        let trail_len = nav_cues.trail_len;
        let trail_direction = nav_cues.trail_direction;
        let (min_score, max_score) = self
            .store
            .results()
            .iter()
            .map(|scored| scored.m.score)
            .fold((i32::MAX, i32::MIN), |(min, max), score| {
                (min.min(score), max.max(score))
            });
        let results_height = visible as f32 * ROW_HEIGHT;
        resize_for_visible_rows(&mut self.state.window_height, visible, window);

        #[cfg(debug_assertions)]
        let t1 = std::time::Instant::now();
        let rows = self.build_visible_rows(
            scroll_offset,
            visible,
            hidden_above,
            hidden_below,
            previous_selected,
            edge_hit,
            trail_len,
            trail_direction,
            min_score,
            max_score,
        );
        #[cfg(debug_assertions)]
        {
            let rows_us = t1.elapsed().as_micros();
            let total_us = render_start.elapsed().as_micros();
            let n = RENDER_COUNT.fetch_add(1, Ordering::Relaxed);
            if n.is_multiple_of(10) {
                eprintln!(
                    "[render #{n}] total={total_us}us filter={filter_us}us rows={rows_us}us gap={gap_us}us visible={visible} results={result_count} q={:?}",
                    &self.state.query
                );
            }
        }
        div()
            .id("launcher")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(view::bg_color())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.is_showing {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" | "esc" => {
                        this.set_showing(false);
                        qol_gpui::ghost::dismiss_to_ghost(
                            super::LAUNCHER_WINDOW_TITLE,
                            &this.window_title,
                        );
                    }
                    _ => this.handle_key(event, window, cx),
                }
            }))
            .child(view::search_bar(
                if self.state.boost_adjusting {
                    "Boost"
                } else {
                    self.state.mode.label()
                },
                self.state.fuzziness.label(),
                &self.state.query,
                self.state.cursor,
                self.state.selected_range(),
                self.state.selected,
                result_count,
                scroll_offset,
                visible,
                momentum_signed,
                hidden_above,
                hidden_below,
            ))
            .child(
                div()
                    .h(px(results_height))
                    .w_full()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .bg(view::bg_color())
                    .children(rows),
            )
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
        let Some(next_offset) = Self::step_toward(self.state.scroll_offset, target) else {
            return;
        };

        self.state.scroll_offset = next_offset;
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

    #[allow(clippy::too_many_arguments)]
    fn build_visible_rows(
        &self,
        scroll_offset: usize,
        visible: usize,
        hidden_above: usize,
        hidden_below: usize,
        previous_selected: Option<usize>,
        edge_hit: Option<EdgeHit>,
        trail_len: usize,
        trail_direction: Option<NavDirection>,
        min_score: i32,
        max_score: i32,
    ) -> Vec<Div> {
        let mut rows = Vec::with_capacity(visible);
        let mut prev_score: Option<i32> = None;
        let selected = self.state.selected;

        for (i, scored) in self
            .store
            .results()
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(MAX_VISIBLE)
        {
            let cluster_break = prev_score
                .map(|prev| scored.m.score.saturating_sub(prev) >= 24)
                .unwrap_or(false);
            prev_score = Some(scored.m.score);

            let is_top_row = i == scroll_offset;
            let is_bottom_row = i + 1 == scroll_offset + visible;
            let hidden_above_for_row = if is_top_row { hidden_above } else { 0 };
            let hidden_below_for_row = if is_bottom_row { hidden_below } else { 0 };
            let trail_depth = Self::trail_depth_for_row(i, selected, trail_len, trail_direction);
            let confidence_pct = Self::confidence_pct(scored.m.score, min_score, max_score);

            rows.push(view::result_row(
                scored,
                self.store.name(scored),
                view::RowWindowCue {
                    selected: i == selected,
                    previous_selected: previous_selected == Some(i),
                    trail_depth,
                    distance_from_selected: i.abs_diff(selected),
                    hidden_above: hidden_above_for_row,
                    hidden_below: hidden_below_for_row,
                    edge_hit_top: is_top_row && matches!(edge_hit, Some(EdgeHit::Top)),
                    edge_hit_bottom: is_bottom_row && matches!(edge_hit, Some(EdgeHit::Bottom)),
                    confidence_pct,
                    cluster_break,
                },
                ROW_HEIGHT,
            ));
        }

        rows
    }

    fn trail_depth_for_row(
        row_index: usize,
        selected: usize,
        trail_len: usize,
        trail_direction: Option<NavDirection>,
    ) -> u8 {
        match trail_direction {
            Some(NavDirection::Down) => {
                let distance = selected.saturating_sub(row_index);
                if distance >= 1 && distance <= trail_len {
                    (trail_len - distance + 1) as u8
                } else {
                    0
                }
            }
            Some(NavDirection::Up) => {
                let distance = row_index.saturating_sub(selected);
                if distance >= 1 && distance <= trail_len {
                    (trail_len - distance + 1) as u8
                } else {
                    0
                }
            }
            None => 0,
        }
    }

    fn confidence_pct(score: i32, min_score: i32, max_score: i32) -> u8 {
        if max_score <= min_score {
            return 100;
        }

        let numerator = (max_score - score) as f32;
        let denominator = (max_score - min_score) as f32;
        ((numerator / denominator) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    }
}
