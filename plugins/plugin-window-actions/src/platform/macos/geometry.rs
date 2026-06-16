use super::screen::Rect;
use super::{ax, screen};
use crate::config::WindowActionsConfig;

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
    let (w, h) = config.center_size_for_monitor(s.w, s.h);
    let target = Rect {
        x: s.x + (s.w - w) / 2.0,
        y: s.y + (s.h - h) / 2.0,
        w,
        h,
    };
    ax_set(pid, target)
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

    let screens = screen::all_screens_sorted();
    if screens.len() < 2 {
        return Ok(());
    }

    let from_idx = screens
        .iter()
        .position(|s| cx >= s.x && cx < s.x + s.w && cy >= s.y && cy < s.y + s.h)
        .unwrap_or(0);

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
