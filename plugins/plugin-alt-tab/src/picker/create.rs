use super::GatheredWindows;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::{ActionMode, AltTabConfig, LabelConfig, PreviewIconPosition};
use crate::discovery::WindowInfo;
use crate::picker::run::SharedPreviewCache;
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
    pub preview_cache: SharedPreviewCache,
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
        PickerInit::new(
            req.config,
            gathered,
            req.preview_cache.clone(),
            title,
            true,
            req.placement.monitor_size(),
        ),
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
    let layout = picker_layout(
        estimated,
        req.config.display.max_columns,
        req.placement.monitor_size(),
        req.config.display.show_hotkey_hints,
        req.config.display.card_scale,
        req.config.display.card_padding,
    );
    size(px(layout.width), px(layout.height))
}

pub(crate) struct PickerInit {
    pub(crate) picker_title: String,
    pub(crate) shown: bool,
    pub(crate) action_mode: ActionMode,
    pub(crate) label_config: LabelConfig,
    pub(crate) transparent_bg: bool,
    pub(crate) card_color: u32,
    pub(crate) card_opacity: f32,
    pub(crate) icon_position: PreviewIconPosition,
    pub(crate) show_debug_overlay: bool,
    pub(crate) show_hotkey_hints: bool,
    pub(crate) cycle_on_open: bool,
    pub(crate) max_columns: usize,
    pub(crate) card_scale: f32,
    pub(crate) card_padding: f32,
    pub(crate) layout_budget: Option<(f32, f32)>,
    pub(crate) preview_cache: SharedPreviewCache,
    pub(crate) windows: Vec<WindowInfo>,
    pub(crate) previews: PreviewMap,
    pub(crate) icons: IconMap,
}

impl PickerInit {
    fn new(
        config: &AltTabConfig,
        gathered: GatheredWindows,
        preview_cache: SharedPreviewCache,
        picker_title: String,
        shown: bool,
        layout_budget: Option<(f32, f32)>,
    ) -> Self {
        let (card_color, card_opacity) = super::resolve_card_bg(&config.display);
        Self {
            picker_title,
            shown,
            action_mode: config.action_mode.clone(),
            label_config: config.label.clone(),
            transparent_bg: config.display.transparent_background,
            card_color,
            card_opacity,
            icon_position: config.display.icon_position,
            show_debug_overlay: config.display.show_debug_overlay,
            show_hotkey_hints: config.display.show_hotkey_hints,
            cycle_on_open: config.open_behavior == crate::config::OpenBehavior::CycleOnce,
            max_columns: config.display.max_columns,
            card_scale: config.display.card_scale,
            card_padding: config.display.card_padding,
            layout_budget,
            preview_cache,
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
    preview_cache: SharedPreviewCache,
    windows: &[WindowInfo],
    cx: &mut App,
) {
    let _reason = qol_gpui::popup_window::reason_scope("boot");
    let layout = super::reuse::compute_layout(&super::reuse::LayoutInput { placement }, cx);
    let target = placement.target();
    if current.borrow().existing(target).is_some() {
        return;
    }
    let title = super::platform::picker_window_title(target);
    let gathered = GatheredWindows {
        windows: windows.to_vec(),
        previews: PreviewMap::new(),
        icons: IconMap::new(),
    };
    let init = PickerInit::new(
        config,
        gathered,
        preview_cache,
        title.clone(),
        false,
        placement.monitor_size(),
    );
    let bounds = layout.bounds;
    let Some(handle) = open_picker_window(bounds, title.clone(), init, false, cx) else {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/boot] pre-create failed; falling back to on-demand creation");
        return;
    };
    current.borrow_mut().insert(target, handle);
    qol_gpui::ghost::hide_invisible(&title);
    super::platform::configure_picker_window(&title);
    let _ = handle.update(cx, |_view, window, _cx| {
        super::platform::sync_picker_window_layout(&title, window, bounds.origin, bounds.size)
    });
    super::platform::disable_window_shadow(&title);
    let keys: Vec<_> = current
        .borrow()
        .iter()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    qol_gpui::ghost::reconcile_active(&keys, super::platform::picker_window_title);
    PICKER_VISIBLE.store(false, Ordering::Relaxed);
    qol_runtime::probe!("PICKER_READY", "title={title}");
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
        super::platform::show_picker_window(&self.title, std::slice::from_ref(&self.title));
        cx.activate(true);
        super::probe_app_active_after_frame(handle, cx);
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
