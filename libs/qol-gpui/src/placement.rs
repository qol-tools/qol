use gpui::{point, px, size, Bounds, Pixels, Point, Size};

pub const CORNER_MARGIN: f32 = 24.0;
pub const TOP_CENTER_MARGIN: f32 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Corner(Corner),
    TopCenter,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorPlacement {
    anchor: Anchor,
    margin: f32,
}

impl MonitorPlacement {
    pub const fn corner(corner: Corner, margin: f32) -> Self {
        Self {
            anchor: Anchor::Corner(corner),
            margin,
        }
    }

    pub const fn top_center(margin: f32) -> Self {
        Self {
            anchor: Anchor::TopCenter,
            margin,
        }
    }

    pub const fn center() -> Self {
        Self {
            anchor: Anchor::Center,
            margin: CORNER_MARGIN,
        }
    }

    pub fn bounds(self, monitor: Bounds<Pixels>, content: Size<Pixels>) -> Bounds<Pixels> {
        let content = clamped_to_monitor(content, monitor, self.margin);
        match self.anchor {
            Anchor::Corner(corner) => corner_bounds(monitor, corner, content, self.margin),
            Anchor::TopCenter => top_center_bounds(monitor, content, self.margin),
            Anchor::Center => centered_bounds(monitor, content),
        }
    }

    pub fn projected_bounds(
        self,
        monitor: Bounds<Pixels>,
        content: Size<Pixels>,
        viewport: Bounds<Pixels>,
    ) -> Option<Bounds<Pixels>> {
        project_bounds(self.bounds(monitor, content), viewport)
    }
}

pub fn monitor_at_point(
    monitors: &[Bounds<Pixels>],
    point: Point<Pixels>,
) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .copied()
        .find(|bounds| bounds_contains(*bounds, point))
}

pub fn intersect_bounds(left: Bounds<Pixels>, right: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let x = left.origin.x.to_f64().max(right.origin.x.to_f64());
    let y = left.origin.y.to_f64().max(right.origin.y.to_f64());
    let right_edge = (left.origin.x + left.size.width)
        .to_f64()
        .min((right.origin.x + right.size.width).to_f64());
    let bottom_edge = (left.origin.y + left.size.height)
        .to_f64()
        .min((right.origin.y + right.size.height).to_f64());
    let width = right_edge - x;
    let height = bottom_edge - y;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Bounds::new(
        point(px(x as f32), px(y as f32)),
        size(px(width as f32), px(height as f32)),
    ))
}

pub fn project_bounds(global: Bounds<Pixels>, viewport: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let clipped = intersect_bounds(global, viewport)?;
    Some(Bounds::new(
        point(
            clipped.origin.x - viewport.origin.x,
            clipped.origin.y - viewport.origin.y,
        ),
        clipped.size,
    ))
}

fn bounds_contains(bounds: Bounds<Pixels>, point: Point<Pixels>) -> bool {
    point.x >= bounds.origin.x
        && point.x < bounds.origin.x + bounds.size.width
        && point.y >= bounds.origin.y
        && point.y < bounds.origin.y + bounds.size.height
}

fn clamped_to_monitor(content: Size<Pixels>, monitor: Bounds<Pixels>, margin: f32) -> Size<Pixels> {
    size(
        clamped_axis(content.width, monitor.size.width, margin),
        clamped_axis(content.height, monitor.size.height, margin),
    )
}

fn clamped_axis(content: Pixels, extent: Pixels, margin: f32) -> Pixels {
    let minimum = if extent < px(1.0) { extent } else { px(1.0) };
    let available = extent - px(2.0 * margin);
    let maximum = if available < minimum {
        minimum
    } else {
        available
    };
    if content > maximum {
        maximum
    } else {
        content
    }
}

fn edge_margin(extent: Pixels, content: Pixels, requested: f32) -> Pixels {
    let available = (extent - content) / 2.0;
    if available <= px(0.0) {
        px(0.0)
    } else if px(requested) > available {
        available
    } else {
        px(requested)
    }
}

fn corner_bounds(
    monitor: Bounds<Pixels>,
    corner: Corner,
    content: Size<Pixels>,
    margin: f32,
) -> Bounds<Pixels> {
    let margin_x = edge_margin(monitor.size.width, content.width, margin);
    let margin_y = edge_margin(monitor.size.height, content.height, margin);
    let min_x = monitor.origin.x + margin_x;
    let max_x = monitor.origin.x + monitor.size.width - content.width - margin_x;
    let min_y = monitor.origin.y + margin_y;
    let max_y = monitor.origin.y + monitor.size.height - content.height - margin_y;
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => min_x,
        Corner::TopRight | Corner::BottomRight => max_x,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => min_y,
        Corner::BottomLeft | Corner::BottomRight => max_y,
    };
    Bounds::new(point(x, y), content)
}

fn top_center_bounds(
    monitor: Bounds<Pixels>,
    content: Size<Pixels>,
    margin: f32,
) -> Bounds<Pixels> {
    let margin_y = edge_margin(monitor.size.height, content.height, margin);
    Bounds::new(
        point(
            monitor.origin.x + (monitor.size.width - content.width) / 2.0,
            monitor.origin.y + margin_y,
        ),
        content,
    )
}

fn centered_bounds(monitor: Bounds<Pixels>, content: Size<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        point(
            monitor.origin.x + (monitor.size.width - content.width) / 2.0,
            monitor.origin.y + (monitor.size.height - content.height) / 3.0,
        ),
        content,
    )
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

#[cfg(test)]
mod tests {
    use gpui::{point, px, size, Bounds};

    use super::{
        intersect_bounds, monitor_at_point, project_bounds, Anchor, Corner, MonitorPlacement,
    };

    #[test]
    fn placements_share_monitor_relative_geometry() {
        let monitor = Bounds::new(point(px(2560.0), px(0.0)), size(px(1920.0), px(1080.0)));
        let content = size(px(520.0), px(78.0));
        let cases = [
            (
                MonitorPlacement::top_center(48.0),
                Anchor::TopCenter,
                (3260.0, 48.0),
            ),
            (
                MonitorPlacement::corner(Corner::BottomRight, 24.0),
                Anchor::Corner(Corner::BottomRight),
                (3936.0, 978.0),
            ),
            (MonitorPlacement::center(), Anchor::Center, (3260.0, 334.0)),
        ];

        for (placement, anchor, expected) in cases {
            assert_eq!(placement.anchor, anchor);
            let bounds = placement.bounds(monitor, content);
            assert_eq!(
                (bounds.origin.x.to_f64(), bounds.origin.y.to_f64()),
                expected
            );
        }
    }

    #[test]
    fn monitor_resolution_uses_physical_topology() {
        let primary = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let secondary = Bounds::new(point(px(2560.0), px(0.0)), size(px(1920.0), px(1080.0)));

        assert_eq!(
            monitor_at_point(&[primary, secondary], point(px(3000.0), px(500.0))),
            Some(secondary)
        );
        assert_eq!(
            monitor_at_point(&[primary, secondary], point(px(2000.0), px(500.0))),
            Some(primary)
        );
    }

    #[test]
    fn projection_converts_global_bounds_to_viewport_coordinates() {
        let viewport = Bounds::new(point(px(2560.0), px(0.0)), size(px(1920.0), px(1080.0)));
        let global = Bounds::new(point(px(3260.0), px(48.0)), size(px(520.0), px(78.0)));

        assert_eq!(
            project_bounds(global, viewport),
            Some(Bounds::new(
                point(px(700.0), px(48.0)),
                size(px(520.0), px(78.0))
            ))
        );
        assert_eq!(
            intersect_bounds(
                global,
                Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)))
            ),
            None
        );
    }

    #[test]
    fn placement_stays_inside_negative_and_tiny_monitor_bounds() {
        let monitor = Bounds::new(point(px(-20.0), px(-10.0)), size(px(20.0), px(10.0)));
        let bounds = MonitorPlacement::corner(Corner::BottomRight, 24.0)
            .bounds(monitor, size(px(340.0), px(76.0)));

        assert_eq!(
            bounds,
            Bounds::new(point(px(-10.5), px(-5.5)), size(px(1.0), px(1.0)))
        );
    }
}
