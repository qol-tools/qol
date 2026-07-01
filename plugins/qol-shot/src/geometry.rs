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

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct BackdropCorners {
    pub top_left: bool,
    pub top_right: bool,
    pub bottom_left: bool,
    pub bottom_right: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BackdropRegions {
    pub hole: Rect,
    pub top: Rect,
    pub bottom: Rect,
    pub left: Rect,
    pub right: Rect,
    pub corners: BackdropCorners,
}

pub fn backdrop_regions(capture: Rect, display: Monitor) -> Option<BackdropRegions> {
    let hole = rect_intersection(capture, display)?;
    let local_x = hole.x - display.x;
    let local_y = hole.y - display.y;
    Some(BackdropRegions {
        hole: Rect {
            x: local_x,
            y: local_y,
            w: hole.w,
            h: hole.h,
        },
        top: Rect {
            x: 0,
            y: 0,
            w: display.w,
            h: local_y,
        },
        bottom: Rect {
            x: 0,
            y: local_y + hole.h,
            w: display.w,
            h: display.h - (local_y + hole.h),
        },
        left: Rect {
            x: 0,
            y: local_y,
            w: local_x,
            h: hole.h,
        },
        right: Rect {
            x: local_x + hole.w,
            y: local_y,
            w: display.w - (local_x + hole.w),
            h: hole.h,
        },
        corners: BackdropCorners {
            top_left: hole.x == capture.x && hole.y == capture.y,
            top_right: hole.x + hole.w == capture.x + capture.w && hole.y == capture.y,
            bottom_left: hole.x == capture.x && hole.y + hole.h == capture.y + capture.h,
            bottom_right: hole.x + hole.w == capture.x + capture.w
                && hole.y + hole.h == capture.y + capture.h,
        },
    })
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

    #[test]
    fn backdrop_none_when_capture_outside_display() {
        let display = Monitor {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let capture = Rect {
            x: 2000,
            y: 0,
            w: 100,
            h: 100,
        };
        assert_eq!(backdrop_regions(capture, display), None);
    }

    #[test]
    fn backdrop_centered_capture_dims_all_sides_and_marks_all_corners() {
        let display = Monitor {
            x: 0,
            y: 0,
            w: 1000,
            h: 800,
        };
        let capture = Rect {
            x: 200,
            y: 100,
            w: 600,
            h: 500,
        };
        let regions = backdrop_regions(capture, display).expect("intersects");
        assert_eq!(
            regions.hole,
            Rect {
                x: 200,
                y: 100,
                w: 600,
                h: 500
            }
        );
        assert_eq!(
            regions.top,
            Rect {
                x: 0,
                y: 0,
                w: 1000,
                h: 100
            }
        );
        assert_eq!(
            regions.bottom,
            Rect {
                x: 0,
                y: 600,
                w: 1000,
                h: 200
            }
        );
        assert_eq!(
            regions.left,
            Rect {
                x: 0,
                y: 100,
                w: 200,
                h: 500
            }
        );
        assert_eq!(
            regions.right,
            Rect {
                x: 800,
                y: 100,
                w: 200,
                h: 500
            }
        );
        assert_eq!(
            regions.corners,
            BackdropCorners {
                top_left: true,
                top_right: true,
                bottom_left: true,
                bottom_right: true,
            }
        );
    }

    #[test]
    fn backdrop_uses_display_local_coordinates_on_secondary_monitor() {
        let display = Monitor {
            x: 1920,
            y: 0,
            w: 1000,
            h: 800,
        };
        let capture = Rect {
            x: 2120,
            y: 100,
            w: 600,
            h: 500,
        };
        let regions = backdrop_regions(capture, display).expect("intersects");
        assert_eq!(
            regions.hole,
            Rect {
                x: 200,
                y: 100,
                w: 600,
                h: 500
            },
            "hole must be display-local"
        );
        assert_eq!(
            regions.left,
            Rect {
                x: 0,
                y: 100,
                w: 200,
                h: 500
            }
        );
    }

    #[test]
    fn backdrop_across_two_displays_leaves_seam_undimmed() {
        let left_display = Monitor {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let right_display = Monitor {
            x: 1920,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let capture = Rect {
            x: 1800,
            y: 100,
            w: 300,
            h: 400,
        };

        let left = backdrop_regions(capture, left_display).expect("touches left display");
        assert_eq!(
            left.right.w, 0,
            "seam side of the left display is not dimmed"
        );
        assert!(
            left.corners.top_left && left.corners.bottom_left,
            "outer corners on the left display are real capture corners"
        );
        assert!(
            !left.corners.top_right && !left.corners.bottom_right,
            "seam edge is not a capture corner"
        );

        let right = backdrop_regions(capture, right_display).expect("touches right display");
        assert_eq!(
            right.left.w, 0,
            "seam side of the right display is not dimmed"
        );
        assert!(
            right.corners.top_right && right.corners.bottom_right,
            "outer corners on the right display are real capture corners"
        );
        assert!(
            !right.corners.top_left && !right.corners.bottom_left,
            "seam edge is not a capture corner"
        );
    }
}
