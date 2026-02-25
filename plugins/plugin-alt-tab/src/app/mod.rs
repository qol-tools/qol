pub(crate) mod alt_poll;
mod input;
mod live_preview;
mod render;

use crate::config::{ActionMode, LabelConfig};
use crate::delegate::WindowDelegate;
use crate::layout::{picker_height_for, rendered_column_count};
use crate::platform;
use crate::platform::WindowInfo;
use crate::window_source::load_windows_with_previews;
use gpui::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub(crate) static PICKER_VISIBLE: AtomicBool = AtomicBool::new(false);

pub(crate) struct AltTabApp {
    pub(crate) delegate: Entity<WindowDelegate>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) action_mode: ActionMode,
    pub(crate) alt_was_held: bool,
    pub(crate) _alt_poll_task: Option<Task<()>>,
    _live_preview_task: Option<Task<()>>,
}

impl AltTabApp {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        last_window_count: Arc<AtomicUsize>,
        window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>>,
        action_mode: ActionMode,
        initial_windows: Vec<WindowInfo>,
        label_config: LabelConfig,
        cycle_on_open: bool,
        initial_previews: HashMap<u32, Arc<RenderImage>>,
        icon_cache: HashMap<String, Arc<RenderImage>>,
    ) -> Self {
        let has_cached_windows = !initial_windows.is_empty();
        let win_delegate =
            WindowDelegate::new_with_previews(initial_windows.clone(), label_config, initial_previews, icon_cache);
        let delegate = cx.new(|_cx| win_delegate);

        if cycle_on_open && initial_windows.len() >= 2 {
            delegate.update(cx, |s, _| s.select_next());
        }

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let mut async_window = window.to_async(cx);
        let gpui_window_handle = async_window.window_handle();

        // Register the focus out subscription for Sticky mode.
        let focus_handle_for_sub = focus_handle.clone();
        let _focus_out_subscription = cx.on_focus_out(
            &focus_handle_for_sub,
            window,
            |this, _event, window, _cx| {
                if this.action_mode != ActionMode::HoldToSwitch {
                    PICKER_VISIBLE.store(false, Ordering::Relaxed);
                    platform::dismiss_picker(window);
                }
            },
        );

        let delegate_clone = delegate.clone();
        cx.spawn(
            move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
                let cx = cx.clone();
                async move {
                    let executor = cx.background_executor().clone();
                    if has_cached_windows {
                        return;
                    }
                    let cached = window_cache
                        .lock()
                        .map(|cache| cache.clone())
                        .unwrap_or_default();
                    let windows = load_windows_with_previews(&executor, cached, false).await;
                    if let Ok(mut cache) = window_cache.lock() {
                        *cache = windows.clone();
                    }
                    let ids: Vec<u32> = windows.iter().map(|w| w.id).collect();
                    last_window_count.store(ids.len().max(1), Ordering::Relaxed);

                    let _ = cx.update(|app_cx| {
                        let _ = delegate_clone.update(app_cx, |state, cx| {
                            state.set_windows(windows.clone(), false);
                            cx.notify();
                        });
                        let _ = this.update(app_cx, |_, cx: &mut Context<AltTabApp>| {
                            cx.notify();
                        });
                    });
                    let total = ids.len().max(1);
                    let _ = async_window.update(|window, _app_cx| {
                        let cols = rendered_column_count(window, total);
                        let next_height = picker_height_for(total, cols).clamp(320.0, 980.0);
                        let current = window.window_bounds().get_bounds().size;
                        if (current.height.to_f64() - next_height as f64).abs() >= 1.0 {
                            window.resize(size(current.width, px(next_height)));
                        }
                    });
                }
            },
        )
        .detach();

        let live_preview_task = live_preview::spawn(delegate.clone(), cx);

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] AltTabApp::new: action_mode={:?}, alt_was_held=true (assumed)",
            action_mode
        );

        let mut app = Self {
            delegate,
            focus_handle,
            action_mode: action_mode.clone(),
            alt_was_held: true,
            _alt_poll_task: None,
            _live_preview_task: Some(live_preview_task),
        };

        if action_mode == ActionMode::HoldToSwitch {
            alt_poll::start(&mut app, gpui_window_handle, cx);
        }

        app
    }

    pub(crate) fn apply_cached_windows(
        &mut self,
        windows: Vec<WindowInfo>,
        reset_selection: bool,
        previews: HashMap<u32, Arc<RenderImage>>,
        icons: HashMap<String, Arc<RenderImage>>,
        cx: &mut Context<Self>,
    ) {
        self.delegate.update(cx, |state, cx| {
            state.set_windows(windows, reset_selection);
            if !previews.is_empty() {
                state.live_previews = previews;
            }
            if !icons.is_empty() {
                state.icon_cache = icons;
            }
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn start_alt_poll(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        alt_poll::start(self, window_handle, cx);
    }
}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
