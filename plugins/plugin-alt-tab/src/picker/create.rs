use super::GatheredWindows;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::{ActionMode, AltTabConfig, LabelConfig};
use crate::discovery::WindowInfo;
use crate::shared::layout::*;
use crate::{IconMap, PickerWindowState, PreviewMap, SharedIconCache};
use gpui::*;
use qol_plugin_api::monitor::{ActiveMonitor, MonitorTracker};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub(super) struct CreateRequest<'a> {
    pub config: &'a AltTabConfig,
    pub tracker: &'a MonitorTracker,
    pub last_window_count: Arc<AtomicUsize>,
    pub icon_cache: SharedIconCache,
    pub current: &'a PickerWindowState,
}

pub(super) fn create_new(req: &CreateRequest, gathered: GatheredWindows, cx: &mut App) {
    let layout = compute_create_layout(req, &gathered, cx);
    let post = PostCreateData::new(req.config, &gathered);
    let handle = open_picker_window(layout.bounds, PickerInit::new(req.config, gathered), cx);
    let Some(handle) = handle else {
        return on_open_failure();
    };
    let target = req
        .tracker
        .snapshot()
        .map(|m| qol_plugin_api::window::MonitorKey::from_bounds(&m.0.bounds()))
        .unwrap_or(qol_plugin_api::window::MonitorKey {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        });
    req.current.borrow_mut().insert(target, handle);
    post.finalize(handle, req.icon_cache.clone(), cx);
}

struct CreateLayout {
    bounds: Bounds<Pixels>,
}

fn compute_create_layout(
    req: &CreateRequest,
    gathered: &GatheredWindows,
    cx: &mut App,
) -> CreateLayout {
    let monitor = req.tracker.snapshot().map(|(m, _)| m);
    let size = estimate_picker_size(req, gathered, &monitor);
    let bounds = super::reuse::centered_bounds(&monitor, size, cx);
    CreateLayout { bounds }
}

fn estimate_picker_size(
    req: &CreateRequest,
    gathered: &GatheredWindows,
    monitor: &Option<ActiveMonitor>,
) -> Size<Pixels> {
    let target = gathered.windows.len().max(1);
    let estimated = target
        .max(req.last_window_count.load(Ordering::Relaxed))
        .max(1);
    let monitor_size = monitor.as_ref().map(|m| m.size());
    let (w, h) = picker_dimensions(
        estimated,
        req.config.display.max_columns,
        monitor_size,
        req.config.display.show_hotkey_hints,
    );
    size(px(w), px(h))
}

pub(crate) struct PickerInit {
    pub(crate) action_mode: ActionMode,
    pub(crate) label_config: LabelConfig,
    pub(crate) transparent_bg: bool,
    pub(crate) card_color: u32,
    pub(crate) card_opacity: f32,
    pub(crate) show_debug_overlay: bool,
    pub(crate) show_hotkey_hints: bool,
    pub(crate) cycle_on_open: bool,
    pub(crate) windows: Vec<WindowInfo>,
    pub(crate) previews: PreviewMap,
    pub(crate) icons: IconMap,
}

impl PickerInit {
    fn new(config: &AltTabConfig, gathered: GatheredWindows) -> Self {
        let (card_color, card_opacity) = super::resolve_card_bg(&config.display);
        Self {
            action_mode: config.action_mode.clone(),
            label_config: config.label.clone(),
            transparent_bg: config.display.transparent_background,
            card_color,
            card_opacity,
            show_debug_overlay: config.display.show_debug_overlay,
            show_hotkey_hints: config.display.show_hotkey_hints,
            cycle_on_open: config.open_behavior == crate::config::OpenBehavior::CycleOnce,
            windows: gathered.windows,
            previews: gathered.previews,
            icons: gathered.icons,
        }
    }

    pub(crate) fn into_app(self, window: &mut Window, cx: &mut Context<AltTabApp>) -> AltTabApp {
        AltTabApp::new(self, window, cx)
    }
}

fn open_picker_window(
    bounds: Bounds<Pixels>,
    init: PickerInit,
    cx: &mut App,
) -> Option<WindowHandle<AltTabApp>> {
    let opts = picker_window_options(bounds, init.transparent_bg);
    cx.open_window(opts, move |window, cx| {
        window.set_window_title("qol-alt-tab-picker");
        let view = cx.new(|cx| init.into_app(window, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    })
    .ok()
}

fn picker_window_options(bounds: Bounds<Pixels>, transparent: bool) -> WindowOptions {
    let bg = if transparent {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    };
    let decor = if transparent {
        WindowDecorations::Server
    } else {
        WindowDecorations::Client
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(decor),
        kind: super::platform::picker_window_kind(),
        focus: true,
        window_background: bg,
        ..Default::default()
    }
}

fn on_open_failure() {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] failed to open picker window");
    PICKER_VISIBLE.store(false, Ordering::Relaxed);
}

struct PostCreateData {
    transparent_bg: bool,
    windows: Vec<WindowInfo>,
    icons: IconMap,
}

impl PostCreateData {
    fn new(config: &AltTabConfig, gathered: &GatheredWindows) -> Self {
        Self {
            transparent_bg: config.display.transparent_background,
            windows: gathered.windows.clone(),
            icons: gathered.icons.clone(),
        }
    }

    fn finalize(self, handle: WindowHandle<AltTabApp>, icon_cache: SharedIconCache, cx: &mut App) {
        PICKER_VISIBLE.store(true, Ordering::Relaxed);
        cx.activate(true);
        if self.transparent_bg {
            super::platform::disable_window_shadow();
        }
        let icon_req = super::IconFillRequest {
            handle,
            windows: self.windows,
            icon_cache,
        };
        super::spawn_icon_fill(icon_req, &self.icons, cx);
        super::platform::set_accessory_policy();
    }
}
