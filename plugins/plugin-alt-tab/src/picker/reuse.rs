use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::AltTabConfig;
use crate::layout::*;
use crate::platform;
use crate::platform::WindowInfo;
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(super) fn try_reuse(
    handle: WindowHandle<AltTabApp>,
    created_on_origin: Point<Pixels>,
    config: &AltTabConfig,
    display_windows: Vec<WindowInfo>,
    initial_previews: HashMap<u32, Arc<RenderImage>>,
    icons: HashMap<String, Arc<RenderImage>>,
    tracker: &MonitorTracker,
    icon_cache: Arc<std::sync::Mutex<HashMap<String, Arc<RenderImage>>>>,
    cx: &mut App,
) -> Option<Point<Pixels>> {
    let target_count = display_windows.len().max(1);
    let target_monitor = tracker.snapshot().map(|(m, _)| m);
    let monitor_size = target_monitor.as_ref().map(|m| m.size());
    let (target_w, target_h) = picker_dimensions(
        target_count, config.display.max_columns, monitor_size, config.display.show_hotkey_hints,
    );
    let target_size = size(px(target_w), px(target_h));
    let target_bounds = if let Some(ref active) = target_monitor {
        active.centered_bounds(target_size)
    } else {
        Bounds::centered(None, target_size, cx)
    };

    let target_origin = target_monitor
        .as_ref()
        .map(|m| m.bounds().origin)
        .unwrap_or(point(px(0.0), px(0.0)));
    const MONITOR_TOLERANCE_PX: f64 = 6.0;
    let dx = (created_on_origin.x.to_f64() - target_origin.x.to_f64()).abs();
    let dy = (created_on_origin.y.to_f64() - target_origin.y.to_f64()).abs();
    let monitor_changed = dx > MONITOR_TOLERANCE_PX || dy > MONITOR_TOLERANCE_PX;

    let (card_color, card_opacity) = super::resolve_card_bg(&config.display);

    let reuse_ok = handle
        .update(cx, |view, window: &mut Window, cx| -> bool {
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/hold] reuse path (poll_task={}) — reset={} monitor_changed={}",
                view._alt_poll_task.is_some(), config.reset_selection_on_open, monitor_changed,
            );

            if monitor_changed {
                #[cfg(target_os = "macos")]
                let moved = platform::reposition_picker(
                    target_bounds.origin.x.to_f64(),
                    target_bounds.origin.y.to_f64(),
                );
                #[cfg(not(target_os = "macos"))]
                let moved = platform::move_app_window(
                    "qol-alt-tab-picker",
                    target_bounds.origin.x.to_f64() as i32,
                    target_bounds.origin.y.to_f64() as i32,
                );
                if !moved {
                    return false;
                }
            }

            view.action_mode = config.action_mode.clone();
            view.alt_was_held = true;

            view.delegate.update(cx, |s, _cx| {
                s.label_config = config.label.clone();
                s.transparent_background = config.display.transparent_background;
                s.card_bg_color = card_color;
                s.card_bg_opacity = card_opacity;
                s.show_debug_overlay = config.display.show_debug_overlay;
                s.show_hotkey_hints = config.display.show_hotkey_hints;
            });

            if config.action_mode == crate::config::ActionMode::HoldToSwitch {
                view.start_alt_poll(window.window_handle(), cx);
            } else {
                view._alt_poll_task = None;
            }

            view.apply_cached_windows(
                display_windows.clone(), config.reset_selection_on_open,
                initial_previews.clone(), icons.clone(), cx,
            );

            if config.open_behavior == crate::config::OpenBehavior::CycleOnce
                && config.reset_selection_on_open
                && display_windows.len() >= 2
            {
                view.delegate.update(cx, |s, _cx| s.select_next());
            }

            let current_size = window.window_bounds().get_bounds().size;
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/reuse] target={}x{} current={}x{} count={} cols={}",
                target_w, target_h, current_size.width.to_f64(), current_size.height.to_f64(),
                target_count, preferred_column_count(target_count, config.display.max_columns),
            );
            if (current_size.width.to_f64() - target_w as f64).abs() >= 1.0
                || (current_size.height.to_f64() - target_h as f64).abs() >= 1.0
            {
                window.resize(target_size);
            }
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            true
        })
        .unwrap_or(false);

    if !reuse_ok {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] closing old window — will recreate on correct monitor");
        let _ = handle.update(cx, |_view, window, _cx| { window.remove_window(); });
        return None;
    }

    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] reused existing picker window");

    let _ = handle.update(cx, |view, _window, cx| {
        view.delegate.update(cx, |state, cx| {
            for (wid, img) in initial_previews { state.live_previews.insert(wid, img); }
            cx.notify();
        });
    });

    super::spawn_icon_fill(handle, display_windows, &icons, icon_cache, cx);
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    cx.activate(true);
    Some(target_origin)
}
