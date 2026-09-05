use std::time::Duration;

use gpui::{px, size, AppContext as _, AsyncApp, ClipboardItem, Context, KeyDownEvent, WeakEntity};

use super::input::InputEffect;
use super::layout::{full_window_height, window_height_for_detail, WINDOW_WIDTH};
use super::trace;
use super::LauncherView;

const FLOW_DEBOUNCE: Duration = Duration::from_millis(200);
const DETAIL_SCROLL_STEP: f32 = 54.0;

enum ClipboardShortcut {
    Copy,
    Cut,
    Paste,
}

impl LauncherView {
    pub(super) fn handle_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let secondary = event.keystroke.modifiers.secondary();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        let alt = event.keystroke.modifiers.alt;

        if self.handle_clipboard_shortcut(key, secondary, cx) {
            return;
        }
        let flow_active = self.state.flow.is_some();
        if !flow_active {
            self.store
                .ensure_filtered(&self.state.query, self.state.mode, self.state.fuzziness);
        }
        let result_count = if flow_active {
            self.state.flow_result_count()
        } else {
            self.store.result_count()
        };
        let selected_before = self.state.scroll_list.selected;
        let effect = self
            .state
            .apply_key(key, secondary, control, shift, alt, result_count);
        trace::input(
            self,
            key,
            effect,
            result_count,
            selected_before,
            event.is_held,
            event.keystroke.key_char.as_deref(),
        );

        if !matches!(effect, InputEffect::BoostUp | InputEffect::BoostDown) {
            self.state.boost_adjusting = false;
        }

        match effect {
            InputEffect::Ignore => {}
            InputEffect::Navigate => {
                self.state.sync_result_window(result_count);
                cx.notify();
            }
            InputEffect::QueryChanged => {
                self.state.clear_launch_error();
                self.state.reset_results_position();
                self.schedule_query_render(cx);
            }
            InputEffect::FlowQueryChanged => {
                self.state.clear_launch_error();
                self.state.reset_results_position();
                self.schedule_flow_query(cx);
                cx.notify();
            }
            InputEffect::BoostUp | InputEffect::BoostDown => {
                let delta = if matches!(effect, InputEffect::BoostUp) {
                    25
                } else {
                    -25
                };
                self.state.boost_adjusting = true;
                self.adjust_selected_boost(delta);
                self.store.invalidate_cache();
                self.store.ensure_filtered(
                    &self.state.query,
                    self.state.mode,
                    self.state.fuzziness,
                );
                cx.notify();
            }
            InputEffect::Launch => self.launch_selected(window, cx),
            InputEffect::Dismiss => self.hide_to_ghost("key", window),
            InputEffect::FlowExit => {
                self.state.exit_flow();
                trace::flow(self, "exited");
                cx.notify();
            }
            InputEffect::FlowActivate => self.activate_flow_row(window, cx),
            InputEffect::FlowDetail => self.open_flow_detail(window, cx),
            InputEffect::FlowDetailClose => self.close_flow_detail(window, cx),
            InputEffect::FlowDetailScrollUp => self.scroll_flow_detail(-DETAIL_SCROLL_STEP, cx),
            InputEffect::FlowDetailScrollDown => self.scroll_flow_detail(DETAIL_SCROLL_STEP, cx),
            InputEffect::FlowDislike => self.dislike_flow_row(cx),
        }
    }

    fn handle_clipboard_shortcut(
        &mut self,
        key: &str,
        secondary: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(shortcut) = Self::clipboard_shortcut(key, secondary) else {
            return false;
        };
        self.apply_clipboard_shortcut(shortcut, cx);
        true
    }

    fn clipboard_shortcut(key: &str, secondary: bool) -> Option<ClipboardShortcut> {
        if !secondary {
            return None;
        }
        match key {
            "c" => Some(ClipboardShortcut::Copy),
            "x" => Some(ClipboardShortcut::Cut),
            "v" => Some(ClipboardShortcut::Paste),
            _ => None,
        }
    }

    fn apply_clipboard_shortcut(&mut self, shortcut: ClipboardShortcut, cx: &mut Context<Self>) {
        match shortcut {
            ClipboardShortcut::Copy => self.copy_selection(cx),
            ClipboardShortcut::Cut => self.cut_selection(cx),
            ClipboardShortcut::Paste => self.paste_from_clipboard(cx),
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.state.selection_text() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn cut_selection(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.state.cut_selection() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.state.reset_results_position();
        cx.notify();
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if self.state.paste_text(&text) {
            self.state.reset_results_position();
            cx.notify();
        }
    }

    fn adjust_selected_boost(&mut self, delta: i32) {
        let Some(scored) = self.store.get(self.state.scroll_list.selected) else {
            return;
        };
        if !matches!(scored.source, crate::discovery::search::ResultSource::App) {
            return;
        }
        let name = self.store.name(scored).to_string();
        self.store.adjust_boost(&name, delta);
    }

    fn launch_selected(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        #[cfg(debug_assertions)]
        let started = std::time::Instant::now();
        #[cfg(not(debug_assertions))]
        let started = ();
        trace::launch(self, "start", started);
        self.store
            .ensure_filtered(&self.state.query, self.state.mode, self.state.fuzziness);
        let Some(scored) = self.store.get(self.state.scroll_list.selected) else {
            eprintln!(
                "[controller] launch_selected: no scored item at index {}",
                self.state.scroll_list.selected
            );
            return;
        };
        eprintln!(
            "[controller] launch_selected: index={} source={:?} name={:?}",
            self.state.scroll_list.selected,
            scored.source,
            self.store.name(scored)
        );
        let Some(item) = self.store.item(scored) else {
            eprintln!("[controller] launch_selected: failed to resolve item");
            return;
        };
        if let crate::discovery::search::ResultItem::Flow(entry) = item {
            self.state.enter_flow(entry.clone());
            trace::flow(self, "entered");
            cx.notify();
            return;
        }
        let is_app = matches!(scored.source, crate::discovery::search::ResultSource::App);
        let name = self.store.name(scored).to_string();
        eprintln!("[controller] launching item...");
        trace::launch(self, "send", started);
        let launch_result = crate::launch::launch_item(&item);
        trace::launch(self, "sent", started);
        if let Err(error) = launch_result {
            eprintln!("[controller] launch error: {error}");
            self.state.set_launch_error(error.to_string());
            cx.notify();
            return;
        }
        eprintln!("[controller] launch succeeded, hiding window");
        if is_app {
            self.store.record_launch(&name);
        }
        self.hide_to_ghost("launch", window);
    }

    fn schedule_flow_query(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_mut() else {
            return;
        };
        flow.generation += 1;
        flow.pending = true;
        flow.verification_deadline = None;
        let epoch = flow.epoch;
        let generation = flow.generation;
        if self.state.query.trim().is_empty() {
            flow.rows.clear();
            flow.verdict = crate::flow::FlowVerdict::Answered;
            flow.pending = false;
            cx.notify();
            return;
        }
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx.background_executor().timer(FLOW_DEBOUNCE).await;
                this.update(&mut async_cx, |view, cx| {
                    let current = view.state.flow.as_ref().is_some_and(|session| {
                        session.matches_request(epoch, generation) && !session.in_flight
                    });
                    if current && view.is_showing {
                        view.start_flow_fetch(cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn start_flow_fetch(&mut self, cx: &mut Context<Self>) {
        if !self.is_showing {
            return;
        }
        let Some(flow) = self.state.flow.as_mut() else {
            return;
        };
        let text = self.state.query.clone();
        if text.trim().is_empty() {
            return;
        }
        let generation = flow.generation;
        let epoch = flow.epoch;
        let entry = flow.entry.clone();
        flow.in_flight = true;
        trace::flow(self, "queried");
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let outcome = async_cx
                    .background_spawn(async move { crate::flow::fetch_rows(&entry, &text) })
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    let Some(session) = view.state.flow.as_mut() else {
                        return;
                    };
                    if session.epoch != epoch || !view.is_showing {
                        return;
                    }
                    session.in_flight = false;
                    if session.generation != generation {
                        view.start_flow_fetch(cx);
                        return;
                    }
                    let (rows, mut verdict, failure) = match outcome {
                        Ok(fetch) => (fetch.rows, fetch.verdict, None),
                        Err(message) => (
                            Vec::new(),
                            crate::flow::FlowVerdict::Answered,
                            Some(message),
                        ),
                    };
                    if verdict == crate::flow::FlowVerdict::Checking {
                        let deadline = session.verification_deadline.get_or_insert_with(|| {
                            std::time::Instant::now() + std::time::Duration::from_secs(60)
                        });
                        if std::time::Instant::now() >= *deadline {
                            verdict = crate::flow::FlowVerdict::Vague;
                        }
                    }
                    session.rows = rows;
                    session.verdict = verdict;
                    session.pending = false;
                    if let Some(message) = failure {
                        view.state.set_launch_error(message);
                    }
                    trace::flow(view, "rows");
                    cx.notify();
                    if verdict == crate::flow::FlowVerdict::Checking {
                        view.refresh_pending_flow(epoch, generation, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn refresh_pending_flow(&mut self, epoch: u64, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx
                    .background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    let current = view.state.flow.as_ref().is_some_and(|session| {
                        session.matches_request(epoch, generation)
                            && !session.in_flight
                            && session.verdict == crate::flow::FlowVerdict::Checking
                    });
                    if current && view.is_showing {
                        view.start_flow_fetch(cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn dislike_flow_row(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_ref() else {
            return;
        };
        if flow.entry.plugin_id != "qol-memory" {
            return;
        }
        let Some(row) = flow.rows.get(self.state.scroll_list.selected) else {
            return;
        };
        let Some(key) = row.raw.get("key").and_then(|value| value.as_str()) else {
            return;
        };
        let query = self.state.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        let entry = flow.entry.clone();
        let key = key.to_string();
        trace::flow(self, "dislike");
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let outcome = async_cx
                    .background_spawn(
                        async move { crate::flow::send_feedback(&entry, &query, &key) },
                    )
                    .await;
                if let Err(error) = outcome {
                    eprintln!("[controller] flow dislike failed: {error}");
                }
                this.update(&mut async_cx, |view, _| trace::flow(view, "disliked"))
                    .ok();
            }
        })
        .detach();
    }

    fn activate_flow_row(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_ref() else {
            return;
        };
        let Some(row) = flow.rows.get(self.state.scroll_list.selected) else {
            return;
        };
        let entry = &flow.entry;
        if !entry.row_actions.is_empty() {
            if let Err(message) = crate::flow::run_row_action(entry, &entry.row_actions[0], row) {
                self.state.set_launch_error(message);
                cx.notify();
                return;
            }
        } else {
            let text = row.copy.clone().unwrap_or_else(|| row.title.clone());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        trace::flow(self, "activated");
        self.hide_to_ghost("flow", window);
    }

    fn open_flow_detail(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if !self.state.open_flow_detail() {
            return;
        }
        self.detail_scroll.set_offset(gpui::Point::default());
        window.resize(size(px(WINDOW_WIDTH), px(window_height_for_detail())));
        trace::flow(self, "detail_open");
        cx.notify();
    }

    fn scroll_flow_detail(&mut self, delta: f32, cx: &mut Context<Self>) {
        let max = self.detail_scroll.max_offset().height;
        let y = (self.detail_scroll.offset().y - px(delta))
            .max(-max)
            .min(px(0.0));
        self.detail_scroll.set_offset(gpui::point(px(0.0), y));
        cx.notify();
    }

    fn close_flow_detail(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if !self.state.close_flow_detail() {
            return;
        }
        window.resize(size(px(WINDOW_WIDTH), px(full_window_height())));
        trace::flow(self, "detail_close");
        cx.notify();
    }
}
