use gpui::{
    Bounds, DisplayId, Pixels, SharedString, WindowBackgroundAppearance, WindowDecorations,
    WindowKind, WindowOptions,
};

use crate::platform::{ghost_window_decorations, ghost_window_kind};
use crate::window::WindowPlacement;

pub struct PopupWindowOptions {
    options: WindowOptions,
}

impl PopupWindowOptions {
    pub fn new() -> Self {
        Self {
            options: WindowOptions {
                titlebar: None,
                kind: ghost_window_kind(),
                window_decorations: Some(ghost_window_decorations(false)),
                window_background: WindowBackgroundAppearance::Transparent,
                is_movable: true,
                focus: false,
                show: true,
                ..WindowOptions::default()
            },
        }
    }

    pub fn from_placement(placement: &WindowPlacement) -> Self {
        Self::new()
            .bounds(placement.bounds)
            .display_id(placement.display_id)
    }

    pub fn bounds(mut self, bounds: Bounds<Pixels>) -> Self {
        self.options.window_bounds = Some(gpui::WindowBounds::Windowed(bounds));
        self
    }

    pub fn display_id(mut self, display_id: Option<DisplayId>) -> Self {
        self.options.display_id = display_id;
        self
    }

    pub fn kind(mut self, kind: WindowKind) -> Self {
        self.options.kind = kind;
        self
    }

    pub fn decorations(mut self, decorations: WindowDecorations) -> Self {
        self.options.window_decorations = Some(decorations);
        self
    }

    pub fn background(mut self, background: WindowBackgroundAppearance) -> Self {
        self.options.window_background = background;
        self
    }

    pub fn focus(mut self, focus: bool) -> Self {
        self.options.focus = focus;
        self
    }

    pub fn show(mut self, show: bool) -> Self {
        self.options.show = show;
        self
    }

    pub fn movable(mut self, movable: bool) -> Self {
        self.options.is_movable = movable;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.options.is_resizable = resizable;
        self
    }

    pub fn minimizable(mut self, minimizable: bool) -> Self {
        self.options.is_minimizable = minimizable;
        self
    }

    pub fn app_id(mut self, app_id: impl Into<SharedString>) -> Self {
        self.options.app_id = Some(app_id.into().to_string());
        self
    }

    pub fn build(self) -> WindowOptions {
        self.options
    }
}

impl Default for PopupWindowOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        point, px, size, Bounds, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
        WindowKind,
    };

    use super::PopupWindowOptions;
    use crate::platform::{ghost_window_decorations, ghost_window_kind};
    use crate::window::{MonitorKey, WindowPlacement};

    #[test]
    fn new_matches_the_ghost_popup_recipe() {
        let options = PopupWindowOptions::new().build();
        assert!(options.window_bounds.is_none());
        assert!(options.titlebar.is_none());
        assert_eq!(options.kind, ghost_window_kind());
        assert_eq!(
            options.window_decorations,
            Some(ghost_window_decorations(false))
        );
        assert_eq!(
            options.window_background,
            WindowBackgroundAppearance::Transparent
        );
        assert!(options.is_movable);
        assert!(!options.focus);
        assert!(options.show);
        assert!(options.display_id.is_none());
        assert!(options.app_id.is_none());
        assert!(options.is_resizable);
        assert!(options.is_minimizable);
        assert!(options.window_min_size.is_none());
        assert!(options.tabbing_identifier.is_none());
    }

    #[test]
    fn builder_overrides_each_field() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(300.0), px(200.0)));
        let options = PopupWindowOptions::new()
            .bounds(bounds)
            .kind(WindowKind::Normal)
            .decorations(WindowDecorations::Client)
            .background(WindowBackgroundAppearance::Opaque)
            .focus(true)
            .show(false)
            .movable(false)
            .resizable(false)
            .minimizable(false)
            .app_id("qol-test")
            .build();
        assert_eq!(options.window_bounds, Some(WindowBounds::Windowed(bounds)));
        assert_eq!(options.kind, WindowKind::Normal);
        assert_eq!(options.window_decorations, Some(WindowDecorations::Client));
        assert_eq!(
            options.window_background,
            WindowBackgroundAppearance::Opaque
        );
        assert!(options.focus);
        assert!(!options.show);
        assert!(!options.is_movable);
        assert!(!options.is_resizable);
        assert!(!options.is_minimizable);
        assert_eq!(options.app_id.as_deref(), Some("qol-test"));
    }

    #[test]
    fn from_placement_takes_bounds_and_display_id() {
        let bounds = Bounds::new(point(px(1.0), px(2.0)), size(px(30.0), px(40.0)));
        let placement = WindowPlacement {
            target: MonitorKey::fallback(),
            bounds,
            display_id: None,
        };
        let options = PopupWindowOptions::from_placement(&placement).build();
        assert_eq!(options.window_bounds, Some(WindowBounds::Windowed(bounds)));
        assert!(options.display_id.is_none());
    }
}
