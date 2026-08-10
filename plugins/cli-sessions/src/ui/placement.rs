use gpui::{point, px, Bounds, Pixels, Size};

pub const CORNER_MARGIN: f32 = 16.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub fn parse(s: &str) -> Corner {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "top-left" => Corner::TopLeft,
            "bottom-left" => Corner::BottomLeft,
            "bottom-right" => Corner::BottomRight,
            _ => Corner::TopRight,
        }
    }
}

pub fn corner_bounds(
    monitor: Bounds<Pixels>,
    win: Size<Pixels>,
    corner: Corner,
    margin: f32,
) -> Bounds<Pixels> {
    let m = px(margin);
    let left = monitor.origin.x + m;
    let top = monitor.origin.y + m;
    let right = monitor.origin.x + monitor.size.width - win.width - m;
    let bottom = monitor.origin.y + monitor.size.height - win.height - m;
    let origin = match corner {
        Corner::TopLeft => point(left, top),
        Corner::TopRight => point(right, top),
        Corner::BottomLeft => point(left, bottom),
        Corner::BottomRight => point(right, bottom),
    };
    Bounds::new(origin, win)
}

pub fn clamp_bounds(
    monitor: Bounds<Pixels>,
    bounds: Bounds<Pixels>,
    margin: f32,
) -> Bounds<Pixels> {
    let m = px(margin);
    let min_x = monitor.origin.x + m;
    let min_y = monitor.origin.y + m;
    let max_x = monitor.origin.x + monitor.size.width - bounds.size.width - m;
    let max_y = monitor.origin.y + monitor.size.height - bounds.size.height - m;
    let x = bounds.origin.x.min(max_x).max(min_x);
    let y = bounds.origin.y.min(max_y).max(min_y);
    Bounds::new(point(x, y), bounds.size)
}
