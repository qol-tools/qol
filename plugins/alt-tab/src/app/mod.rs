mod input;
mod live_preview;
mod render;

use crate::config::ActionMode;
use crate::picker;
use crate::picker::create::PickerInit;
use crate::picker::gather::GatheredWindows;
use crate::picker::layout::{picker_layout, preview_rect_for_card};
use crate::picker::run::SharedPreviewCache;
use crate::picker::state::PickerState;
use crate::picker::{IconMap, PreviewMap};
use crate::preview_plane::{PreviewPlaneItem, PreviewPlanePayload, PreviewPlaneRect};
use crate::rendering::RenderingFlow;
use gpui::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) static PICKER_VISIBLE: AtomicBool = AtomicBool::new(false);
pub(crate) static ACTIVE_PICKER_MONITOR: std::sync::Mutex<Option<qol_gpui::window::MonitorKey>> =
    std::sync::Mutex::new(None);

const BLUR_GUARD_MS: u64 = 250;

pub(crate) struct AltTabApp {
    pub(crate) picker_title: String,
    pub(crate) delegate: Entity<PickerState>,
    preview_cache: SharedPreviewCache,
    pub(crate) grid_scroll: qol_gpui::scroll_list::SelectionScroll,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) rendering: RenderingFlow,
    pub(crate) action_mode: ActionMode,
    pub(crate) alt_was_held: bool,
    pub(crate) blur_guard_until: Instant,
    pub(crate) _alt_poll_task: Option<Task<()>>,
    _live_preview_task: Option<Task<()>>,
    _dismiss_sub: (Subscription, Subscription, Option<Task<()>>),
    #[cfg(debug_assertions)]
    pending_cycle: Option<PendingCycle>,
}

#[cfg(debug_assertions)]
pub(crate) struct PendingCycle {
    pub(crate) method: &'static str,
    pub(crate) from: Option<usize>,
    pub(crate) started: Instant,
}

#[cfg(debug_assertions)]
thread_local! {
    static CYCLE_ORIGIN: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

#[cfg(debug_assertions)]
pub(crate) fn set_cycle_origin(at: Instant) {
    CYCLE_ORIGIN.with(|cell| cell.set(Some(at)));
}

#[cfg(debug_assertions)]
pub(crate) fn clear_cycle_origin() {
    CYCLE_ORIGIN.with(|cell| cell.set(None));
}

impl AltTabApp {
    pub(crate) fn is_active_visible(&self) -> bool {
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            return false;
        }
        let Ok(lock) = ACTIVE_PICKER_MONITOR.lock() else {
            return false;
        };
        let Some(target) = *lock else {
            return false;
        };
        picker::platform::picker_window_title(target) == self.picker_title
    }

    pub(crate) fn focus_for_keys(&self, _phase: &str, _show_id: Option<u64>, window: &mut Window) {
        #[cfg(debug_assertions)]
        let show_id = _show_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string());
        #[cfg(not(debug_assertions))]
        let show_id = "none";
        #[cfg(debug_assertions)]
        let before = self.focus_handle.is_focused(window);
        #[cfg(not(debug_assertions))]
        let before = false;
        window.focus(&self.focus_handle);
        window.activate_window();
        #[cfg(debug_assertions)]
        let after = self.focus_handle.is_focused(window);
        #[cfg(not(debug_assertions))]
        let after = false;
        #[cfg(debug_assertions)]
        let visible = PICKER_VISIBLE.load(Ordering::Relaxed);
        #[cfg(not(debug_assertions))]
        let visible = false;
        qol_runtime::probe!(
            "KEY_FOCUS",
            "phase={_phase} show_id={show_id} before={before} after={after} visible={visible} title={}",
            self.picker_title
        );
    }

    pub(crate) fn mark_cycle(&mut self, method: &'static str, from: Option<usize>) {
        #[cfg(debug_assertions)]
        {
            let started = CYCLE_ORIGIN
                .with(|cell| cell.take())
                .unwrap_or_else(Instant::now);
            self.pending_cycle = Some(PendingCycle {
                method,
                from,
                started,
            });
        }
        #[cfg(not(debug_assertions))]
        let _ = (method, from);
    }
}

impl AltTabApp {
    pub(crate) fn new(init: PickerInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let should_cycle = init.cycle_on_open && init.windows.len() >= 2;
        let action_mode = init.action_mode.clone();
        let picker_title = init.picker_title.clone();
        let shown = init.shown;
        let rendering = init.rendering;
        let preview_cache = init.preview_cache.clone();
        let delegate: Entity<PickerState> = cx.new(|state_cx| {
            let state = PickerState::from_init(init);
            state_cx
                .on_release(|state: &mut PickerState, app: &mut App| {
                    state.drain_to_registry(app);
                })
                .detach();
            state
        });

        if should_cycle {
            delegate.update(cx, |s, _| s.select_next());
        }

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] AltTabApp::new: action_mode={:?}",
            action_mode
        );

        let dismiss_sub = qol_gpui::ghost::track_dismiss(
            "alt-tab",
            &focus_handle,
            window,
            |this: &Self| this.blur_guard_until,
            |this: &Self| this.is_active_visible(),
            cx,
            |this, window, cx| {
                let picker_visible = this.is_active_visible();
                let modifier_held = picker::is_modifier_held();
                match focus_out_decision(picker_visible, &this.action_mode, modifier_held) {
                    FocusOutDecision::IgnoreHidden => {
                        qol_runtime::probe!(
                            "FOCUS_DISMISS_OUTCOME",
                            "outcome=ignored_hidden visible={picker_visible} mode={:?} modifier_held={modifier_held}",
                            this.action_mode
                        );
                    }
                    FocusOutDecision::RefocusAltHeld => {
                        qol_runtime::probe!(
                            "FOCUS_DISMISS_OUTCOME",
                            "outcome=refocused_alt_held visible={picker_visible} mode={:?} modifier_held={modifier_held}",
                            this.action_mode
                        );
                        this.focus_for_keys("focus-out/alt-held", None, window);
                    }
                    FocusOutDecision::Dismiss => {
                        qol_runtime::probe!(
                            "FOCUS_DISMISS_OUTCOME",
                            "outcome=dismissed visible={picker_visible} mode={:?} modifier_held={modifier_held}",
                            this.action_mode
                        );
                        this.dismiss("focus-lost", window, cx);
                    }
                }
            },
        );

        let mut app = Self {
            _dismiss_sub: dismiss_sub,
            _live_preview_task: None,
            picker_title,
            delegate,
            preview_cache,
            grid_scroll: qol_gpui::scroll_list::SelectionScroll::new(),
            focus_handle,
            rendering,
            action_mode: action_mode.clone(),
            alt_was_held: true,
            blur_guard_until: Instant::now() + Duration::from_millis(BLUR_GUARD_MS),
            _alt_poll_task: None,
            #[cfg(debug_assertions)]
            pending_cycle: None,
        };

        if shown && action_mode == ActionMode::HoldToSwitch {
            app.start_alt_poll(window.to_async(cx).window_handle(), cx);
        }

        app
    }

    pub(crate) fn apply_reuse(
        &mut self,
        req: &crate::picker::ReuseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.log_reuse_layout(req);
        self.apply_reuse_config(req, window, cx);
        self.sync_alt_poll(window, cx);
        self.apply_reuse_windows(req, window, cx);
        self.probe_show_list("reuse", Some(req.show_id), cx);
        true
    }

    fn log_reuse_layout(&self, req: &crate::picker::ReuseRequest) {
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/show] count={} max_cols={} hints={} layout.size={}x{} bounds.origin=({},{})",
            req.gathered.windows.len(),
            req.config.display.max_columns,
            req.config.display.show_hotkey_hints,
            req.layout.size.width.to_f64(),
            req.layout.size.height.to_f64(),
            req.layout.bounds.origin.x.to_f64(),
            req.layout.bounds.origin.y.to_f64(),
        );

        #[cfg(not(debug_assertions))]
        let _ = req;
    }

    fn apply_reuse_config(
        &mut self,
        req: &crate::picker::ReuseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_transparent = self.delegate.read(cx).transparent_background;
        let now_transparent = req.config.display.transparent_background;
        if was_transparent != now_transparent {
            let appearance = if now_transparent {
                WindowBackgroundAppearance::Transparent
            } else {
                WindowBackgroundAppearance::Opaque
            };
            window.set_background_appearance(appearance);
        }
        let (card_color, card_opacity) = crate::picker::resolve_card_bg(&req.config.display);
        self.rendering = req.rendering;
        self.action_mode = req.config.action_mode.clone();
        self.alt_was_held = true;
        self.blur_guard_until = Instant::now() + Duration::from_millis(BLUR_GUARD_MS);
        self.delegate.update(cx, |s, _| {
            s.apply_config(req.config, card_color, card_opacity, req.monitor_size)
        });
        if now_transparent {
            picker::platform::disable_window_shadow(&self.picker_title);
        }
    }

    fn sync_alt_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_mode == ActionMode::HoldToSwitch {
            self.start_alt_poll(window.window_handle(), cx);
            return;
        }
        self._alt_poll_task = None;
    }

    fn apply_reuse_windows(
        &mut self,
        req: &crate::picker::ReuseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_gathered(req.gathered, req.config.reset_selection_on_open, window, cx);
        if req.config.open_behavior != crate::config::OpenBehavior::CycleOnce {
            return;
        }
        if !req.config.reset_selection_on_open {
            return;
        }
        if req.gathered.windows.len() < 2 {
            return;
        }
        self.delegate.update(cx, |s, _| s.cycle(req.reverse));
    }

    fn probe_show_list(&self, _path: &str, _show_id: Option<u64>, _cx: &Context<Self>) {
        #[cfg(debug_assertions)]
        {
            let show_id = _show_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string());
            let state = self.delegate.read(_cx);
            let head: Vec<String> = state
                .windows
                .iter()
                .take(6)
                .map(|w| format!("{}:{}:\"{:.24}\"", w.id, w.app_name, w.title))
                .collect();
            let order: Vec<String> = state
                .windows
                .iter()
                .take(24)
                .map(|w| w.id.to_string())
                .collect();
            qol_runtime::probe!(
                "SHOW_LIST",
                "show_id={show_id} path={_path} sel={:?} n={} head=[{}] order=[{}]",
                state.selected_index,
                state.windows.len(),
                head.join(" "),
                order.join(" "),
            );
        }
    }

    fn apply_gathered(
        &mut self,
        gathered: &GatheredWindows,
        reset: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(debug_assertions)]
        let title = self.picker_title.clone();
        self.delegate.update(cx, |state, ctx| {
            #[cfg(debug_assertions)]
            let was_stale = state
                .windows
                .iter()
                .any(|w| w.app_name.starts_with("__warmup_"));
            state.set_windows(gathered.windows.clone(), reset, ctx, Some(&mut *window));
            state.replace_caches(
                gathered.previews.clone(),
                gathered.icons.clone(),
                ctx,
                Some(&mut *window),
            );
            #[cfg(debug_assertions)]
            {
                let is_stale = state
                    .windows
                    .iter()
                    .any(|w| w.app_name.starts_with("__warmup_"));
                if was_stale && !is_stale {
                    qol_runtime::probe!("PICKER_READY", "title={title}");
                }
            }
            ctx.notify();
        });
        cx.notify();
    }

    pub(crate) fn apply_ghost_gathered(
        &mut self,
        gathered: &GatheredWindows,
        reset: bool,
        rest_forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_gathered(gathered, reset, window, cx);
        if rest_forward {
            self.delegate.update(cx, |s, _| s.select_next());
            cx.notify();
        }
        self.probe_show_list("ghost", None, cx);
    }

    pub(crate) fn update_icons(
        &mut self,
        icons: IconMap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delegate.update(cx, |state, ctx| {
            state.insert_icons(icons, ctx, Some(window))
        });
        cx.notify();
    }

    pub(crate) fn update_previews(
        &mut self,
        previews: PreviewMap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delegate.update(cx, |state, ctx| {
            state.insert_fresh_previews(previews, ctx, Some(window))
        });
        cx.notify();
    }

    pub(crate) fn ensure_live_preview(&mut self, cx: &mut Context<Self>) {
        if !self.rendering.captures_live_selection() {
            qol_runtime::probe!(
                "PREVIEW_LIVE",
                "outcome=skipped reason=preview_plane backend={}",
                self.rendering.preview_plane_backend().unwrap_or("none")
            );
            return;
        }
        if self._live_preview_task.is_some() {
            return;
        }
        self._live_preview_task = Some(live_preview::spawn(
            self.delegate.clone(),
            self.preview_cache.clone(),
            cx,
        ));
    }

    pub(crate) fn sync_preview_plane(
        &self,
        show_id: Option<u64>,
        window: &Window,
        cx: &Context<Self>,
    ) {
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            return;
        }
        if self.rendering.preview_plane_backend().is_none() {
            return;
        }

        let state = self.delegate.read(cx);
        let layout = picker_layout(
            state.windows.len().max(1),
            state.max_columns,
            state.layout_budget,
            state.show_hotkey_hints,
            state.card_scale,
            state.card_padding,
            state.dynamic_card_scale,
        );
        let metrics = layout.metrics;
        let bounds = window.bounds();
        let win_x = bounds.origin.x.to_f64() as f32;
        let win_y = bounds.origin.y.to_f64() as f32;
        let win_w = bounds.size.width.to_f64() as f32;
        let win_h = bounds.size.height.to_f64() as f32;
        let panel_x = win_x + ((win_w - layout.width) / 2.0).max(0.0);
        let panel_y = win_y + ((win_h - layout.height) / 2.0).max(0.0);
        let panel_right = panel_x + layout.width;
        let panel_bottom = panel_y + layout.height;
        let show_id = show_id
            .map(|id| format!("show#{id}"))
            .unwrap_or_else(|| "visible".to_string());

        let mut skipped = 0usize;
        let items: Vec<_> = state
            .windows
            .iter()
            .enumerate()
            .filter_map(|(index, win)| {
                if win.is_minimized {
                    skipped += 1;
                    return None;
                }
                let rect = preview_rect_for_card(
                    index,
                    state.selected_index == Some(index),
                    layout.columns,
                    (panel_x, panel_y),
                    state.show_hotkey_hints,
                    &metrics,
                );
                if rect.x < panel_x
                    || rect.y < panel_y
                    || rect.x + rect.w > panel_right
                    || rect.y + rect.h > panel_bottom
                {
                    skipped += 1;
                    return None;
                }
                Some(PreviewPlaneItem {
                    wid: win.id,
                    selected: state.selected_index == Some(index),
                    title: win.title.clone(),
                    rect: PreviewPlaneRect {
                        x: rect.x,
                        y: rect.y,
                        w: rect.w,
                        h: rect.h,
                    },
                })
            })
            .collect();

        qol_runtime::probe!(
            "PREVIEW_PLANE_LAYOUT",
            "show_id={} items={} skipped={} cols={} window=({:.0},{:.0} {:.0}x{:.0}) panel=({:.0},{:.0} {:.0}x{:.0})",
            show_id,
            items.len(),
            skipped,
            layout.columns,
            win_x,
            win_y,
            win_w,
            win_h,
            panel_x,
            panel_y,
            layout.width,
            layout.height
        );

        if items.is_empty() {
            crate::preview_plane::hide_async("empty");
            return;
        }

        crate::preview_plane::show_async(PreviewPlanePayload::new(show_id, items));
    }

    pub(crate) fn start_alt_poll(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        qol_runtime::probe!("ALT_POLL_START", "title={}", self.picker_title);
        let delegate = self.delegate.clone();
        self.alt_was_held = true;
        self._alt_poll_task = Some(cx.spawn(move |this, cx: &mut AsyncApp| {
            alt_release_check(this, delegate, window_handle, cx.clone())
        }));
    }
}

const ALT_POLL_INTERVAL_MS: u64 = 30;

async fn alt_release_check(
    this: WeakEntity<AltTabApp>,
    delegate: Entity<PickerState>,
    window_handle: AnyWindowHandle,
    mut cx: AsyncApp,
) {
    let executor = cx.background_executor().clone();
    qol_gpui::probe::probe("ALT_POLL", "start");
    loop {
        if this.upgrade().is_none() {
            qol_gpui::probe::probe("ALT_POLL", "entity gone, NO dismiss");
            return;
        }
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            qol_gpui::probe::probe("ALT_POLL", "hidden before release -> no activate");
            return;
        }
        if !picker::is_modifier_held() {
            break;
        }
        executor
            .timer(Duration::from_millis(ALT_POLL_INTERVAL_MS))
            .await;
    }
    qol_gpui::probe::probe("ALT_POLL", "release detected -> activate+dismiss");
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/hold] Alt released via poll — activating selection");
    let weak = this.clone();
    let updated = cx.update_window(window_handle, move |_, window, cx| {
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            qol_gpui::probe::probe("ALT_POLL", "hidden in update -> no activate");
            return;
        }
        if let Some(entity) = weak.upgrade() {
            entity.update(cx, |app, cx| {
                if !app.is_active_visible() {
                    qol_gpui::probe::probe("ALT_POLL", "inactive picker in update -> no activate");
                    return;
                }
                app.dismiss("alt-release/poll", window, cx);
                delegate.update(cx, |s, _| s.activate_selected_target());
            });
        } else {
            qol_gpui::probe::probe("ALT_POLL", "weak gone in update, NO dismiss");
        }
    });
    if updated.is_err() {
        qol_gpui::probe::probe("ALT_POLL", "update_window FAILED, NO dismiss");
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FocusOutDecision {
    IgnoreHidden,
    RefocusAltHeld,
    Dismiss,
}

fn focus_out_decision(
    picker_visible: bool,
    action_mode: &ActionMode,
    modifier_held: bool,
) -> FocusOutDecision {
    if !picker_visible {
        return FocusOutDecision::IgnoreHidden;
    }
    if action_mode == &ActionMode::HoldToSwitch && modifier_held {
        return FocusOutDecision::RefocusAltHeld;
    }
    FocusOutDecision::Dismiss
}

impl AltTabApp {
    pub(crate) fn dismiss(
        &mut self,
        _source: &'static str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/dismiss] from={}", _source);
        let active = self.is_active_visible();
        qol_runtime::probe!(
            "DISMISS",
            "from={_source} title={} active={active}",
            self.picker_title
        );
        if self.rendering.preview_plane_backend().is_some() {
            crate::preview_plane::hide_async(_source);
        }
        if !active {
            qol_runtime::probe!(
                "DISMISS",
                "from={_source} title={} outcome=ignored_inactive",
                self.picker_title
            );
            return;
        }
        self._alt_poll_task = None;
        PICKER_VISIBLE.store(false, Ordering::Relaxed);
        if let Ok(mut lock) = ACTIVE_PICKER_MONITOR.lock() {
            *lock = None;
        }
        qol_gpui::ghost::dismiss_to_ghost_with(
            &self.picker_title,
            picker::platform::picker_window_title,
        );
        qol_gpui::popup_window::restore_composite(&self.picker_title);
        picker::platform::probe_picker_app_active("dismiss");
        picker::request_frontmost_preview_refresh();
        cx.notify();
    }
}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod focus_out_tests {
    use super::{focus_out_decision, FocusOutDecision};
    use crate::config::ActionMode;

    #[test]
    fn hidden_picker_ignores_focus_out_regardless_of_other_signals() {
        let cases = [
            (ActionMode::HoldToSwitch, true),
            (ActionMode::HoldToSwitch, false),
            (ActionMode::Sticky, true),
            (ActionMode::Sticky, false),
        ];
        for (mode, modifier) in cases {
            assert_eq!(
                focus_out_decision(false, &mode, modifier),
                FocusOutDecision::IgnoreHidden,
                "hidden picker must win over mode={mode:?} modifier={modifier}",
            );
        }
    }

    #[test]
    fn hold_mode_recovers_focus_out_while_alt_is_held() {
        assert_eq!(
            focus_out_decision(true, &ActionMode::HoldToSwitch, true),
            FocusOutDecision::RefocusAltHeld
        );
    }

    #[test]
    fn hold_mode_dismisses_without_activating_on_click_outside() {
        assert_eq!(
            focus_out_decision(true, &ActionMode::HoldToSwitch, false),
            FocusOutDecision::Dismiss
        );
    }

    #[test]
    fn sticky_mode_keeps_focus_out_as_plain_dismiss() {
        assert_eq!(
            focus_out_decision(true, &ActionMode::Sticky, true),
            FocusOutDecision::Dismiss
        );
    }
}
