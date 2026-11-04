use std::fmt;

use qol_windowing::WindowRect;

use crate::config::{CenterMode, WindowActionsConfig};

pub(super) fn snap_left(work: WindowRect, fraction: f64) -> WindowRect {
    WindowRect {
        width: snapped(work.width, fraction),
        ..work
    }
}

pub(super) fn snap_right(work: WindowRect, fraction: f64) -> WindowRect {
    let width = snapped(work.width, fraction);
    WindowRect {
        x: work.x + (work.width - width),
        width,
        ..work
    }
}

pub(super) fn snap_bottom(work: WindowRect, fraction: f64) -> WindowRect {
    let height = snapped(work.height, fraction);
    WindowRect {
        y: work.y + (work.height - height),
        height,
        ..work
    }
}

fn snapped(length: f64, fraction: f64) -> f64 {
    (length * fraction).round().clamp(1.0, length)
}

pub(super) fn centered(work: WindowRect, config: &WindowActionsConfig) -> WindowRect {
    let (width, height) = center_size_for_monitor(config, work.width, work.height);
    WindowRect {
        x: work.x + (work.width - width) / 2.0,
        y: work.y + (work.height - height) / 2.0,
        width,
        height,
    }
}

fn center_size_for_monitor(
    config: &WindowActionsConfig,
    monitor_width: f64,
    monitor_height: f64,
) -> (f64, f64) {
    let width = if config.center_mode == CenterMode::Percent {
        monitor_width * config.center_width_percent
    } else {
        config.center_width_px
    };
    let height = if config.center_mode == CenterMode::Percent {
        monitor_height * config.center_height_percent
    } else {
        config.center_height_px
    };
    (
        width.clamp(1.0, monitor_width),
        height.clamp(1.0, monitor_height),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorMoveError {
    NoOtherMonitor(usize),
    WindowOffScreen,
}

impl fmt::Display for MonitorMoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOtherMonitor(monitors) => write!(
                f,
                "Cannot move the window to another monitor: {monitors} monitor(s) found"
            ),
            Self::WindowOffScreen => f.write_str(
                "Cannot move the window to another monitor: its center is on no monitor's work area",
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    pub(super) work: WindowRect,
    pub(super) frame: WindowRect,
}

pub(super) fn moved_to_monitor(
    window: WindowRect,
    screens_left_to_right: &[WindowRect],
    delta: i32,
) -> Result<Placement, MonitorMoveError> {
    let screens = screens_left_to_right;
    if screens.len() < 2 {
        return Err(MonitorMoveError::NoOtherMonitor(screens.len()));
    }
    let from_index = screen_index_at(
        screens,
        window.x + window.width / 2.0,
        window.y + window.height / 2.0,
    )
    .ok_or(MonitorMoveError::WindowOffScreen)?;
    let to_index = ((from_index as i32 + delta).rem_euclid(screens.len() as i32)) as usize;
    let from = screens[from_index];
    let to = screens[to_index];
    Ok(Placement {
        work: to,
        frame: WindowRect {
            x: (to.x + (window.x - from.x) / from.width * to.width).round(),
            y: (to.y + (window.y - from.y) / from.height * to.height).round(),
            width: (window.width / from.width * to.width).round(),
            height: (window.height / from.height * to.height).round(),
        },
    })
}

const FRAME_TOLERANCE: f64 = 1.0;

pub(super) fn settled_frame(
    work: WindowRect,
    requested: WindowRect,
    granted: WindowRect,
) -> Option<WindowRect> {
    let settled = WindowRect {
        x: settled_start(
            Span::new(work.x, work.width),
            Span::new(requested.x, requested.width),
            Span::new(granted.x, granted.width),
        ),
        y: settled_start(
            Span::new(work.y, work.height),
            Span::new(requested.y, requested.height),
            Span::new(granted.y, granted.height),
        ),
        ..granted
    };
    let moved = !near(settled.x, granted.x) || !near(settled.y, granted.y);
    moved.then_some(settled)
}

#[derive(Clone, Copy)]
struct Span {
    start: f64,
    length: f64,
}

impl Span {
    fn new(start: f64, length: f64) -> Self {
        Self { start, length }
    }

    fn end(self) -> f64 {
        self.start + self.length
    }
}

enum Anchor {
    Start,
    End,
    Center,
}

impl Anchor {
    fn of(work: Span, requested: Span) -> Self {
        if near(requested.start, work.start) {
            return Self::Start;
        }
        if near(requested.end(), work.end()) {
            return Self::End;
        }
        Self::Center
    }
}

fn settled_start(work: Span, requested: Span, granted: Span) -> f64 {
    if near(granted.length, requested.length) {
        return granted.start;
    }
    if granted.length >= work.length {
        return work.start;
    }
    let anchored = match Anchor::of(work, requested) {
        Anchor::Start => requested.start,
        Anchor::End => requested.end() - granted.length,
        Anchor::Center => requested.start + (requested.length - granted.length) / 2.0,
    };
    anchored.clamp(work.start, work.end() - granted.length)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= FRAME_TOLERANCE
}

fn screen_index_at(screens: &[WindowRect], cx: f64, cy: f64) -> Option<usize> {
    screens.iter().position(|screen| {
        cx >= screen.x
            && cx < screen.x + screen.width
            && cy >= screen.y
            && cy < screen.y + screen.height
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> WindowRect {
        WindowRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn snaps_fill_the_named_part_of_the_work_area() {
        let work = rect(0.0, 40.0, 1280.0, 712.0);
        let cases = [
            ("left", snap_left(work, 0.5), rect(0.0, 40.0, 640.0, 712.0)),
            (
                "right",
                snap_right(work, 0.5),
                rect(640.0, 40.0, 640.0, 712.0),
            ),
            (
                "bottom",
                snap_bottom(work, 0.5),
                rect(0.0, 396.0, 1280.0, 356.0),
            ),
            (
                "left third",
                snap_left(work, 0.333),
                rect(0.0, 40.0, 426.0, 712.0),
            ),
            ("right all", snap_right(work, 1.0), work),
        ];
        for (name, actual, expected) in cases {
            assert_eq!(actual, expected, "{name}");
        }
    }

    #[test]
    fn screen_index_at_locates_point_and_rejects_off_screen() {
        let screens = [rect(0.0, 0.0, 100.0, 100.0), rect(120.0, 0.0, 100.0, 100.0)];
        let cases = [
            (50.0, 50.0, Some(0)),
            (160.0, 50.0, Some(1)),
            (110.0, 50.0, None),
            (50.0, 200.0, None),
            (-5.0, 50.0, None),
        ];
        for (cx, cy, expected) in cases {
            assert_eq!(
                screen_index_at(&screens, cx, cy),
                expected,
                "point ({cx},{cy})"
            );
        }
    }

    #[test]
    fn moved_to_monitor_keeps_the_relative_frame_and_wraps() {
        let screens = [
            rect(0.0, 0.0, 1000.0, 800.0),
            rect(1000.0, 0.0, 2000.0, 1600.0),
        ];
        let window = rect(100.0, 80.0, 500.0, 400.0);
        let on_second = Placement {
            work: screens[1],
            frame: rect(1200.0, 160.0, 1000.0, 800.0),
        };
        let cases = [
            ("next", &screens[..], window, 1, Ok(on_second)),
            ("previous wraps", &screens[..], window, -1, Ok(on_second)),
            (
                "full turn",
                &screens[..],
                window,
                2,
                Ok(Placement {
                    work: screens[0],
                    frame: window,
                }),
            ),
            (
                "one monitor",
                &screens[..1],
                window,
                1,
                Err(MonitorMoveError::NoOtherMonitor(1)),
            ),
            (
                "no monitor",
                &screens[..0],
                window,
                1,
                Err(MonitorMoveError::NoOtherMonitor(0)),
            ),
            (
                "off screen",
                &screens[..],
                rect(5000.0, 0.0, 10.0, 10.0),
                1,
                Err(MonitorMoveError::WindowOffScreen),
            ),
        ];
        for (name, screens, window, delta, expected) in cases {
            assert_eq!(moved_to_monitor(window, screens, delta), expected, "{name}");
        }
    }

    #[test]
    fn a_refused_size_keeps_the_window_inside_the_work_area_on_the_targeted_edge() {
        let work = rect(0.0, 33.0, 1512.0, 949.0);
        let left = rect(0.0, 33.0, 756.0, 949.0);
        let right = rect(756.0, 33.0, 756.0, 949.0);
        let bottom = rect(0.0, 508.0, 1512.0, 474.0);
        let center = rect(356.0, 233.0, 800.0, 549.0);
        let cases = [
            ("right half granted", work, right, right, None),
            (
                "right half too narrow",
                work,
                right,
                rect(756.0, 33.0, 856.0, 949.0),
                Some(rect(656.0, 33.0, 856.0, 949.0)),
            ),
            (
                "right half capped below the request",
                work,
                right,
                rect(756.0, 33.0, 600.0, 949.0),
                Some(rect(912.0, 33.0, 600.0, 949.0)),
            ),
            (
                "right half wider than the work area",
                work,
                right,
                rect(756.0, 33.0, 1800.0, 949.0),
                Some(rect(0.0, 33.0, 1800.0, 949.0)),
            ),
            (
                "left half too narrow stays flush left",
                work,
                left,
                rect(0.0, 33.0, 856.0, 949.0),
                None,
            ),
            (
                "left half too narrow drifted right",
                work,
                left,
                rect(10.0, 33.0, 856.0, 949.0),
                Some(rect(0.0, 33.0, 856.0, 949.0)),
            ),
            (
                "bottom half too short",
                work,
                bottom,
                rect(0.0, 508.0, 1512.0, 600.0),
                Some(rect(0.0, 382.0, 1512.0, 600.0)),
            ),
            (
                "bottom half taller than the work area",
                work,
                bottom,
                rect(0.0, 508.0, 1512.0, 1200.0),
                Some(rect(0.0, 33.0, 1512.0, 1200.0)),
            ),
            (
                "maximize wider and taller than the work area",
                work,
                work,
                rect(40.0, 60.0, 1600.0, 1000.0),
                Some(rect(0.0, 33.0, 1600.0, 1000.0)),
            ),
            (
                "centered window grows around its center",
                work,
                center,
                rect(356.0, 233.0, 900.0, 549.0),
                Some(rect(306.0, 233.0, 900.0, 549.0)),
            ),
            (
                "centered growth clamps at the right edge",
                work,
                rect(1100.0, 100.0, 400.0, 300.0),
                rect(1100.0, 100.0, 600.0, 300.0),
                Some(rect(912.0, 100.0, 600.0, 300.0)),
            ),
            (
                "secondary monitor right half too narrow",
                rect(1512.0, 0.0, 1920.0, 1080.0),
                rect(2472.0, 0.0, 960.0, 1080.0),
                rect(2472.0, 0.0, 1000.0, 1080.0),
                Some(rect(2432.0, 0.0, 1000.0, 1080.0)),
            ),
            (
                "rounding stays within tolerance",
                work,
                right,
                rect(756.0, 33.0, 756.5, 949.0),
                None,
            ),
            (
                "accepted axis keeps the platform position",
                work,
                right,
                rect(756.0, 40.0, 856.0, 949.0),
                Some(rect(656.0, 40.0, 856.0, 949.0)),
            ),
        ];
        for (name, work, requested, granted, expected) in cases {
            assert_eq!(settled_frame(work, requested, granted), expected, "{name}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_percent_center_size_tracks_monitor_dimensions(
            monitor_width in 100.0f64..6000.0,
            monitor_height in 100.0f64..4000.0,
            width_percent in 0.1f64..1.0,
            height_percent in 0.1f64..1.0
        ) {
            let config = WindowActionsConfig {
                center_mode: CenterMode::Percent,
                center_width_px: 1152.0,
                center_height_px: 892.0,
                center_width_percent: width_percent,
                center_height_percent: height_percent,
                snap_fraction: 0.5,
                reveal_taskbar_after_move: true,
                glide_speed_px_per_second: 1200.0,
            };

            let (width, height) =
                center_size_for_monitor(&config, monitor_width, monitor_height);

            prop_assert_eq!(width, monitor_width * width_percent);
            prop_assert_eq!(height, monitor_height * height_percent);
        }

        #[test]
        fn prop_a_refused_size_that_fits_lands_inside_the_work_area(
            fraction in 0.1f64..0.9,
            granted_width in 1.0f64..1512.0,
            right in any::<bool>(),
        ) {
            let work = rect(0.0, 33.0, 1512.0, 949.0);
            let requested = if right {
                snap_right(work, fraction)
            } else {
                snap_left(work, fraction)
            };
            let granted = WindowRect { width: granted_width, ..requested };
            let settled = settled_frame(work, requested, granted).unwrap_or(granted);
            prop_assert!(settled.x >= work.x - FRAME_TOLERANCE);
            prop_assert!(settled.x + settled.width <= work.x + work.width + FRAME_TOLERANCE);
            if right {
                prop_assert!(near(settled.x + settled.width, work.x + work.width));
            } else {
                prop_assert!(near(settled.x, work.x));
            }
        }
    }
}
