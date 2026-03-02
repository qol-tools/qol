use super::{ax, screen};
use super::screen::Rect;

fn frontmost_screen() -> Result<(i32, Rect), String> {
    let pid = ax::frontmost_pid().ok_or("No frontmost application")?;
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] frontmost pid={pid}");
    let is_normal = ax::is_normal_window(pid);
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] is_normal_window={is_normal}");
    let win = ax::front_window_rect(pid).ok_or("Cannot read window geometry")?;
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] window rect: x={} y={} w={} h={}", win.x, win.y, win.w, win.h);
    let scr = screen::screen_for_point(win.x + win.w / 2.0, win.y + win.h / 2.0)
        .ok_or("Cannot determine screen")?;
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] screen rect: x={} y={} w={} h={}", scr.x, scr.y, scr.w, scr.h);
    Ok((pid, scr))
}

fn ax_set(pid: i32, rect: Rect) -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] ax_set pid={pid} target: x={} y={} w={} h={}", rect.x, rect.y, rect.w, rect.h);
    let ok = ax::set_position_and_size(pid, rect);
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] ax_set result={ok}");
    if !ok {
        return Err("Failed to set window geometry".into());
    }
    #[cfg(debug_assertions)]
    if let Some(after) = ax::front_window_rect(pid) {
        eprintln!("[window-actions:dbg] after set: x={} y={} w={} h={}", after.x, after.y, after.w, after.h);
    }
    Ok(())
}

pub fn snap_left() -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] action: snap_left");
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, Rect { x: s.x, y: s.y, w: s.w / 2.0, h: s.h })
}

pub fn snap_right() -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] action: snap_right");
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, Rect { x: s.x + s.w / 2.0, y: s.y, w: s.w / 2.0, h: s.h })
}

pub fn snap_bottom() -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] action: snap_bottom");
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, Rect { x: s.x, y: s.y + s.h / 2.0, w: s.w, h: s.h / 2.0 })
}

pub fn maximize() -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] action: maximize");
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, s)
}

pub fn center() -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] action: center");
    let (pid, s) = frontmost_screen()?;
    let w = 1152.0_f64.min(s.w);
    let h = 892.0_f64.min(s.h);
    let target = Rect {
        x: s.x + (s.w - w) / 2.0,
        y: s.y + (s.h - h) / 2.0,
        w,
        h,
    };
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] center target: x={} y={} w={} h={}", target.x, target.y, target.w, target.h);
    ax_set(pid, target)
}

pub fn move_monitor_left() -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] action: move_monitor_left");
    move_monitor(-1)
}

pub fn move_monitor_right() -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] action: move_monitor_right");
    move_monitor(1)
}

fn move_monitor(delta: i32) -> Result<(), String> {
    let pid = ax::frontmost_pid().ok_or("No frontmost application")?;
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] move_monitor pid={pid} delta={delta}");
    let win = ax::front_window_rect(pid).ok_or("Cannot read window geometry")?;
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] window rect: x={} y={} w={} h={}", win.x, win.y, win.w, win.h);
    let cx = win.x + win.w / 2.0;
    let cy = win.y + win.h / 2.0;

    let screens = screen::all_screens_sorted();
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] screens ({} total): {screens:?}", screens.len());
    if screens.len() < 2 {
        return Ok(());
    }

    let from_idx = screens
        .iter()
        .position(|s| cx >= s.x && cx < s.x + s.w && cy >= s.y && cy < s.y + s.h)
        .unwrap_or(0);

    let to_idx = ((from_idx as i32 + delta).rem_euclid(screens.len() as i32)) as usize;
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] from_idx={from_idx} to_idx={to_idx}");
    let from = &screens[from_idx];
    let to = &screens[to_idx];

    let x_ratio = (win.x - from.x) / from.w;
    let y_ratio = (win.y - from.y) / from.h;
    let w_ratio = win.w / from.w;
    let h_ratio = win.h / from.h;

    ax_set(pid, Rect {
        x: (to.x + x_ratio * to.w).round(),
        y: (to.y + y_ratio * to.h).round(),
        w: (w_ratio * to.w).round(),
        h: (h_ratio * to.h).round(),
    })
}
