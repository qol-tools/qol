pub(crate) mod keepalive;
pub(crate) mod run;

use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::{parse_hex_color, ActionMode, AltTabConfig, DisplayConfig};
use crate::icon::build_icon_cache;
use crate::layout::*;
use crate::monitor::MonitorTracker;
use crate::platform;
use crate::platform::WindowInfo;
use crate::preview::bgra_to_render_image;
use gpui::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const DEFAULT_ESTIMATED_WINDOW_COUNT: usize = 8;

pub(crate) fn default_estimated_window_count() -> usize {
    DEFAULT_ESTIMATED_WINDOW_COUNT
}

pub(crate) fn open_picker(
    config: &AltTabConfig,
    current: &std::rc::Rc<std::cell::RefCell<Option<(WindowHandle<AltTabApp>, Point<Pixels>)>>>,
    tracker: &MonitorTracker,
    last_window_count: Arc<AtomicUsize>,
    window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>>,
    preview_cache: Arc<std::sync::Mutex<HashMap<u32, Arc<RenderImage>>>>,
    icon_cache: Arc<std::sync::Mutex<HashMap<String, Arc<RenderImage>>>>,
    reverse: bool,
    cx: &mut App,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] show request (reverse={})", reverse);

    // Reverse only cycles within an already-open picker — never opens one.
    if reverse && current.borrow().is_none() {
        return;
    }

    let display_windows: Vec<WindowInfo> = {
        let all = platform::get_open_windows();
        if config.display.show_minimized {
            all
        } else {
            all.into_iter().filter(|w| !w.is_minimized).collect()
        }
    };
    // Update the cache centrally so background processes see the current layout
    if let Ok(mut cache) = window_cache.lock() {
        *cache = display_windows.clone();
    }

    // Fast path: if picker is already visible and alt held, just cycle selection.
    let existing = current.borrow().clone();
    if let Some((ref handle, _)) = existing {
        let cycled = handle
            .update(cx, |view, _window: &mut Window, cx| -> bool {
                let alt_held = platform::is_modifier_held();
                if view.action_mode == ActionMode::HoldToSwitch
                    && view._alt_poll_task.is_some()
                    && alt_held
                {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[alt-tab/hold] window already visible (alt_held={} reverse={}) — cycling",
                        alt_held, reverse
                    );
                    view.delegate.update(cx, |s, _cx| {
                        if reverse {
                            s.select_prev();
                        } else {
                            s.select_next();
                        }
                    });
                    cx.notify();
                    return true;
                }
                false
            })
            .unwrap_or(false);
        if cycled {
            cx.activate(true);
            return;
        }
    }

    // Grab pre-warmed previews from cache (instant). Only capture missing windows.
    let mut initial_previews: HashMap<u32, Arc<RenderImage>> = HashMap::new();
    let mut missing_targets: Vec<(usize, u32)> = Vec::new();
    if let Ok(pcache) = preview_cache.lock() {
        for (i, win) in display_windows.iter().enumerate() {
            if let Some(img) = pcache.get(&win.id) {
                initial_previews.insert(win.id, img.clone());
            } else {
                missing_targets.push((i, win.id));
            }
        }
    } else {
        missing_targets = display_windows
            .iter()
            .enumerate()
            .map(|(i, w)| (i, w.id))
            .collect();
    }
    // Synchronous CG capture only for windows not yet in the prewarm cache
    if !missing_targets.is_empty() {
        for (idx, rgba_opt) in
            platform::capture_previews_cg(&missing_targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        {
            let Some(rgba) = rgba_opt else { continue };
            let Some(win) = display_windows.get(idx) else { continue };
            if let Some(img) = bgra_to_render_image(&rgba.data, rgba.width, rgba.height) {
                initial_previews.insert(win.id, img);
            }
        }
    }

    let icons = icon_cache
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default();

    // Reuse existing picker window if possible (reopen after dismiss).
    if let Some((handle, created_on_origin)) = existing {
        let target_count = display_windows.len().max(1);
        let target_monitor = tracker.snapshot().map(|(m, _)| m);
        let monitor_size = target_monitor.as_ref().map(|m| m.size());
        let (target_w, target_h) =
            picker_dimensions(target_count, config.display.max_columns, monitor_size);
        let target_size = size(px(target_w), px(target_h));
        let target_bounds = if let Some(ref active) = target_monitor {
            active.centered_bounds(target_size)
        } else {
            Bounds::centered(None, target_size, cx)
        };

        // Determine if the target monitor differs from the one the window was created on.
        let target_origin = target_monitor
            .as_ref()
            .map(|m| m.bounds().origin)
            .unwrap_or(point(px(0.0), px(0.0)));
        const MONITOR_TOLERANCE_PX: f64 = 6.0;
        let monitor_changed = {
            let dx = (created_on_origin.x.to_f64() - target_origin.x.to_f64()).abs();
            let dy = (created_on_origin.y.to_f64() - target_origin.y.to_f64()).abs();
            dx > MONITOR_TOLERANCE_PX || dy > MONITOR_TOLERANCE_PX
        };

        let reuse_ok = handle
            .update(cx, |view, window: &mut Window, cx| -> bool {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[alt-tab/hold] reuse path (poll_task={}) — applying config reset={} monitor_changed={}",
                    view._alt_poll_task.is_some(),
                    config.reset_selection_on_open,
                    monitor_changed,
                );

                if monitor_changed {
                    let x = target_bounds.origin.x.to_f64() as i32;
                    let y = target_bounds.origin.y.to_f64() as i32;
                    if !platform::move_app_window("qol-alt-tab-picker", x, y) {
                        return false;
                    }
                }

                #[cfg(debug_assertions)]
                eprintln!(
                    "[alt-tab/hold] open_picker reusing window: setting action_mode={:?}",
                    config.action_mode
                );

                view.action_mode = config.action_mode.clone();
                view.alt_was_held = true;

                let (card_color, card_opacity) = resolve_card_bg(&config.display);
                view.delegate.update(cx, |s, _cx| {
                    s.label_config = config.label.clone();
                    s.transparent_background = config.display.transparent_background;
                    s.card_bg_color = card_color;
                    s.card_bg_opacity = card_opacity;
                    s.show_debug_overlay = config.display.show_debug_overlay;
                });

                if config.action_mode == ActionMode::HoldToSwitch {
                    let wh = window.window_handle();
                    view.start_alt_poll(wh, cx);
                } else {
                    view._alt_poll_task = None;
                }

                view.apply_cached_windows(
                    display_windows.clone(),
                    config.reset_selection_on_open,
                    initial_previews.clone(),
                    icons.clone(),
                    cx,
                );

                let current_bounds = window.window_bounds().get_bounds();
                let current_size = current_bounds.size;
                let next_size = target_size;
                if (current_size.width.to_f64() - target_w as f64).abs() >= 1.0
                    || (current_size.height.to_f64() - target_h as f64).abs() >= 1.0
                {
                    window.resize(next_size);
                }
                window.focus(&view.focus_handle(cx));
                window.activate_window();
                true
            })
            .unwrap_or(false);

        if reuse_ok {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/open] reused existing picker window");
            if !initial_previews.is_empty() {
                let _ = handle.update(cx, |view, _window, cx| {
                    view.delegate.update(cx, |state, cx| {
                        for (wid, img) in initial_previews {
                            state.live_previews.insert(wid, img);
                        }
                        cx.notify();
                    });
                });
            }
            // Async-fill missing icons for reuse path too
            let missing_apps: Vec<String> = display_windows
                .iter()
                .map(|w| w.app_name.clone())
                .filter(|name| !icons.contains_key(name))
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            if !missing_apps.is_empty() {
                let windows_for_icons = display_windows;
                let icon_cache_for_fill = icon_cache;
                let handle_for_fill = handle;
                cx.spawn(async move |cx: &mut AsyncApp| {
                    let executor = cx.background_executor().clone();
                    let raw_icons = executor
                        .spawn(async move { platform::get_app_icons(&windows_for_icons) })
                        .await;
                    if raw_icons.is_empty() {
                        return;
                    }
                    let rendered = build_icon_cache(raw_icons);
                    if let Ok(mut icache) = icon_cache_for_fill.lock() {
                        for (k, v) in &rendered {
                            icache.insert(k.clone(), v.clone());
                        }
                    }
                    let _ = cx.update(|cx| {
                        let _ = handle_for_fill.update(cx, |view, _window, cx| {
                            view.delegate.update(cx, |state, cx| {
                                for (name, img) in rendered {
                                    state.icon_cache.insert(name, img);
                                }
                                cx.notify();
                            });
                        });
                    });
                })
                .detach();
            }
            cx.activate(true);
            return;
        }

        // Close the old window so we don't leak orphaned windows
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] closing old window — will recreate on correct monitor");
        let _ = handle.update(cx, |_view, window, _cx| {
            window.remove_window();
        });
        *current.borrow_mut() = None;
    }

    let target_count = display_windows.len().max(1);
    let estimated_count = target_count
        .max(last_window_count.load(Ordering::Relaxed))
        .max(1);
    let create_monitor = tracker.snapshot().map(|(m, _)| m);
    let monitor_size = create_monitor.as_ref().map(|m| m.size());
    let (win_w, win_h) =
        picker_dimensions(estimated_count, config.display.max_columns, monitor_size);
    let win_size = size(px(win_w), px(win_h));
    let bounds = if let Some(ref active) = create_monitor {
        active.centered_bounds(win_size)
    } else {
        Bounds::centered(None, win_size, cx)
    };
    let create_origin = create_monitor
        .as_ref()
        .map(|m| m.bounds().origin)
        .unwrap_or(point(px(0.0), px(0.0)));

    println!(
        "[alt-tab] opening at {:?} with size {:?}",
        bounds.origin, bounds.size
    );

    let action_mode_for_init = config.action_mode.clone();
    let display_windows_for_init = display_windows.clone();
    let config_for_init = config.clone();
    let cycle_on_open = config.open_behavior == crate::config::OpenBehavior::CycleOnce;
    let icons_for_init = icons.clone();
    let transparent_bg = config.display.transparent_background;
    let show_debug_overlay = config.display.show_debug_overlay;
    let (card_color_init, card_opacity_init) = resolve_card_bg(&config.display);

    let window_background = if transparent_bg {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    };

    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(if transparent_bg { WindowDecorations::Server } else { WindowDecorations::Client }),
            kind: platform::picker_window_kind(),
            focus: true,
            window_background: window_background,
            ..Default::default()
        },
        move |window, cx| {
            window.set_window_title("qol-alt-tab-picker");
            let label_config = config_for_init.label.clone();
            let transparent_background = config_for_init.display.transparent_background;
            let view = cx.new(|cx| {
                AltTabApp::new(
                    window,
                    cx,
                    action_mode_for_init,
                    display_windows_for_init,
                    label_config,
                    transparent_background,
                    card_color_init,
                    card_opacity_init,
                    show_debug_overlay,
                    cycle_on_open,
                    initial_previews,
                    icons_for_init,
                )
            });
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        },
    );
    let opened_handle = if let Ok(h) = handle {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] opened new picker window");
        *current.borrow_mut() = Some((h.clone(), create_origin));
        Some(h)
    } else {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] failed to open picker window");
        None
    };
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    cx.activate(true);

    if transparent_bg {
        platform::disable_window_shadow();
    }

    // Spawn background icon fetch for any apps not yet in the cache.
    // This fills icons within ~50ms instead of waiting for the next prewarm cycle.
    if let Some(wh) = opened_handle {
        let missing_apps: Vec<String> = display_windows
            .iter()
            .map(|w| w.app_name.clone())
            .filter(|name| !icons.contains_key(name))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        if !missing_apps.is_empty() {
            let windows_for_icons = display_windows;
            let icon_cache_for_fill = icon_cache;
            cx.spawn(async move |cx: &mut AsyncApp| {
                let executor = cx.background_executor().clone();
                let raw_icons = executor
                    .spawn(async move { platform::get_app_icons(&windows_for_icons) })
                    .await;
                if raw_icons.is_empty() {
                    return;
                }
                let rendered = build_icon_cache(raw_icons);
                if let Ok(mut icache) = icon_cache_for_fill.lock() {
                    for (k, v) in &rendered {
                        icache.insert(k.clone(), v.clone());
                    }
                }
                let _ = cx.update(|cx| {
                    let _ = wh.update(cx, |view, _window, cx| {
                        view.delegate.update(cx, |state, cx| {
                            for (name, img) in rendered {
                                state.icon_cache.insert(name, img);
                            }
                            cx.notify();
                        });
                    });
                });
            })
            .detach();
        }
    }

    #[cfg(target_os = "macos")]
    set_macos_accessory_policy();
}

fn resolve_card_bg(display: &DisplayConfig) -> (u32, f32) {
    let (r, g, b) = parse_hex_color(&display.card_background_color).unwrap_or((0x1a, 0x1e, 0x2a));
    let color = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    let opacity = display.card_background_opacity.clamp(0.0, 1.0);
    (color, opacity)
}

#[cfg(target_os = "macos")]
pub(crate) fn set_macos_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

