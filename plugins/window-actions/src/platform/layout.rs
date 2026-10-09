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

pub(super) fn moved_to_monitor(
    window: WindowRect,
    screens_left_to_right: &[WindowRect],
    delta: i32,
) -> Option<WindowRect> {
    let screens = screens_left_to_right;
    if screens.len() < 2 {
        return None;
    }
    let from_index = screen_index_at(
        screens,
        window.x + window.width / 2.0,
        window.y + window.height / 2.0,
    )?;
    let to_index = ((from_index as i32 + delta).rem_euclid(screens.len() as i32)) as usize;
    let from = screens[from_index];
    let to = screens[to_index];
    Some(WindowRect {
        x: (to.x + (window.x - from.x) / from.width * to.width).round(),
        y: (to.y + (window.y - from.y) / from.height * to.height).round(),
        width: (window.width / from.width * to.width).round(),
        height: (window.height / from.height * to.height).round(),
    })
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
        let cases = [
            (
                "next",
                &screens[..],
                window,
                1,
                Some(rect(1200.0, 160.0, 1000.0, 800.0)),
            ),
            (
                "previous wraps",
                &screens[..],
                window,
                -1,
                Some(rect(1200.0, 160.0, 1000.0, 800.0)),
            ),
            ("full turn", &screens[..], window, 2, Some(window)),
            ("one monitor", &screens[..1], window, 1, None),
            (
                "off screen",
                &screens[..],
                rect(5000.0, 0.0, 10.0, 10.0),
                1,
                None,
            ),
        ];
        for (name, screens, window, delta, expected) in cases {
            assert_eq!(moved_to_monitor(window, screens, delta), expected, "{name}");
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
    }
}
