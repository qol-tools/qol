use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

const SNAP_MARGIN_PX: i32 = 50;

pub(crate) fn rect_label(rect: Rect) -> String {
    format!("{}x{}+{},{}", rect.w, rect.h, rect.x, rect.y)
}

pub(crate) fn monitor_label(monitor: Monitor) -> String {
    format!("{}x{}+{},{}", monitor.w, monitor.h, monitor.x, monitor.y)
}

pub(crate) fn prepare_recording_rect(
    mut rect: Rect,
    monitors: &[Monitor],
    fallback_bounds: Monitor,
) -> Rect {
    let bounds = bounds_for_selection(rect, monitors).unwrap_or(fallback_bounds);
    rect = clamp_to_bounds(rect, bounds);
    snap_to_bottom(rect, bounds.y + bounds.h)
}

pub(crate) fn prepare_screenshot_rect(
    mut rect: Rect,
    monitors: &[Monitor],
    fallback_bounds: Monitor,
) -> Rect {
    let bounds = bounds_for_selection(rect, monitors).unwrap_or(fallback_bounds);
    rect = clamp_to_bounds(rect, bounds);
    rect
}

pub(crate) fn even_dimensions(mut rect: Rect) -> Rect {
    if rect.w % 2 != 0 {
        rect.w -= 1;
    }
    if rect.h % 2 != 0 {
        rect.h -= 1;
    }
    rect
}

pub(crate) fn monitor_for_selection(rect: Rect, monitors: &[Monitor]) -> Option<Monitor> {
    let center_x = rect.x + rect.w / 2;
    let center_y = rect.y + rect.h / 2;
    monitors.iter().copied().find(|monitor| {
        center_x >= monitor.x
            && center_x < monitor.x + monitor.w
            && center_y >= monitor.y
            && center_y < monitor.y + monitor.h
    })
}

fn bounds_for_selection(rect: Rect, monitors: &[Monitor]) -> Option<Monitor> {
    let intersected = monitors
        .iter()
        .copied()
        .filter(|monitor| rects_intersect(rect, *monitor))
        .collect::<Vec<_>>();

    match intersected.as_slice() {
        [] => monitor_for_selection(rect, monitors),
        [monitor] => Some(*monitor),
        monitors => union_bounds(monitors),
    }
}

pub(crate) fn rect_intersection(rect: Rect, bounds: Monitor) -> Option<Rect> {
    let x = rect.x.max(bounds.x);
    let y = rect.y.max(bounds.y);
    let right = (rect.x + rect.w).min(bounds.x + bounds.w);
    let bottom = (rect.y + rect.h).min(bounds.y + bounds.h);
    let w = right - x;
    let h = bottom - y;
    (w > 0 && h > 0).then_some(Rect { x, y, w, h })
}

fn rects_intersect(left: Rect, right: Monitor) -> bool {
    rect_intersection(left, right).is_some()
}

pub(crate) fn union_bounds(monitors: &[Monitor]) -> Option<Monitor> {
    let first = monitors.first().copied()?;
    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.x + first.w;
    let mut bottom = first.y + first.h;

    for monitor in monitors.iter().skip(1) {
        left = left.min(monitor.x);
        top = top.min(monitor.y);
        right = right.max(monitor.x + monitor.w);
        bottom = bottom.max(monitor.y + monitor.h);
    }

    Some(Monitor {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

pub(crate) fn clamp_to_bounds(mut rect: Rect, bounds: Monitor) -> Rect {
    if rect.x < bounds.x {
        rect.w -= bounds.x - rect.x;
        rect.x = bounds.x;
    }
    if rect.y < bounds.y {
        rect.h -= bounds.y - rect.y;
        rect.y = bounds.y;
    }
    if rect.x + rect.w > bounds.x + bounds.w {
        rect.w = bounds.x + bounds.w - rect.x;
    }
    if rect.y + rect.h > bounds.y + bounds.h {
        rect.h = bounds.y + bounds.h - rect.y;
    }
    rect
}

fn snap_to_bottom(mut rect: Rect, bottom: i32) -> Rect {
    let gap = bottom - (rect.y + rect.h);
    if gap > 0 && gap <= SNAP_MARGIN_PX {
        rect.h = bottom - rect.y;
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_clamps_to_selected_monitor() {
        let rect = Rect {
            x: 10,
            y: 10,
            w: 400,
            h: 300,
        };
        let monitors = [Monitor {
            x: 0,
            y: 0,
            w: 320,
            h: 240,
        }];
        let prepared = prepare_recording_rect(rect, &monitors, monitors[0]);
        assert_eq!(prepared.x, 10);
        assert_eq!(prepared.y, 10);
        assert_eq!(prepared.w, 310);
        assert_eq!(prepared.h, 230);
    }

    #[test]
    fn prepare_preserves_selection_across_adjacent_monitors() {
        let rect = Rect {
            x: 1800,
            y: 100,
            w: 500,
            h: 400,
        };
        let monitors = [
            Monitor {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            Monitor {
                x: 1920,
                y: 0,
                w: 1920,
                h: 1080,
            },
        ];

        let prepared = prepare_recording_rect(rect, &monitors, monitors[0]);
        assert_eq!(prepared.x, 1800);
        assert_eq!(prepared.y, 100);
        assert_eq!(prepared.w, 500);
        assert_eq!(prepared.h, 400);
    }

    #[test]
    fn prepare_preserves_selection_across_vertical_monitors() {
        let rect = Rect {
            x: 100,
            y: 900,
            w: 700,
            h: 500,
        };
        let monitors = [
            Monitor {
                x: 0,
                y: 0,
                w: 1440,
                h: 1000,
            },
            Monitor {
                x: 0,
                y: 1000,
                w: 1440,
                h: 1000,
            },
        ];

        let prepared = prepare_recording_rect(rect, &monitors, monitors[0]);
        assert_eq!(prepared.x, 100);
        assert_eq!(prepared.y, 900);
        assert_eq!(prepared.w, 700);
        assert_eq!(prepared.h, 500);
    }

    #[test]
    fn even_dimensions_drop_odd_pixel() {
        let rect = even_dimensions(Rect {
            x: 0,
            y: 0,
            w: 101,
            h: 99,
        });
        assert_eq!(rect.w, 100);
        assert_eq!(rect.h, 98);
    }

    #[test]
    fn prepare_screenshot_rect_does_not_snap_bottom_edge() {
        let bounds = Monitor {
            x: 0,
            y: 0,
            w: 320,
            h: 240,
        };
        let rect = prepare_screenshot_rect(
            Rect {
                x: 10,
                y: 10,
                w: 100,
                h: 200,
            },
            &[bounds],
            bounds,
        );
        assert_eq!(rect.h, 200);
    }

    #[test]
    fn rect_intersection_cases() {
        let display = Monitor {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let cases = [
            (
                "inside",
                Rect {
                    x: 100,
                    y: 100,
                    w: 200,
                    h: 200,
                },
                Some(Rect {
                    x: 100,
                    y: 100,
                    w: 200,
                    h: 200,
                }),
            ),
            (
                "clipped-right",
                Rect {
                    x: 1800,
                    y: 0,
                    w: 400,
                    h: 100,
                },
                Some(Rect {
                    x: 1800,
                    y: 0,
                    w: 120,
                    h: 100,
                }),
            ),
            (
                "disjoint",
                Rect {
                    x: 3000,
                    y: 0,
                    w: 100,
                    h: 100,
                },
                None,
            ),
            (
                "edge-touch",
                Rect {
                    x: 1920,
                    y: 0,
                    w: 100,
                    h: 100,
                },
                None,
            ),
        ];
        for (name, rect, expected) in cases {
            assert_eq!(rect_intersection(rect, display), expected, "case: {name}");
        }
    }
}
