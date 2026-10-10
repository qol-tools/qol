use anyhow::{anyhow, Result};
use qol_windowing::platform::windows::{cursor_position, ensure_dpi_awareness, monitors};

use crate::capture::geometry::{rect_intersection, union_bounds};
use crate::{Monitor, Rect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NativeDisplay {
    pub(super) physical: Rect,
    pub(super) scale: f32,
}

impl NativeDisplay {
    pub(super) fn logical(self) -> Rect {
        Rect {
            x: scale_down(self.physical.x, self.scale),
            y: scale_down(self.physical.y, self.scale),
            w: scale_down(self.physical.w, self.scale),
            h: scale_down(self.physical.h, self.scale),
        }
    }

    fn to_physical(self, rect: Rect) -> Option<Rect> {
        let logical = self.logical();
        let mapped = Rect {
            x: self.physical.x + scale_up(rect.x - logical.x, self.scale),
            y: self.physical.y + scale_up(rect.y - logical.y, self.scale),
            w: scale_up(rect.w, self.scale),
            h: scale_up(rect.h, self.scale),
        };
        rect_intersection(mapped, monitor_from_rect(self.physical))
    }

    fn to_logical(self, rect: Rect) -> Rect {
        let logical = self.logical();
        Rect {
            x: logical.x + scale_down(rect.x - self.physical.x, self.scale),
            y: logical.y + scale_down(rect.y - self.physical.y, self.scale),
            w: scale_down(rect.w, self.scale),
            h: scale_down(rect.h, self.scale),
        }
    }

    fn logical_point(self, x: i32, y: i32) -> (f32, f32) {
        let logical = self.logical();
        (
            logical.x as f32 + (x - self.physical.x) as f32 / self.scale,
            logical.y as f32 + (y - self.physical.y) as f32 / self.scale,
        )
    }

    fn contains_physical(self, x: i32, y: i32) -> bool {
        let rect = self.physical;
        x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
    }
}

fn scale_down(value: i32, scale: f32) -> i32 {
    (value as f32 / scale).round() as i32
}

fn scale_up(value: i32, scale: f32) -> i32 {
    (value as f32 * scale).round() as i32
}

pub(super) fn monitor_from_rect(rect: Rect) -> Monitor {
    Monitor {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

fn rect_from_monitor(monitor: Monitor) -> Rect {
    Rect {
        x: monitor.x,
        y: monitor.y,
        w: monitor.w,
        h: monitor.h,
    }
}

pub(super) fn physical_rect(rect: Rect, displays: &[NativeDisplay]) -> Rect {
    let pieces = displays
        .iter()
        .filter_map(|display| {
            let piece = rect_intersection(rect, monitor_from_rect(display.logical()))?;
            display.to_physical(piece).map(monitor_from_rect)
        })
        .collect::<Vec<_>>();
    union_bounds(&pieces).map(rect_from_monitor).unwrap_or(rect)
}

pub(super) fn logical_rect(rect: Rect, displays: &[NativeDisplay]) -> Option<Rect> {
    let center_x = rect.x + rect.w / 2;
    let center_y = rect.y + rect.h / 2;
    displays
        .iter()
        .find(|display| display.contains_physical(center_x, center_y))
        .map(|display| display.to_logical(rect))
}

pub(super) fn logical_point(x: i32, y: i32, displays: &[NativeDisplay]) -> Option<(f32, f32)> {
    displays
        .iter()
        .find(|display| display.contains_physical(x, y))
        .map(|display| display.logical_point(x, y))
}

pub(super) fn logical_cursor(displays: &[NativeDisplay]) -> Option<(f32, f32)> {
    let (x, y) = cursor_position()?;
    logical_point(x, y, displays)
}

pub(super) fn native_displays() -> Vec<NativeDisplay> {
    ensure_dpi_awareness();
    monitors()
        .into_iter()
        .filter_map(|monitor| {
            let bounds = monitor.bounds;
            let physical = Rect {
                x: bounds.x as i32,
                y: bounds.y as i32,
                w: bounds.width as i32,
                h: bounds.height as i32,
            };
            (physical.w > 0 && physical.h > 0).then_some(NativeDisplay {
                physical,
                scale: monitor.scale,
            })
        })
        .collect()
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    let monitors = native_displays()
        .into_iter()
        .map(|display| monitor_from_rect(display.logical()))
        .collect::<Vec<_>>();
    if monitors.is_empty() {
        return Err(anyhow!("Windows reported no display monitors"));
    }
    Ok(monitors)
}

pub fn full_screen_bounds() -> Result<Monitor> {
    union_bounds(&get_monitors()?).ok_or_else(|| anyhow!("Windows reported no display monitors"))
}

#[cfg(test)]
mod tests {
    use super::{logical_point, logical_rect, physical_rect, NativeDisplay};
    use crate::Rect;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    fn display(physical: Rect, scale: f32) -> NativeDisplay {
        NativeDisplay { physical, scale }
    }

    #[test]
    fn logical_bounds_divide_the_physical_rect_by_the_display_scale() {
        let cases = [
            (display(rect(0, 0, 1920, 1080), 1.0), rect(0, 0, 1920, 1080)),
            (display(rect(0, 0, 3840, 2160), 2.0), rect(0, 0, 1920, 1080)),
            (
                display(rect(1920, 0, 1920, 1080), 1.25),
                rect(1536, 0, 1536, 864),
            ),
            (
                display(rect(-2560, -200, 2560, 1440), 1.0),
                rect(-2560, -200, 2560, 1440),
            ),
        ];
        for (display, expected) in cases {
            assert_eq!(display.logical(), expected, "display: {display:?}");
        }
    }

    #[test]
    fn selections_map_to_physical_pixels_per_display() {
        let single_2x = [display(rect(0, 0, 3840, 2160), 2.0)];
        let side_by_side = [
            display(rect(0, 0, 1920, 1080), 1.0),
            display(rect(1920, 0, 1920, 1080), 1.0),
        ];
        let offset_left = [
            display(rect(-1920, 0, 1920, 1080), 1.0),
            display(rect(0, 0, 2560, 1440), 1.0),
        ];
        let cases: [(&str, &[NativeDisplay], Rect, Rect); 6] = [
            (
                "identity at 100%",
                &side_by_side,
                rect(10, 20, 300, 200),
                rect(10, 20, 300, 200),
            ),
            (
                "scaled at 200%",
                &single_2x,
                rect(10, 20, 300, 200),
                rect(20, 40, 600, 400),
            ),
            (
                "spans two monitors",
                &side_by_side,
                rect(1800, 100, 300, 100),
                rect(1800, 100, 300, 100),
            ),
            (
                "negative origin monitor",
                &offset_left,
                rect(-100, 10, 50, 50),
                rect(-100, 10, 50, 50),
            ),
            (
                "clamped to the display edge",
                &single_2x,
                rect(1900, 1000, 100, 100),
                rect(3800, 2000, 40, 160),
            ),
            (
                "outside every display keeps the selection",
                &side_by_side,
                rect(5000, 5000, 10, 10),
                rect(5000, 5000, 10, 10),
            ),
        ];
        for (name, displays, selection, expected) in cases {
            assert_eq!(physical_rect(selection, displays), expected, "{name}");
        }
    }

    #[test]
    fn physical_windows_and_cursor_map_back_to_logical_space() {
        let displays = [
            display(rect(0, 0, 1920, 1080), 1.0),
            display(rect(1920, 0, 3840, 2160), 2.0),
        ];
        let window_cases = [
            (rect(100, 100, 400, 300), Some(rect(100, 100, 400, 300))),
            (rect(2120, 200, 800, 600), Some(rect(1060, 100, 400, 300))),
            (rect(9000, 9000, 10, 10), None),
        ];
        for (window, expected) in window_cases {
            assert_eq!(logical_rect(window, &displays), expected, "{window:?}");
        }
        let point_cases = [
            ((50, 60), Some((50.0, 60.0))),
            ((1920, 0), Some((960.0, 0.0))),
            ((2920, 1000), Some((1460.0, 500.0))),
            ((-1, 0), None),
        ];
        for ((x, y), expected) in point_cases {
            assert_eq!(logical_point(x, y, &displays), expected, "{x},{y}");
        }
    }
}
