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
            live_previews,
            icon_cache,
        }
    }

    pub(crate) fn set_windows(&mut self, windows: Vec<WindowInfo>, reset_selection: bool) {
        self.windows = windows;
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
}
