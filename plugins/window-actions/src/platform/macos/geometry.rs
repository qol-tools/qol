use super::screen::Rect;
use super::{ax, screen};
use crate::config::{CenterMode, WindowActionsConfig};

fn frontmost_screen() -> Result<(i32, Rect), String> {
    let pid = ax::frontmost_pid().ok_or("No frontmost application")?;
    let win = ax::front_window_rect(pid).ok_or("Cannot read window geometry")?;
    let scr = screen::screen_for_point(win.x + win.w / 2.0, win.y + win.h / 2.0)
        .ok_or("Cannot determine screen")?;
    Ok((pid, scr))
}

fn ax_set(pid: i32, rect: Rect) -> Result<(), String> {
    if ax::set_position_and_size(pid, rect) {
        Ok(())
    } else {
        Err("Failed to set window geometry".into())
    }
}

pub fn snap_left(config: &WindowActionsConfig) -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    let width = (s.w * config.snap_fraction).round().clamp(1.0, s.w);
    ax_set(
        pid,
        Rect {
            x: s.x,
            y: s.y,
            w: width,
            h: s.h,
        },
    )
}

pub fn snap_right(config: &WindowActionsConfig) -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    let width = (s.w * config.snap_fraction).round().clamp(1.0, s.w);
    ax_set(
        pid,
        Rect {
            x: s.x + (s.w - width),
            y: s.y,
            w: width,
            h: s.h,
        },
    )
}

pub fn snap_bottom(config: &WindowActionsConfig) -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    let height = (s.h * config.snap_fraction).round().clamp(1.0, s.h);
    ax_set(
        pid,
        Rect {
            x: s.x,
            y: s.y + (s.h - height),
            w: s.w,
            h: height,
        },
    )
}

pub fn maximize() -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, s)
}

pub fn center(config: &WindowActionsConfig) -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    let (w, h) = center_size_for_monitor(config, s.w, s.h);
    let target = Rect {
        x: s.x + (s.w - w) / 2.0,
        y: s.y + (s.h - h) / 2.0,
        w,
        h,
    };
    ax_set(pid, target)
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

pub fn move_monitor_left() -> Result<(), String> {
    move_monitor(-1)
}

pub fn move_monitor_right() -> Result<(), String> {
    move_monitor(1)
}

fn move_monitor(delta: i32) -> Result<(), String> {
    let pid = ax::frontmost_pid().ok_or("No frontmost application")?;
    let win = ax::front_window_rect(pid).ok_or("Cannot read window geometry")?;
    let cx = win.x + win.w / 2.0;
    let cy = win.y + win.h / 2.0;

    let screens = screen::all_screens_sorted().ok_or("Cannot read display layout")?;
    if screens.len() < 2 {
        return Ok(());
    }

    let Some(from_idx) = screen_index_at(&screens, cx, cy) else {
        return Ok(());
    };

    let to_idx = ((from_idx as i32 + delta).rem_euclid(screens.len() as i32)) as usize;
    let from = &screens[from_idx];
    let to = &screens[to_idx];

    let x_ratio = (win.x - from.x) / from.w;
    let y_ratio = (win.y - from.y) / from.h;
    let w_ratio = win.w / from.w;
    let h_ratio = win.h / from.h;

    ax_set(
        pid,
        Rect {
            x: (to.x + x_ratio * to.w).round(),
            y: (to.y + y_ratio * to.h).round(),
            w: (w_ratio * to.w).round(),
            h: (h_ratio * to.h).round(),
        },
    )
}

fn screen_index_at(screens: &[Rect], cx: f64, cy: f64) -> Option<usize> {
    screens
        .iter()
        .position(|s| cx >= s.x && cx < s.x + s.w && cy >= s.y && cy < s.y + s.h)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
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
