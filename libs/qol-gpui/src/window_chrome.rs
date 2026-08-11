use gpui::{point, px, size, App, Bounds, Pixels, Window};

use crate::monitor::MonitorTracker;
use crate::placement::{clamp_bounds, Corner, CORNER_MARGIN};
use crate::popup_window;

pub const STRIP_HEIGHT: f32 = 32.0;

pub fn strip_bounds(expanded: Bounds<Pixels>, corner: Corner, strip_height: f32) -> Bounds<Pixels> {
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => expanded.origin.y,
        Corner::BottomLeft | Corner::BottomRight => {
            expanded.origin.y + expanded.size.height - px(strip_height)
        }
    };
    Bounds::new(
        point(expanded.origin.x, y),
        size(expanded.size.width, px(strip_height)),
    )
}

pub fn reanchor_expanded(
    expanded: Bounds<Pixels>,
    strip: Bounds<Pixels>,
    corner: Corner,
) -> Bounds<Pixels> {
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => strip.origin.x,
        Corner::TopRight | Corner::BottomRight => {
            strip.origin.x + strip.size.width - expanded.size.width
        }
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => strip.origin.y,
        Corner::BottomLeft | Corner::BottomRight => {
            strip.origin.y + strip.size.height - expanded.size.height
        }
    };
    Bounds::new(point(x, y), expanded.size)
}

#[derive(Debug)]
pub struct CollapseState {
    corner: Corner,
    collapsed: bool,
    expanded: Option<Bounds<Pixels>>,
}

impl CollapseState {
    pub fn new(corner: Corner) -> Self {
        Self {
            corner,
            collapsed: false,
            expanded: None,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub fn corner(&self) -> Corner {
        self.corner
    }

    pub fn collapse(&mut self, expanded: Bounds<Pixels>) -> Bounds<Pixels> {
        self.collapsed = true;
        self.expanded = Some(expanded);
        strip_bounds(expanded, self.corner, STRIP_HEIGHT)
    }

    pub fn expand(&mut self) -> Option<Bounds<Pixels>> {
        let expanded = self.expanded.take()?;
        self.collapsed = false;
        Some(expanded)
    }
}

pub struct PanelChrome {
    title: String,
    collapse: CollapseState,
}

impl PanelChrome {
    pub fn new(title: impl Into<String>, corner: Corner) -> Self {
        Self {
            title: title.into(),
            collapse: CollapseState::new(corner),
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapse.is_collapsed()
    }

    pub fn hide(&self) -> bool {
        self.hide_with_reason("window-chrome-hide")
    }

    pub fn hide_with_reason(&self, reason: &str) -> bool {
        let _scope = popup_window::reason_scope(reason);
        popup_window::hide_window_by_title(&self.title)
    }

    pub fn collapse(&mut self, window: &mut Window) -> bool {
        let expanded = window.bounds();
        let strip = self.collapse.collapse(expanded);
        if !apply_panel_bounds(&self.title, window, strip) {
            self.collapse.expand();
            return false;
        }
        true
    }

    pub fn expand(&mut self, window: &mut Window, cx: &mut App) -> bool {
        let Some(expanded) = self.collapse.expand() else {
            return false;
        };
        let anchored = reanchor_expanded(expanded, window.bounds(), self.collapse.corner());
        let anchored = clamp_to_monitor(anchored, cx);
        if !apply_panel_bounds(&self.title, window, anchored) {
            self.collapse.collapse(anchored);
            return false;
        }
        true
    }
}

pub fn apply_panel_bounds(title: &str, window: &mut Window, bounds: Bounds<Pixels>) -> bool {
    let locked = popup_window::set_window_fixed_size_by_title(title, bounds.size);
    let synced = popup_window::sync_window_layout(title, window, bounds.origin, bounds.size);
    locked && synced
}

fn clamp_to_monitor(bounds: Bounds<Pixels>, cx: &mut App) -> Bounds<Pixels> {
    match MonitorTracker::start(cx).snapshot_monitor() {
        Some(monitor) => clamp_bounds(monitor.bounds(), bounds, CORNER_MARGIN),
        None => bounds,
    }
}

#[cfg(test)]
mod tests {
    use super::{reanchor_expanded, strip_bounds, CollapseState};
    use crate::placement::Corner;
    use gpui::{point, px, size, Bounds};

    fn bounds(origin: (f32, f32), dims: (f32, f32)) -> Bounds<gpui::Pixels> {
        Bounds::new(
            point(px(origin.0), px(origin.1)),
            size(px(dims.0), px(dims.1)),
        )
    }

    #[test]
    fn strip_keeps_the_top_edge_for_top_corners() {
        let expanded = bounds((100.0, 200.0), (800.0, 600.0));
        let strip = strip_bounds(expanded, Corner::TopLeft, 32.0);
        assert_eq!(strip.origin.y, expanded.origin.y);
        assert_eq!(strip.size.height, px(32.0));
        assert_eq!(strip.size.width, expanded.size.width);
    }

    #[test]
    fn strip_keeps_the_bottom_edge_for_bottom_corners() {
        let expanded = bounds((100.0, 200.0), (800.0, 600.0));
        let strip = strip_bounds(expanded, Corner::BottomRight, 32.0);
        assert_eq!(
            strip.origin.y,
            expanded.origin.y + expanded.size.height - px(32.0)
        );
        assert_eq!(strip.size.height, px(32.0));
    }

    #[test]
    fn expand_restores_the_original_bounds() {
        let mut state = CollapseState::new(Corner::TopLeft);
        let expanded = bounds((100.0, 200.0), (800.0, 600.0));
        let strip = state.collapse(expanded);
        assert!(state.is_collapsed());
        assert_eq!(state.expand(), Some(expanded));
        assert!(!state.is_collapsed());
        assert_eq!(state.expand(), None, "a second expand is a no-op");
        let _ = strip;
    }

    #[test]
    fn reanchor_moves_the_expanded_window_onto_a_moved_strip() {
        let expanded = bounds((100.0, 200.0), (800.0, 600.0));
        let moved_strip = bounds((400.0, 500.0), (800.0, 32.0));
        let anchored = reanchor_expanded(expanded, moved_strip, Corner::TopLeft);
        assert_eq!(anchored.origin.x, moved_strip.origin.x);
        assert_eq!(anchored.origin.y, moved_strip.origin.y);
        assert_eq!(anchored.size, expanded.size);
    }
}
