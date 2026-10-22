pub mod collapse;
mod fold_memory;
pub mod nav;
pub mod notify;
pub mod placement;
pub mod render;
pub mod run;
pub mod selection;
mod trace;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gpui::{
    px, size, App, AppContext, AsyncApp, Bounds, Context, FocusHandle, Focusable, Pixels, Point,
    Size, Subscription, WeakEntity, Window,
};
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{DragGestureState, SurfaceDismisser};
use qol_terminal_sessions::{SessionBinding, SessionId};

use crate::host::TerminalHost;
use crate::session::registry::Registry;
use crate::ui::placement::Corner;
use crate::ui::selection::Selection;

pub(crate) const WINDOW_TITLE: &str = "cli-sessions-panel";

static CYCLE_FOCUS_GEN: AtomicU64 = AtomicU64::new(0);

/// How long a terminal takes to report the focus the panel just gave up.
const FOCUS_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);
const REVEAL_POLL: std::time::Duration = std::time::Duration::from_millis(50);
const REVEAL_POLLS: usize = 60;

pub struct SessionsView {
    pub registry: Arc<Mutex<Registry>>,
    pub host: Arc<dyn TerminalHost + Send + Sync>,
    title: String,
    dismisser: SurfaceDismisser,
    selection: Selection,
    is_showing: bool,
    collapse_state: collapse::CollapseState,
    drag_gesture: std::rc::Rc<std::cell::RefCell<DragGestureState>>,
    edge_press: bool,
    key_hold: KeyHold,
    nav_guard: qol_gpui::phantom_nav::PhantomNavGuard,
    list_scroll: qol_gpui::scroll_list::SelectionScroll,
    last_jumped: Option<SessionId>,
    pub focus_handle: FocusHandle,
    _activation: Subscription,
    _bounds: Subscription,
}

impl SessionsView {
    pub fn new(
        registry: Arc<Mutex<Registry>>,
        host: Arc<dyn TerminalHost + Send + Sync>,
        corner: Corner,
        unfolded: Option<Bounds<Pixels>>,
        dismisser: SurfaceDismisser,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = dismisser.current_title();
        let activation = cx.observe_window_activation(window, |view: &mut Self, window, cx| {
            if !window.is_window_active() {
                view.read_focus_after_leaving(cx);
            }
        });
        let bounds = cx.observe_window_bounds(window, |view: &mut Self, window, _| {
            view.hold_folded_height(window);
        });
        dismisser.set_min_size(collapse::unfolded_min_size());
        if let Some(unfolded) = unfolded {
            restore_unfolded_after_reveal(window, cx, unfolded);
        }
        Self {
            registry,
            host,
            title,
            dismisser,
            selection: Selection::default(),
            list_scroll: qol_gpui::scroll_list::SelectionScroll::new(),
            is_showing: true,
            collapse_state: collapse::CollapseState::new(corner),
            drag_gesture: std::rc::Rc::new(std::cell::RefCell::new(DragGestureState::new(4.0))),
            edge_press: false,
            key_hold: KeyHold::default(),
            nav_guard: qol_gpui::phantom_nav::PhantomNavGuard::new(),
            last_jumped: None,
            focus_handle: cx.focus_handle(),
            _activation: activation,
            _bounds: bounds,
        }
    }

    fn read_focus_after_leaving(&self, cx: &mut Context<Self>) {
        let Ok(read) = self.registry.lock().map(|mut registry| {
            registry.focus_moved();
            registry.focus_read()
        }) else {
            return;
        };
        let registry = self.registry.clone();
        let host = self.host.clone();
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(FOCUS_SETTLE).await;
            cx.background_spawn(async move {
                let panes = host.discover();
                if let Ok(mut registry) = registry.lock() {
                    registry.observe_focus(read, &panes);
                }
            })
            .await;
            let _ = view.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    pub fn rows(&self) -> Vec<crate::session::registry::SessionState> {
        self.registry.lock().map(|r| r.sorted()).unwrap_or_default()
    }

    pub fn is_showing(&self) -> bool {
        self.is_showing
    }

    pub fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapse_state.is_collapsed()
    }

    pub fn collapse_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let expanded = qol_gpui::popup_window::window_bounds_primary_anchored(window);
        if self.edge_press {
            trace::fold("refused-while-resizing", expanded);
            return false;
        }
        let strip = self.collapse_state.collapse(expanded);
        fold_memory::save(expanded);
        self.dismisser.set_min_size(collapse::folded_min_size());
        trace::collapse(true);
        schedule_panel_bounds(window, cx, strip, |view, _| {
            view.collapse_state.expand();
            view.dismisser.set_min_size(collapse::unfolded_min_size());
            fold_memory::clear();
            trace::collapse(false);
        });
        self.key_hold.clear();
        window.focus(&self.focus_handle);
        true
    }

    pub fn expand_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let strip = qol_gpui::popup_window::window_bounds_primary_anchored(window);
        let Some(unfolded) = self.collapse_state.unfolded(strip) else {
            return false;
        };
        if self.edge_press {
            trace::fold("refused-while-resizing", strip);
            return true;
        }
        self.collapse_state.expand();
        let anchored = clamp_to_monitor(unfolded, strip, cx);
        self.dismisser.set_min_size(collapse::unfolded_min_size());
        fold_memory::clear();
        schedule_panel_bounds(window, cx, anchored, |view, bounds| {
            view.collapse_state.collapse(bounds);
            view.dismisser.set_min_size(collapse::folded_min_size());
            fold_memory::save(bounds);
        });
        trace::collapse(false);
        self.key_hold.clear();
        window.focus(&self.focus_handle);
        true
    }

    pub fn dismiss_with_reason(
        &mut self,
        reason: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let strip = qol_gpui::popup_window::window_bounds_primary_anchored(window);
        if let Some(unfolded) = self.collapse_state.unfolded(strip) {
            fold_memory::save(unfolded);
        }
        let _scope = qol_gpui::popup_window::reason_scope(reason);
        let showing = self.is_showing;
        self.is_showing = false;
        self.dismisser.dismiss(cx);
        trace::dismiss(reason, showing);
        showing
    }

    fn hold_folded_height(&mut self, window: &mut Window) {
        let viewport = window.viewport_size();
        if !self.is_collapsed() || !collapse::folded_height_drifted(viewport.height) {
            return;
        }
        trace::fold(
            "held-height",
            qol_gpui::popup_window::window_bounds_primary_anchored(window),
        );
        self.dismisser
            .resize_window(size(viewport.width, px(collapse::STRIP_HEIGHT)), window);
    }

    fn blocks_press(&mut self, position: Point<Pixels>, viewport: Size<Pixels>) -> bool {
        if self.is_collapsed() && collapse::presses_height_edge(position, viewport) {
            self.edge_press = false;
            return true;
        }
        self.edge_press = collapse::presses_edge(position, viewport);
        false
    }

    fn end_press(&mut self) {
        self.edge_press = false;
    }

    fn restore_unfolded(
        &mut self,
        unfolded: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_collapsed() {
            return;
        }
        fold_memory::clear();
        let current = qol_gpui::popup_window::window_bounds_primary_anchored(window);
        let target = clamp_to_monitor(unfolded, current, cx);
        if !collapse::bounds_differ(current, target) {
            return;
        }
        trace::fold("restored", target);
        schedule_panel_bounds(window, cx, target, |_, _| {});
    }

    fn key_repeat_guard(&mut self, key: &str) -> bool {
        self.key_hold.press(key)
    }

    fn key_released(&mut self, key: &str) {
        self.key_hold.release(key);
    }

    fn order(&self) -> Vec<SessionId> {
        self.rows().iter().map(|row| row.id.clone()).collect()
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn jump_to_session(&mut self, id: SessionId, reason: &'static str, cx: &mut Context<Self>) {
        self.selection.select(id.clone());
        self.last_jumped = Some(id.clone());
        let rows = self.rows();
        let binding = match rows.iter().enumerate().find(|(_, row)| row.id == id) {
            Some((index, row)) => {
                trace::jump_target(reason, index, rows.len(), row);
                if let Some(park) = row.park.clone() {
                    open_park_async(park, cx);
                    return;
                }
                let Some(binding) = row.binding() else {
                    trace::jump_missing(reason, index, rows.len());
                    return;
                };
                binding
            }
            None => {
                trace::jump_missing(reason, rows.len(), rows.len());
                return;
            }
        };
        self.focus_session_async(binding, reason, cx);
    }

    pub fn move_selection_down(&mut self) {
        if self.swallow_nav(1.0) {
            return;
        }
        let order = self.order();
        self.selection.move_down(&order);
    }

    pub fn move_selection_up(&mut self) {
        if self.swallow_nav(-1.0) {
            return;
        }
        let order = self.order();
        self.selection.move_up(&order);
    }

    fn swallow_nav(&mut self, direction: f64) -> bool {
        self.nav_guard
            .swallow(qol_gpui::phantom_nav::NavAxis::Vertical, direction)
    }

    pub fn focus_selected(&mut self, cx: &mut Context<Self>) {
        let order = self.order();
        if let Some(id) = self.selection.resolved(&order) {
            self.jump_to_session(id, "enter", cx);
        }
    }

    fn focus_session_async(
        &self,
        target: SessionBinding,
        reason: &'static str,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let title = self.title.clone();
        trace::focus_start(reason, target.session_id());
        let result_id = target.session_id().clone();
        let retain_panel_focus = reason == "cycle-implementer";
        cx.spawn(move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move {
                        let result = host.focus(&target);
                        if retain_panel_focus {
                            let commit = CYCLE_FOCUS_GEN.fetch_add(1, Ordering::SeqCst) + 1;
                            qol_gpui::popup_window::reassert_focus_until_held(
                                &title,
                                &CYCLE_FOCUS_GEN,
                                commit,
                            );
                        }
                        result
                    })
                    .await;
                trace::focus_result(reason, &result_id, &result);
            }
        })
        .detach();
    }

    pub fn acknowledge(&mut self, id: &SessionId) {
        if let Ok(mut reg) = self.registry.lock() {
            if let Some(s) = reg.get_mut(id) {
                let previous = s.status;
                s.acknowledge();
                if previous != s.status {
                    self.selection.select(id.clone());
                    qol_runtime::probe!(
                        "CLI_SESSIONS_ACK",
                        "id={id} previous={previous:?} status={:?}",
                        s.status
                    );
                }
            }
        }
    }

    pub fn acknowledge_selected(&mut self) {
        let order = self.order();
        if let Some(id) = self.selection.resolved(&order) {
            self.acknowledge(&id);
        }
    }

    pub fn wake_selected(&mut self) {
        let order = self.order();
        let Some(id) = self.selection.resolved(&order) else {
            return;
        };
        let key = self
            .registry
            .lock()
            .ok()
            .and_then(|reg| reg.get(&id).and_then(|s| s.key.clone()));
        if let Some(key) = key {
            crate::daemon::knocks::spawn_wake(&key);
        }
    }

    pub fn cycle_implementers(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        let order = self.order();
        let driver = self
            .selection
            .resolved(&order)
            .filter(|id| {
                rows.iter()
                    .any(|row| &row.id == id && !row.driving.is_empty())
            })
            .or_else(|| {
                rows.iter()
                    .find(|row| !row.driving.is_empty())
                    .map(|row| row.id.clone())
            });
        if let Some(driver) = driver {
            self.cycle_implementers_of(&driver, forward, cx);
        }
    }

    pub fn cycle_implementers_of(
        &mut self,
        driver: &SessionId,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        let driven = self
            .rows()
            .iter()
            .find(|row| &row.id == driver)
            .map(|row| row.driving.clone())
            .unwrap_or_default();
        if driven.is_empty() {
            return;
        }
        let current = self
            .last_jumped
            .as_ref()
            .and_then(|id| driven.iter().position(|d| d == id));
        let index = match (current, forward) {
            (Some(i), true) => (i + 1) % driven.len(),
            (Some(i), false) => (i + driven.len() - 1) % driven.len(),
            (None, true) => 0,
            (None, false) => driven.len() - 1,
        };
        self.jump_to_session(driven[index].clone(), "cycle-implementer", cx);
    }

    pub fn jump_to_next_attention(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows();
        let current = self
            .last_jumped
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.id == id));
        if let Some(id) = crate::ui::nav::next_attention_row(&rows, current)
            .and_then(|index| rows.get(index))
            .map(|row| row.id.clone())
        {
            self.jump_to_session(id, "next-attention", cx);
        }
    }
}

impl Focusable for SessionsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn schedule_panel_bounds(
    window: &mut Window,
    cx: &mut Context<SessionsView>,
    bounds: Bounds<Pixels>,
    on_failure: fn(&mut SessionsView, Bounds<Pixels>),
) {
    cx.spawn_in(window, async move |view, cx| {
        let _ = view.update_in(cx, |view, window, cx| {
            let resized = view.dismisser.resize_window(bounds.size, window);
            let moved = view.dismisser.reposition_window(bounds.origin);
            if !(resized && moved) {
                on_failure(view, bounds);
            }
            cx.notify();
        });
    })
    .detach();
}

fn clamp_to_monitor(
    bounds: Bounds<Pixels>,
    anchor: Bounds<Pixels>,
    cx: &mut Context<SessionsView>,
) -> Bounds<Pixels> {
    let tracker = MonitorTracker::start(cx);
    let monitors: Vec<Bounds<Pixels>> = tracker
        .all_monitors_or_snapshot()
        .iter()
        .map(|monitor| monitor.logical_bounds())
        .collect();
    let monitor = qol_gpui::placement::monitor_at_point(&monitors, anchor.center()).or_else(|| {
        tracker
            .snapshot_monitor()
            .map(|monitor| monitor.logical_bounds())
    });
    match monitor {
        Some(monitor) => qol_gpui::placement::clamp_origin_to_monitor(monitor, bounds, 0.0),
        None => bounds,
    }
}

fn restore_unfolded_after_reveal(
    window: &mut Window,
    cx: &mut Context<SessionsView>,
    unfolded: Bounds<Pixels>,
) {
    cx.spawn_in(window, async move |view, cx| {
        for _ in 0..REVEAL_POLLS {
            cx.background_executor().timer(REVEAL_POLL).await;
            let restored = view.update_in(cx, |view, window, cx| {
                if !view.dismisser.is_visible() {
                    return false;
                }
                view.restore_unfolded(unfolded, window, cx);
                true
            });
            if !matches!(restored, Ok(false)) {
                return;
            }
        }
    })
    .detach();
}

fn open_park_async(park: String, cx: &mut Context<SessionsView>) {
    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=open park={park}");
    cx.background_spawn(async move {
        match crate::park::open(&park) {
            Ok(outcome) => log::info!("[cli-sessions] {outcome}"),
            Err(error) => {
                log::warn!("[cli-sessions] opening park {park} failed: {error:#}");
                qol_plugin_daemon::notification::send_notification(
                    "CLI Sessions",
                    &format!("{error:#}"),
                );
            }
        }
    })
    .detach();
}

use std::collections::HashSet;

#[derive(Default)]
struct KeyHold {
    held: HashSet<String>,
}

impl KeyHold {
    fn press(&mut self, key: &str) -> bool {
        self.held.insert(key.to_owned())
    }

    fn release(&mut self, key: &str) {
        self.held.remove(key);
    }

    fn clear(&mut self) {
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::KeyHold;

    #[test]
    fn key_hold_accepts_fresh_presses_and_denies_repeats_until_release() {
        for key in ["enter", "a"] {
            let mut hold = KeyHold::default();
            assert!(hold.press(key));
            assert!(!hold.press(key));
            assert!(!hold.press(key));
            hold.release(key);
            assert!(hold.press(key));
        }
    }

    #[test]
    fn key_hold_tracks_keys_independently() {
        let mut hold = KeyHold::default();
        assert!(hold.press("enter"));
        assert!(hold.press("space"));
        assert!(!hold.press("enter"));
        assert!(!hold.press("space"));
        hold.release("space");
        assert!(hold.press("space"));
        assert!(!hold.press("enter"));
        hold.release("enter");
        assert!(hold.press("enter"));
    }

    #[test]
    fn key_hold_ignores_releases_of_other_keys() {
        let mut hold = KeyHold::default();
        assert!(hold.press("enter"));
        hold.release("space");
        assert!(!hold.press("enter"));
    }
}
