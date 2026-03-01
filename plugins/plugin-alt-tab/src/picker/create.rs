use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::AltTabConfig;
use crate::layout::*;
use crate::platform;
use crate::platform::WindowInfo;
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub(super) fn create_new(
    config: &AltTabConfig,
    display_windows: Vec<WindowInfo>,
    initial_previews: HashMap<u32, Arc<RenderImage>>,
    icons: HashMap<String, Arc<RenderImage>>,
    tracker: &MonitorTracker,
    last_window_count: Arc<AtomicUsize>,
    icon_cache: Arc<std::sync::Mutex<HashMap<String, Arc<RenderImage>>>>,
    current: &std::rc::Rc<std::cell::RefCell<Option<(WindowHandle<AltTabApp>, Point<Pixels>)>>>,
    cx: &mut App,
) {
    let target_count = display_windows.len().max(1);
    let estimated_count = target_count.max(last_window_count.load(Ordering::Relaxed)).max(1);
    let create_monitor = tracker.snapshot().map(|(m, _)| m);
    let monitor_size = create_monitor.as_ref().map(|m| m.size());
    let (win_w, win_h) = picker_dimensions(
        estimated_count, config.display.max_columns, monitor_size, config.display.show_hotkey_hints,
    );
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

    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/create] size={}x{} estimated_count={} actual_count={} cols={} hints={}",
        win_w, win_h, estimated_count, target_count,
        preferred_column_count(estimated_count, config.display.max_columns),
        config.display.show_hotkey_hints,
    );
    println!("[alt-tab] opening at {:?} with size {:?}", bounds.origin, bounds.size);

    let action_mode = config.action_mode.clone();
    let label_config = config.label.clone();
    let transparent_bg = config.display.transparent_background;
    let show_debug_overlay = config.display.show_debug_overlay;
    let show_hotkey_hints = config.display.show_hotkey_hints;
    let (card_color, card_opacity) = super::resolve_card_bg(&config.display);
    let cycle_on_open = config.open_behavior == crate::config::OpenBehavior::CycleOnce;
    let windows_for_init = display_windows.clone();
    let icons_for_init = icons.clone();

    let window_background = if transparent_bg {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    };

    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(if transparent_bg {
                WindowDecorations::Server
            } else {
                WindowDecorations::Client
            }),
            kind: platform::picker_window_kind(),
            focus: true,
            window_background,
            ..Default::default()
        },
        move |window, cx| {
            window.set_window_title("qol-alt-tab-picker");
            let view = cx.new(|cx| {
                AltTabApp::new(
                    window, cx, action_mode, windows_for_init, label_config,
                    transparent_bg, card_color, card_opacity,
                    show_debug_overlay, show_hotkey_hints, cycle_on_open,
                    initial_previews, icons_for_init,
                )
            });
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        },
    );

    let Some(handle) = handle.ok() else {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] failed to open picker window");
        PICKER_VISIBLE.store(false, Ordering::Relaxed);
        return;
    };

    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] opened new picker window");
    *current.borrow_mut() = Some((handle.clone(), create_origin));
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    cx.activate(true);

    if transparent_bg {
        platform::disable_window_shadow();
    }

    super::spawn_icon_fill(handle, display_windows, &icons, icon_cache, cx);

    #[cfg(target_os = "macos")]
    super::set_macos_accessory_policy();
}
