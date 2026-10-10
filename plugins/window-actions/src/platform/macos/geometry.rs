use qol_windowing::WindowRect;

use super::screen::Rect;
use super::{ax, screen};
use crate::config::WindowActionsConfig;
use crate::platform::layout;

fn frontmost_screen() -> Result<(i32, Rect), String> {
    let pid = ax::frontmost_pid().ok_or("No frontmost application")?;
    let win = ax::front_window_rect(pid).ok_or("Cannot read window geometry")?;
    let scr = screen::screen_for_point(win.x + win.w / 2.0, win.y + win.h / 2.0)
        .ok_or("Cannot determine screen")?;
    Ok((pid, scr))
}

pub(super) fn ax_set(pid: i32, rect: Rect) -> Result<(), String> {
    if ax::set_position_and_size(pid, rect) {
        Ok(())
    } else {
        Err("Failed to set window geometry".into())
    }
}

fn frame(rect: Rect) -> WindowRect {
    WindowRect {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
    }
}

fn rect(frame: WindowRect) -> Rect {
    Rect {
        x: frame.x,
        y: frame.y,
        w: frame.width,
        h: frame.height,
    }
}

fn place_on_frontmost_screen(target: impl FnOnce(WindowRect) -> WindowRect) -> Result<(), String> {
    let (pid, screen) = frontmost_screen()?;
    ax_set(pid, rect(target(frame(screen))))
}

pub fn snap_left(config: &WindowActionsConfig) -> Result<(), String> {
    place_on_frontmost_screen(|work| layout::snap_left(work, config.snap_fraction))
}

pub fn snap_right(config: &WindowActionsConfig) -> Result<(), String> {
    place_on_frontmost_screen(|work| layout::snap_right(work, config.snap_fraction))
}

pub fn snap_bottom(config: &WindowActionsConfig) -> Result<(), String> {
    place_on_frontmost_screen(|work| layout::snap_bottom(work, config.snap_fraction))
}

pub fn maximize() -> Result<(), String> {
    place_on_frontmost_screen(|work| work)
}

pub fn center(config: &WindowActionsConfig) -> Result<(), String> {
    place_on_frontmost_screen(|work| layout::centered(work, config))
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
    let screens: Vec<WindowRect> = screen::all_screens_sorted()
        .ok_or("Cannot read display layout")?
        .into_iter()
        .map(frame)
        .collect();
    match layout::moved_to_monitor(frame(win), &screens, delta) {
        Some(target) => ax_set(pid, rect(target)),
        None => Ok(()),
    }
}
