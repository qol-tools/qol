pub(crate) mod alt_poll;
mod input;
mod live_preview;
mod render;

use crate::config::{ActionMode, LabelConfig};
use crate::delegate::WindowDelegate;
use crate::platform;
use crate::platform::WindowInfo;
use gpui::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
        action_mode: ActionMode,
        initial_windows: Vec<WindowInfo>,
        label_config: LabelConfig,
        cycle_on_open: bool,
        initial_previews: HashMap<u32, Arc<RenderImage>>,
        icon_cache: HashMap<String, Arc<RenderImage>>,
    ) -> Self {
        let win_delegate =
            WindowDelegate::new_with_previews(initial_windows.clone(), label_config, initial_previews, icon_cache);
        let delegate = cx.new(|_cx| win_delegate);

        if cycle_on_open && initial_windows.len() >= 2 {
            delegate.update(cx, |s, _| s.select_next());
        }

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let gpui_window_handle = window.to_async(cx).window_handle();

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
