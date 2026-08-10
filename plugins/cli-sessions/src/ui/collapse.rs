use gpui::{point, px, size, Bounds, Pixels};

use crate::ui::placement::Corner;

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
