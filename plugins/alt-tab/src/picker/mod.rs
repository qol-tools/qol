pub(crate) mod create;
pub(crate) mod gather;
pub(crate) mod layout;
mod monitor_listener;
pub(crate) mod platform;
mod reuse;
pub(crate) mod run;
pub(crate) mod state;

pub(crate) use monitor_listener::request_frontmost_preview_refresh;
pub use platform::is_modifier_held;
pub(crate) use reuse::ReuseRequest;

use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::capture;
use crate::config::{ActionMode, AltTabConfig, DisplayConfig, DEFAULT_CARD_BACKGROUND_COLOR};
use crate::rendering::RenderingFlow;
use gather::{gather, spawn_icon_fill, GatheredWindows, IconFillRequest};
use gpui::*;
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_gpui::theme::resolve_surface_color;
use qol_gpui::window::{MonitorKey, PopupPlacement};
use qol_gpui::MonitorBounds;
use run::{SharedPreviewCache, WindowCache};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub(crate) type PreviewMap = std::collections::HashMap<u32, Arc<gpui::RenderImage>>;
pub(crate) type LiveFrameMap = std::collections::HashMap<u32, capture::LiveFrame>;
pub(crate) type IconMap = std::collections::HashMap<String, Arc<gpui::RenderImage>>;
pub(crate) type SharedIconCache = Arc<std::sync::Mutex<IconMap>>;
pub(crate) type PickerWindowState =
    std::rc::Rc<std::cell::RefCell<qol_gpui::window::ActiveWindows<AltTabApp>>>;

const DEFAULT_ESTIMATED_WINDOW_COUNT: usize = 8;

pub(crate) fn default_estimated_window_count() -> usize {
    DEFAULT_ESTIMATED_WINDOW_COUNT
}

pub(crate) struct OpenPickerRequest<'a> {
    pub config: &'a AltTabConfig,
    pub current: &'a PickerWindowState,
    pub tracker: &'a MonitorTracker,
    pub last_window_count: Arc<AtomicUsize>,
    pub icon_cache: SharedIconCache,
    pub window_cache: WindowCache,
    pub preview_cache: SharedPreviewCache,
    pub has_shown_once: Arc<AtomicBool>,
    pub reverse: bool,
    pub show_id: u64,
}

pub(crate) fn open_picker(req: &OpenPickerRequest, cx: &mut App) {
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/open] show request id={} reverse={}",
        req.show_id, req.reverse
    );

    crate::actions::cancel_pending_activation();
    let is_visible = PICKER_VISIBLE.load(Ordering::Relaxed);
    let placement = resolve_placement(req.tracker, req.current, is_visible);
    let rendering = RenderingFlow::current();
    monitor_listener::store_flow_fill(rendering.captures_preview_fill());
    qol_runtime::probe!(
        "OPEN_PICKER",
        "show_id={} reverse={} visible={} target={},{},{}x{}",
        req.show_id,
        req.reverse,
        is_visible,
        placement.target().x,
        placement.target().y,
        placement.target().width,
        placement.target().height,
    );
    rendering.trace_show(req.show_id);

    if req.reverse && req.current.borrow().is_empty() {
        qol_runtime::probe!(
            "OPEN_PICKER_SKIP",
            "show_id={} reason=reverse_empty",
            req.show_id
        );
        return;
    }
    if try_cycle_existing(req.current, req.reverse, req.show_id, &placement, cx) {
        return;
    }

    let gathered = gather(
        req.config,
        &req.icon_cache,
        &req.window_cache,
        &req.preview_cache,
    );
    if try_reuse_existing(req, rendering, &placement, &gathered, cx) {
        return;
    }
    destroy_non_target_windows(req, &placement, cx);
    create_from_request(req, rendering, placement, gathered, cx);
}

fn resolve_placement(
    tracker: &MonitorTracker,
    current: &PickerWindowState,
    is_visible: bool,
) -> PopupPlacement {
    let placement = PopupPlacement::from_tracker(tracker);
    if !is_visible {
        let placement = stabilize_placement(placement, current, "hidden");
        *crate::app::ACTIVE_PICKER_MONITOR.lock().unwrap() = Some(placement.target());
        return placement;
    }
    let Some(active_target) = *crate::app::ACTIVE_PICKER_MONITOR.lock().unwrap() else {
        return stabilize_placement(placement, current, "visible-no-active-target");
    };
    tracker
        .all_monitors()
        .into_iter()
        .find(|m| MonitorKey::from_bounds(&m.bounds()) == active_target)
        .map(|monitor| PopupPlacement::from_monitor(Some(monitor)))
        .or_else(|| placement_from_key(active_target, "visible-active-target"))
        .unwrap_or_else(|| stabilize_placement(placement, current, "visible"))
}

fn stabilize_placement(
    placement: PopupPlacement,
    current: &PickerWindowState,
    reason: &'static str,
) -> PopupPlacement {
    if placement.monitor_size().is_some() {
        return placement;
    }
    if let Some((key, _)) = any_existing(current) {
        if let Some(placement) = placement_from_key(key, reason) {
            return placement;
        }
    }
    qol_runtime::probe!("PLACEMENT_FALLBACK", "reason={reason} source=default");
    placement
}

fn placement_from_key(key: MonitorKey, _reason: &'static str) -> Option<PopupPlacement> {
    if key.width <= 0 || key.height <= 0 {
        return None;
    }
    qol_runtime::probe!(
        "PLACEMENT_FALLBACK",
        "reason={_reason} source=existing target={},{},{}x{}",
        key.x,
        key.y,
        key.width,
        key.height,
    );
    Some(PopupPlacement::from_monitor(Some(
        ActiveMonitor::from_bounds(MonitorBounds {
            x: key.x as f32,
            y: key.y as f32,
            width: key.width as f32,
            height: key.height as f32,
        }),
    )))
}

fn try_cycle_existing(
    current: &PickerWindowState,
    reverse: bool,
    show_id: u64,
    placement: &PopupPlacement,
    cx: &mut App,
) -> bool {
    cycle_existing_window(current, Some(placement.target()), reverse, show_id, cx)
}

fn cycle_existing_window(
    current: &PickerWindowState,
    target: Option<MonitorKey>,
    reverse: bool,
    show_id: u64,
    cx: &mut App,
) -> bool {
    let handle = match target
        .and_then(|key| current.borrow().existing(key))
        .or_else(|| any_existing(current).map(|(_, h)| h))
    {
        Some(h) => h,
        None => {
            qol_runtime::probe!("CYCLE_EXISTING", "show_id={show_id} outcome=no_handle");
            return false;
        }
    };
    if !try_cycle_selection(&handle, reverse, show_id, cx) {
        qol_runtime::probe!("CYCLE_EXISTING", "show_id={show_id} outcome=rejected");
        return false;
    }
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    cx.activate(true);
    probe_app_active_after_frame(handle, cx);
    qol_runtime::probe!("CYCLE_EXISTING", "show_id={show_id} outcome=cycled");
    true
}

fn probe_app_active_after_frame(handle: WindowHandle<AltTabApp>, cx: &mut App) {
    #[cfg(debug_assertions)]
    {
        let _ = handle.update(cx, |_, window, _| {
            window.on_next_frame(|_, _| platform::probe_picker_app_active("show"));
        });
    }
    #[cfg(not(debug_assertions))]
    let _ = (handle, cx);
}

pub(super) fn try_cycle_visible(
    current: &PickerWindowState,
    reverse: bool,
    show_id: u64,
    cx: &mut App,
) -> bool {
    if !PICKER_VISIBLE.load(Ordering::Relaxed) {
        return false;
    }
    let target = *crate::app::ACTIVE_PICKER_MONITOR.lock().unwrap();
    cycle_existing_window(current, target, reverse, show_id, cx)
}

fn try_reuse_existing(
    req: &OpenPickerRequest,
    rendering: RenderingFlow,
    placement: &PopupPlacement,
    gathered: &GatheredWindows,
    cx: &mut App,
) -> bool {
    if !platform::reuse_hidden_picker_across_shows() {
        discard_any_existing_window(req, cx);
        return false;
    }
    let target = placement.target();
    let (handle, source_key) = if let Some(h) = req.current.borrow().existing(target) {
        (h, target)
    } else if platform::reuse_picker_across_targets() {
        match any_existing(req.current) {
            Some((key, h)) => (h, key),
            None => return false,
        }
    } else {
        return false;
    };
    if source_key != target && !platform::reuse_picker_across_targets() {
        discard_old_window(req, source_key, handle, cx);
        return false;
    }
    let input = reuse::LayoutInput { placement };
    let layout = reuse::compute_layout(&input, cx);
    let all_titles = req
        .current
        .borrow()
        .titles_with(platform::picker_window_title);
    let reuse_req = reuse::ReuseRequest {
        handle: &handle,
        layout: &layout,
        config: req.config,
        gathered,
        all_titles: &all_titles,
        reverse: req.reverse,
        monitor_size: placement.monitor_size(),
        show_id: req.show_id,
        rendering,
    };
    if reuse::try_reuse(&reuse_req, cx) {
        if source_key != target {
            req.current.borrow_mut().remove(source_key);
            req.current.borrow_mut().insert(target, handle);
        }
        finalize_reuse(req, handle, gathered, cx);
        return true;
    }
    discard_old_window(req, source_key, handle, cx);
    false
}

fn any_existing(current: &PickerWindowState) -> Option<(MonitorKey, WindowHandle<AltTabApp>)> {
    current.borrow().iter().into_iter().next()
}

fn destroy_non_target_windows(req: &OpenPickerRequest, placement: &PopupPlacement, cx: &mut App) {
    platform::destroy_non_target_windows(req.current, placement.target(), cx);
}

fn discard_any_existing_window(req: &OpenPickerRequest, cx: &mut App) {
    let Some((key, handle)) = any_existing(req.current) else {
        return;
    };
    discard_old_window(req, key, handle, cx);
}

fn discard_old_window(
    req: &OpenPickerRequest,
    target: qol_gpui::window::MonitorKey,
    handle: WindowHandle<AltTabApp>,
    cx: &mut App,
) {
    platform::discard_old_window(req.current, target, handle, cx);
}

fn create_from_request(
    req: &OpenPickerRequest,
    rendering: RenderingFlow,
    placement: PopupPlacement,
    gathered: GatheredWindows,
    cx: &mut App,
) {
    let create_req = create::CreateRequest {
        config: req.config,
        placement,
        last_window_count: req.last_window_count.clone(),
        icon_cache: req.icon_cache.clone(),
        preview_cache: req.preview_cache.clone(),
        current: req.current,
        has_shown_once: req.has_shown_once.clone(),
        show_id: req.show_id,
        rendering,
    };
    create::create_new(&create_req, gathered, cx);
}

fn try_cycle_selection(
    handle: &WindowHandle<AltTabApp>,
    reverse: bool,
    _show_id: u64,
    cx: &mut App,
) -> bool {
    handle
        .update(cx, |view, window: &mut Window, cx| -> bool {
            if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                qol_runtime::probe!("CYCLE_SELECTION", "show_id={_show_id} outcome=hidden");
                return false;
            }
            if view.action_mode != ActionMode::HoldToSwitch {
                qol_runtime::probe!(
                    "CYCLE_SELECTION",
                    "show_id={_show_id} outcome=wrong_mode mode={:?}",
                    view.action_mode
                );
                return false;
            }
            view.ensure_live_preview(cx);
            if view._alt_poll_task.is_none() {
                view.start_alt_poll(window.window_handle(), cx);
            }
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/hold] window already visible (reverse={}) — cycling",
                reverse
            );
            let from = view.delegate.read(cx).selected_index;
            view.delegate.update(cx, |s, _| s.cycle(reverse));
            view.mark_cycle(if reverse { "shift-tab" } else { "tab" }, from);
            cx.notify();
            qol_runtime::probe!("CYCLE_SELECTION", "show_id={_show_id} outcome=cycled");
            true
        })
        .unwrap_or(false)
}

fn finalize_reuse(
    req: &OpenPickerRequest,
    handle: WindowHandle<AltTabApp>,
    gathered: &GatheredWindows,
    cx: &mut App,
) {
    let previews = gathered.previews.clone();
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    req.has_shown_once.store(true, Ordering::Release);
    let _ = handle.update(cx, |view, window, cx| {
        view.ensure_live_preview(cx);
        view.delegate.update(cx, |state, ctx| {
            state.insert_previews(previews, ctx, Some(window))
        });
        cx.notify();
    });
    let icon_req = IconFillRequest {
        handle,
        windows: gathered.windows.clone(),
        icon_cache: req.icon_cache.clone(),
    };
    spawn_icon_fill(icon_req, &gathered.icons, cx);
    cx.activate(true);
    probe_app_active_after_frame(handle, cx);
}

pub(crate) fn resolve_card_bg(display: &DisplayConfig) -> (u32, f32) {
    resolve_surface_color(
        &display.card_background_color,
        DEFAULT_CARD_BACKGROUND_COLOR,
        display.card_background_brightness,
        display.card_background_opacity,
    )
}

#[cfg(test)]
mod color_tests {
    use super::resolve_card_bg;
    use crate::config::DisplayConfig;

    #[test]
    fn card_background_accepts_color_picker_hex() {
        let display = DisplayConfig {
            card_background_color: "#203040".to_string(),
            card_background_brightness: 1.0,
            card_background_opacity: 1.2,
            ..Default::default()
        };
        assert_eq!(resolve_card_bg(&display), (0x203040, 1.0));
    }

    #[test]
    fn card_background_brightness_dims_color_picker_hex() {
        let display = DisplayConfig {
            card_background_color: "#ff8040".to_string(),
            card_background_brightness: 0.25,
            ..Default::default()
        };
        assert_eq!(resolve_card_bg(&display), (0x402010, 0.85));
    }

    #[test]
    fn card_background_brightness_is_clamped() {
        let bright = DisplayConfig {
            card_background_color: "#102030".to_string(),
            card_background_brightness: 2.0,
            ..Default::default()
        };
        let dark = DisplayConfig {
            card_background_color: "#102030".to_string(),
            card_background_brightness: -1.0,
            ..Default::default()
        };

        assert_eq!(resolve_card_bg(&bright), (0x102030, 0.85));
        assert_eq!(resolve_card_bg(&dark), (0x000000, 0.85));
    }

    #[test]
    fn invalid_card_background_falls_back_to_default() {
        let display = DisplayConfig {
            card_background_color: "nope".to_string(),
            card_background_opacity: -1.0,
            ..Default::default()
        };
        assert_eq!(resolve_card_bg(&display), (0x202322, 0.0));
    }
}

#[cfg(test)]
mod placement_tests {
    use super::placement_from_key;
    use qol_gpui::window::MonitorKey;

    #[test]
    fn placement_from_nonzero_key_preserves_monitor_size() {
        let key = MonitorKey {
            x: 0,
            y: 0,
            width: 1800,
            height: 1169,
        };

        let placement = placement_from_key(key, "test").expect("valid monitor key");

        assert_eq!(placement.target(), key);
        assert_eq!(placement.monitor_size(), Some((1800.0, 1169.0)));
    }

    #[test]
    fn placement_from_zero_key_is_not_a_real_monitor() {
        let key = MonitorKey {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };

        assert!(placement_from_key(key, "test").is_none());
    }
}
