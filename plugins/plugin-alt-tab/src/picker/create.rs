use super::GatheredWindows;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::{ActionMode, AltTabConfig, LabelConfig};
use crate::discovery::WindowInfo;
use crate::shared::layout::*;
use crate::{IconMap, PickerWindowState, PreviewMap, SharedIconCache};
use gpui::*;
use qol_gpui::window::PopupPlacement;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub(crate) const PICKER_WINDOW_TITLE: &str = "qol-alt-tab-picker";

pub(super) struct CreateRequest<'a> {
    pub config: &'a AltTabConfig,
    pub placement: PopupPlacement,
    pub last_window_count: Arc<AtomicUsize>,
    pub icon_cache: SharedIconCache,
    pub current: &'a PickerWindowState,
    pub has_shown_once: Arc<AtomicBool>,
}

pub(super) fn create_new(req: &CreateRequest, gathered: GatheredWindows, cx: &mut App) {
    let bounds = compute_create_bounds(req, &gathered, cx);
    let target = req.placement.target();
    let title = super::platform::picker_window_title(target);
    let post = PostCreateData::new(req.config, &gathered, title.clone());
    let handle = open_picker_window(
        bounds,
        title.clone(),
        PickerInit::new(req.config, gathered, title),
        true,
        cx,
    );
    let Some(handle) = handle else {
        return on_open_failure();
    };
    req.current.borrow_mut().insert(target, handle);
    post.finalize(
        handle,
        req.icon_cache.clone(),
        req.has_shown_once.clone(),
        cx,
    );
}

fn compute_create_bounds(
    req: &CreateRequest,
    gathered: &GatheredWindows,
    cx: &mut App,
) -> Bounds<Pixels> {
    let size = estimate_picker_size(req, gathered);
    req.placement.centered_bounds(size, cx)
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
    pub(crate) picker_title: String,
    pub(crate) action_mode: ActionMode,
    pub(crate) label_config: LabelConfig,
    pub(crate) transparent_bg: bool,
    pub(crate) card_color: u32,
    pub(crate) card_opacity: f32,
    pub(crate) show_debug_overlay: bool,
    pub(crate) show_hotkey_hints: bool,
    pub(crate) cycle_on_open: bool,
    pub(crate) max_columns: usize,
    pub(crate) windows: Vec<WindowInfo>,
    pub(crate) previews: PreviewMap,
    pub(crate) icons: IconMap,
}

impl PickerInit {
    fn new(config: &AltTabConfig, gathered: GatheredWindows, picker_title: String) -> Self {
        let (card_color, card_opacity) = super::resolve_card_bg(&config.display);
        Self {
            picker_title,
            action_mode: config.action_mode.clone(),
            label_config: config.label.clone(),
            transparent_bg: config.display.transparent_background,
            card_color,
            card_opacity,
            show_debug_overlay: config.display.show_debug_overlay,
            show_hotkey_hints: config.display.show_hotkey_hints,
            cycle_on_open: config.open_behavior == crate::config::OpenBehavior::CycleOnce,
            max_columns: config.display.max_columns,
            windows: gathered.windows,
            previews: gathered.previews,
            icons: gathered.icons,
        }
    }

    pub(crate) fn warmup_seed(config: &AltTabConfig, picker_title: String) -> Self {
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
            let pixels = vec![0u8; 4 * 4 * 4];
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
            picker_title,
        )
    }

    pub(crate) fn into_app(self, window: &mut Window, cx: &mut Context<AltTabApp>) -> AltTabApp {
        AltTabApp::new(self, window, cx)
    }
}

fn open_picker_window(
    bounds: Bounds<Pixels>,
    title: String,
    init: PickerInit,
    activate: bool,
    cx: &mut App,
) -> Option<WindowHandle<AltTabApp>> {
    let opts = picker_window_options(bounds, init.transparent_bg, activate);
    let window_title = title.clone();
    cx.open_window(opts, move |window, cx| {
        window.set_window_title(&window_title);
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
    let decor = super::platform::picker_window_decorations(transparent);
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
    if let Ok(mut lock) = crate::app::ACTIVE_PICKER_MONITOR.lock() {
        *lock = None;
    }
}

pub(crate) fn pre_create_ghost(
    config: &AltTabConfig,
    current: &PickerWindowState,
    placement: &PopupPlacement,
    cx: &mut App,
) {
    let layout = super::reuse::compute_layout(&super::reuse::LayoutInput { placement }, cx);
    let target = placement.target();
    if current.borrow().existing(target).is_some() {
        return;
    }
    let title = super::platform::picker_window_title(target);
    let init = PickerInit::warmup_seed(config, title.clone());
    let bounds = layout.bounds;
    let Some(handle) = open_picker_window(bounds, title.clone(), init, false, cx) else {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/boot] pre-create failed; falling back to on-demand creation");
        return;
    };
    current.borrow_mut().insert(target, handle);
    super::platform::configure_picker_window(&title);
    let _ = handle.update(cx, |_view, window, _cx| {
        super::platform::sync_picker_window_layout(&title, window, bounds.origin, bounds.size)
    });
    super::platform::disable_window_shadow(&title);
    #[cfg(target_os = "linux")]
    {
        qol_gpui::popup_window::hide_window_invisible(&title);
    }
    #[cfg(not(target_os = "linux"))]
    {
        super::platform::hide_picker(&title);
    }
    let keys: Vec<_> = current
        .borrow()
        .iter()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    qol_gpui::ghost::reconcile_active(&keys, super::platform::picker_window_title);
    PICKER_VISIBLE.store(false, Ordering::Relaxed);
    qol_runtime::probe!("PICKER_STALE", "title={title}");
    qol_gpui::popup_window::dump_ghost_windows(&format!("pre-create title={title}"));
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/boot] pre-created picker window target={:?} title={:?}",
        target, title
    );
}

struct PostCreateData {
    title: String,
    transparent_bg: bool,
    windows: Vec<WindowInfo>,
    icons: IconMap,
}

impl PostCreateData {
    fn new(config: &AltTabConfig, gathered: &GatheredWindows, title: String) -> Self {
        Self {
            title,
            transparent_bg: config.display.transparent_background,
            windows: gathered.windows.clone(),
            icons: gathered.icons.clone(),
        }
    }

    fn finalize(
        self,
        handle: WindowHandle<AltTabApp>,
        icon_cache: SharedIconCache,
        has_shown_once: Arc<AtomicBool>,
        cx: &mut App,
    ) {
        PICKER_VISIBLE.store(true, Ordering::Relaxed);
        has_shown_once.store(true, Ordering::Release);
        qol_runtime::probe!("PICKER_READY", "title={}", self.title);
        cx.activate(true);
        if self.transparent_bg {
            super::platform::disable_window_shadow(&self.title);
        }
        super::platform::configure_picker_window(&self.title);
        let _ = handle.update(cx, |view, _window, cx| {
            view.ensure_live_preview(cx);
        });
        let icon_req = super::IconFillRequest {
            handle,
            windows: self.windows.clone(),
            icon_cache,
        };
        super::spawn_icon_fill(icon_req, &self.icons, cx);
        super::platform::set_accessory_policy();
    }
}
