mod activation;
mod selection;


use crate::config::LabelConfig;
use crate::platform::WindowInfo;
use gpui::RenderImage;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct WindowDelegate {
    pub(crate) windows: Vec<WindowInfo>,
    pub(crate) selected_index: Option<usize>,
    pub(crate) label_config: LabelConfig,
    pub(crate) transparent_background: bool,
    pub(crate) card_bg_color: u32,
    pub(crate) card_bg_opacity: f32,
    pub(crate) show_debug_overlay: bool,
    pub(crate) show_hotkey_hints: bool,
    pub(crate) live_previews: HashMap<u32, Arc<RenderImage>>,
    pub(crate) icon_cache: HashMap<String, Arc<RenderImage>>,
}

impl WindowDelegate {
    pub(crate) fn new_with_previews(
        windows: Vec<WindowInfo>,
        label_config: LabelConfig,
        transparent_background: bool,
        card_bg_color: u32,
        card_bg_opacity: f32,
        show_debug_overlay: bool,
        show_hotkey_hints: bool,
        live_previews: HashMap<u32, Arc<RenderImage>>,
        icon_cache: HashMap<String, Arc<RenderImage>>,
    ) -> Self {
        let selected_index = if windows.is_empty() { None } else { Some(0) };
        Self {
            windows,
            selected_index,
            label_config,
            transparent_background,
            card_bg_color,
            card_bg_opacity,
            show_debug_overlay,
            show_hotkey_hints,
            live_previews,
            icon_cache,
        }
    }

    pub(crate) fn set_windows(&mut self, windows: Vec<WindowInfo>, reset_selection: bool) {
        self.windows = windows;
        let active_ids: std::collections::HashSet<u32> =
            self.windows.iter().map(|w| w.id).collect();
        self.live_previews.retain(|id, _| active_ids.contains(id));
        if self.windows.is_empty() {
            self.selected_index = None;
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/select] set_windows reset={} next=None total=0",
                reset_selection
            );
            return;
        }

        if reset_selection {
            self.selected_index = Some(0);
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/select] set_windows reset={} next=Some(0) total={}",
                reset_selection,
                self.windows.len()
            );
            return;
        }

        let selected_row = self.selected_index.unwrap_or(0);
        self.selected_index = Some(selected_row.min(self.windows.len() - 1));
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/select] set_windows reset={} next={:?} total={}",
            reset_selection,
            self.selected_index,
            self.windows.len()
        );
    }

    /// Optimistically remove a single window (e.g. after close).
    pub(crate) fn remove_window(&mut self, window_id: u32) {
        let remaining: Vec<_> = self.windows.iter().filter(|w| w.id != window_id).cloned().collect();
        self.set_windows(remaining, false);
    }

    /// Optimistically remove all windows belonging to an app (e.g. after quit).
    pub(crate) fn remove_app_windows(&mut self, app_name: &str) {
        let remaining: Vec<_> = self.windows.iter().filter(|w| w.app_name != app_name).cloned().collect();
        self.set_windows(remaining, false);
    }

    /// Optimistically mark a window as minimized and move it to the end.
    pub(crate) fn mark_minimized(&mut self, window_id: u32) {
        let mut reordered = Vec::with_capacity(self.windows.len());
        let mut target = None;
        for w in self.windows.drain(..) {
            if w.id == window_id {
                target = Some(w);
            } else {
                reordered.push(w);
            }
        }
        if let Some(mut w) = target {
            w.is_minimized = true;
            reordered.push(w);
        }
        self.set_windows(reordered, false);
    }
}
