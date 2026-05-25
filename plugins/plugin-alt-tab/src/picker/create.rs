use super::GatheredWindows;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::{ActionMode, AltTabConfig, LabelConfig};
use crate::discovery::WindowInfo;
use crate::picker::run::SharedPreviewCache;
use crate::shared::layout::*;
use crate::{IconMap, PickerWindowState, PreviewMap, SharedIconCache};
use gpui::*;
use qol_plugin_api::window::{MonitorKey, PopupPlacement};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub(crate) const PICKER_WINDOW_TITLE: &str = "qol-alt-tab-picker";

pub(super) struct CreateRequest<'a> {
    pub config: &'a AltTabConfig,
    pub placement: PopupPlacement,
    pub last_window_count: Arc<AtomicUsize>,
    pub icon_cache: SharedIconCache,
    pub preview_cache: SharedPreviewCache,
    pub current: &'a PickerWindowState,
    pub placement_dirty: &'a AtomicBool,
}

pub(super) fn create_new(req: &CreateRequest, gathered: GatheredWindows, cx: &mut App) {
    let layout = compute_create_layout(req, &gathered, cx);
    let post = PostCreateData::new(req.config, &gathered);
    let handle = open_picker_window(
        layout.bounds,
        PickerInit::new(req.config, gathered, Some(layout.target)),
        true,
        cx,
    );
    let Some(handle) = handle else {
        return on_open_failure();
    };
    let target = req.placement.target();
    req.current.borrow_mut().insert(target, handle);
    req.placement_dirty.store(false, Ordering::Release);
    post.finalize(
        handle,
        req.icon_cache.clone(),
        req.preview_cache.clone(),
        cx,
    );
}

struct CreateLayout {
    bounds: Bounds<Pixels>,
    target: MonitorKey,
}

fn compute_create_layout(
    req: &CreateRequest,
    gathered: &GatheredWindows,
    cx: &mut App,
) -> CreateLayout {
    let size = estimate_picker_size(req, gathered);
    let bounds = req.placement.centered_bounds(size, cx);
    let target = MonitorKey::from_bounds(&bounds);
    CreateLayout { bounds, target }
}

fn estimate_picker_size(req: &CreateRequest, gathered: &GatheredWindows) -> Size<Pixels> {
    let target = gathered.windows.len().max(1);
    let estimated = target
        .max(req.last_window_count.load(Ordering::Relaxed))
        .max(1);
    let (w, h) = picker_dimensions(
        estimated,
        req.config.display.max_columns,
        req.placement.monitor_size(),
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
    pub(crate) applied_layout: Option<MonitorKey>,
}

impl PickerInit {
    fn new(
        config: &AltTabConfig,
        gathered: GatheredWindows,
        applied_layout: Option<MonitorKey>,
    ) -> Self {
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
            applied_layout,
        }
    }

    /// Seed the offscreen pre-warmed picker with a synthetic 7-card grid so GPUI
    /// compiles its shaders, allocates atlas tiles for card + label + icon primitives,
    /// and warms text shaping caches before the user's first real Alt+Tab.
    /// The window is alpha=0 offscreen during this paint, so the placeholder content
    /// is never visible. First real show overwrites the delegate with live windows.
    pub(crate) fn warmup_seed(config: &AltTabConfig) -> Self {
        const WARMUP_CARDS: usize = 7;
        let mut windows = Vec::with_capacity(WARMUP_CARDS);
        let mut icons = IconMap::new();
        for i in 0..WARMUP_CARDS {
            let app_name = format!("__warmup_{i}");
            windows.push(WindowInfo {
                id: 0,
                title: "warmup".to_string(),
                app_name: app_name.clone(),
                preview_path: None,
                icon: None,
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 120.0,
                is_minimized: false,
            });
            let pixels = vec![0u8; 4 * 4 * 4]; // 4x4 RGBA, transparent
            if let Some(buf) =
                image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(4, 4, pixels)
            {
                icons.insert(
                    app_name,
                    Arc::new(gpui::RenderImage::new(smallvec::smallvec![
                        image::Frame::new(buf)
                    ])),
                );
            }
        }
        Self::new(
            config,
            GatheredWindows {
                windows,
                previews: PreviewMap::new(),
                icons,
            },
            None,
        )
    }

    pub(crate) fn into_app(self, window: &mut Window, cx: &mut Context<AltTabApp>) -> AltTabApp {
        AltTabApp::new(self, window, cx)
    }
}

fn open_picker_window(
    bounds: Bounds<Pixels>,
    init: PickerInit,
    activate: bool,
    cx: &mut App,
) -> Option<WindowHandle<AltTabApp>> {
    let opts = picker_window_options(bounds, init.transparent_bg, activate);
    cx.open_window(opts, move |window, cx| {
        window.set_window_title(PICKER_WINDOW_TITLE);
        let view = cx.new(|cx| init.into_app(window, cx));
        if activate {
            window.focus(&view.focus_handle(cx));
            window.activate_window();
        }
        view
    })
    .ok()
}

fn picker_window_options(bounds: Bounds<Pixels>, transparent: bool, focus: bool) -> WindowOptions {
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
        focus,
        window_background: bg,
        ..Default::default()
    }
}

fn on_open_failure() {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] failed to open picker window");
    PICKER_VISIBLE.store(false, Ordering::Relaxed);
}

/// Pre-create an offscreen picker window at daemon boot and register it under the
/// `BOOTSTRAP_KEY` sentinel so subsequent opens reuse one GPUI window instead of paying
/// platform window creation cost on the hotkey path.
pub(crate) fn pre_create_offscreen(
    config: &AltTabConfig,
    current: &PickerWindowState,
    cx: &mut App,
) {
    let init = PickerInit::warmup_seed(config);
    let bounds = offscreen_bounds();
    let Some(handle) = open_picker_window(bounds, init, false, cx) else {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/boot] pre-create failed — falling back to on-demand creation");
        return;
    };
    current.borrow_mut().insert(super::BOOTSTRAP_KEY, handle);
    super::platform::hide_picker_offscreen();
    // Resetting PICKER_VISIBLE keeps the pre-created window out of dispatch_show / cache
    // gates even though its WindowHandle is now permanently registered.
    PICKER_VISIBLE.store(false, Ordering::Relaxed);
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/boot] pre-created picker window (hidden offscreen, warmup-seeded)");
}

fn offscreen_bounds() -> Bounds<Pixels> {
    let (x, y) = super::platform::offscreen_origin();
    Bounds {
        origin: point(px(x as f32), px(y as f32)),
        size: size(px(720.0), px(320.0)),
    }
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

    fn finalize(
        self,
        handle: WindowHandle<AltTabApp>,
        icon_cache: SharedIconCache,
        preview_cache: SharedPreviewCache,
        cx: &mut App,
    ) {
        PICKER_VISIBLE.store(true, Ordering::Relaxed);
        cx.activate(true);
        if self.transparent_bg {
            super::platform::disable_window_shadow();
        }
        let _ = handle.update(cx, |view, _window, cx| {
            view.ensure_live_preview(cx);
        });
        let icon_req = super::IconFillRequest {
            handle,
            windows: self.windows.clone(),
            icon_cache,
        };
        super::spawn_icon_fill(icon_req, &self.icons, cx);
        let preview_req = super::PreviewFillRequest {
            handle,
            windows: self.windows,
            preview_cache,
        };
        super::spawn_preview_fill(preview_req, cx);
        super::platform::set_accessory_policy();
    }
}
