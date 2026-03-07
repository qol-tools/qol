use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use x11::{xcursor, xlib};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-os-themes/";
const POLL_INTERVAL: Duration = Duration::from_millis(16);
const VELOCITY_WINDOW: Duration = Duration::from_millis(150);

static RUNNING: AtomicBool = AtomicBool::new(true);
static RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn reset_running() {
    RELOAD_REQUESTED.store(false, Ordering::SeqCst);
    RUNNING.store(true, Ordering::SeqCst);
}

pub fn request_shutdown() {
    RUNNING.store(false, Ordering::Relaxed);
}

pub fn request_reload() {
    RELOAD_REQUESTED.store(true, Ordering::SeqCst);
    RUNNING.store(false, Ordering::SeqCst);
}

pub fn was_reload_requested() -> bool {
    RELOAD_REQUESTED.load(Ordering::SeqCst)
}

extern "C" fn handle_signal(_: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}

pub fn run() -> Result<()> {
    let config = crate::config::load();
    let calm_duration = Duration::from_millis(config.calm_duration_ms);
    unsafe {
        libc::signal(libc::SIGTERM, handle_signal as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_signal as libc::sighandler_t);
    }
    let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
    anyhow::ensure!(!display.is_null(), "failed to open X11 display");
    let root = unsafe { xlib::XDefaultRootWindow(display) };
    eprintln!("[shake-to-grow] started");

    let mut samples: VecDeque<(Instant, i32, i32)> = VecDeque::new();
    let mut last_pos = query_pointer(display, root);
    let mut grown: Option<xlib::Cursor> = None;
    let mut last_shake: Option<Instant> = None;

    while RUNNING.load(Ordering::Relaxed) {
        std::thread::sleep(POLL_INTERVAL);
        let now = Instant::now();
        let pos = query_pointer(display, root);
        samples.push_back((now, pos.0 - last_pos.0, pos.1 - last_pos.1));
        last_pos = pos;

        while samples.front().map_or(false, |(t, _, _)| now - *t > VELOCITY_WINDOW) {
            samples.pop_front();
        }

        let v = velocity(&samples);
        if v > config.velocity_threshold {
            last_shake = Some(now);
            if grown.is_none() {
                eprintln!("[shake-to-grow] grow velocity={v:.0} px/s");
                grown = grow_cursor(display, root, config.scale_factor);
            }
        } else if grown.is_some() && v > config.post_trigger_threshold {
            last_shake = Some(now);
        } else if last_shake.map_or(false, |t| now - t > calm_duration) {
            if let Some(cursor) = grown.take() {
                eprintln!("[shake-to-grow] restore");
                restore_cursor(display, root, cursor);
            }
            last_shake = None;
        }
    }

    if let Some(cursor) = grown.take() {
        restore_cursor(display, root, cursor);
    }
    unsafe { xlib::XCloseDisplay(display) };
    Ok(())
}

fn query_pointer(display: *mut xlib::Display, root: xlib::Window) -> (i32, i32) {
    let (mut root_out, mut child_out): (xlib::Window, xlib::Window) = (0, 0);
    let (mut rx, mut ry, mut wx, mut wy): (i32, i32, i32, i32) = (0, 0, 0, 0);
    let mut mask: u32 = 0;
    unsafe {
        xlib::XQueryPointer(
            display,
            root,
            &mut root_out,
            &mut child_out,
            &mut rx,
            &mut ry,
            &mut wx,
            &mut wy,
            &mut mask,
        );
    }
    (rx, ry)
}

fn velocity(samples: &VecDeque<(Instant, i32, i32)>) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let dist: f64 = samples
        .iter()
        .map(|(_, dx, dy)| ((*dx as f64).powi(2) + (*dy as f64).powi(2)).sqrt())
        .sum();
    let elapsed = (samples.back().unwrap().0 - samples.front().unwrap().0).as_secs_f64();
    if elapsed < f64::EPSILON {
        0.0
    } else {
        dist / elapsed
    }
}

fn grow_cursor(display: *mut xlib::Display, root: xlib::Window, scale: u32) -> Option<xlib::Cursor> {
    let cursor = make_grown_cursor(display, scale)?;
    apply_to_tree(display, root, cursor);
    unsafe { xlib::XFlush(display) };
    Some(cursor)
}

fn restore_cursor(display: *mut xlib::Display, root: xlib::Window, grown: xlib::Cursor) {
    if let Some(normal) = make_grown_cursor(display, 1) {
        apply_to_tree(display, root, normal);
        unsafe { xlib::XFreeCursor(display, normal) };
    } else {
        clear_from_tree(display, root);
    }
    unsafe {
        xlib::XFreeCursor(display, grown);
        xlib::XFlush(display);
    }
}

fn apply_to_tree(display: *mut xlib::Display, window: xlib::Window, cursor: xlib::Cursor) {
    unsafe { xlib::XDefineCursor(display, window, cursor) };
    for child in window_children(display, window) {
        apply_to_tree(display, child, cursor);
    }
}

fn clear_from_tree(display: *mut xlib::Display, window: xlib::Window) {
    unsafe { xlib::XUndefineCursor(display, window) };
    for child in window_children(display, window) {
        clear_from_tree(display, child);
    }
}

fn window_children(display: *mut xlib::Display, window: xlib::Window) -> Vec<xlib::Window> {
    let mut root_ret: xlib::Window = 0;
    let mut parent_ret: xlib::Window = 0;
    let mut children: *mut xlib::Window = ptr::null_mut();
    let mut nchildren: u32 = 0;
    let ok = unsafe {
        xlib::XQueryTree(display, window, &mut root_ret, &mut parent_ret, &mut children, &mut nchildren)
    };
    if ok == 0 || children.is_null() {
        return Vec::new();
    }
    let vec = unsafe { std::slice::from_raw_parts(children, nchildren as usize).to_vec() };
    unsafe { xlib::XFree(children as *mut _) };
    vec
}

fn make_grown_cursor(display: *mut xlib::Display, scale: u32) -> Option<xlib::Cursor> {
    let raw_size = unsafe { xcursor::XcursorGetDefaultSize(display) };
    let base_size = if raw_size > 0 { raw_size } else { 24 };
    let target_size = base_size * scale as i32;
    let theme = unsafe { xcursor::XcursorGetTheme(display) };
    let images = unsafe {
        xcursor::XcursorLibraryLoadImages(c"left_ptr".as_ptr(), theme, target_size)
    };
    if images.is_null() {
        eprintln!("[shake-to-grow] warn: XcursorLibraryLoadImages returned null");
        return None;
    }
    let cursor = unsafe { xcursor::XcursorImagesLoadCursor(display, images) };
    unsafe { xcursor::XcursorImagesDestroy(images) };
    if cursor == 0 {
        eprintln!("[shake-to-grow] warn: XcursorImagesLoadCursor returned 0");
        return None;
    }
    Some(cursor)
}

pub fn open_settings() -> Result<()> {
    Command::new("xdg-open")
        .arg(SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn velocity_empty_returns_zero() {
        assert_eq!(velocity(&VecDeque::new()), 0.0);
    }

    #[test]
    fn velocity_single_sample_returns_zero() {
        let mut samples = VecDeque::new();
        samples.push_back((Instant::now(), 100, 0));
        assert_eq!(velocity(&samples), 0.0);
    }

    #[test]
    fn velocity_300px_over_100ms_is_3000px_per_sec() {
        let mut samples = VecDeque::new();
        let t0 = Instant::now();
        samples.push_back((t0, 300, 0));
        samples.push_back((t0 + Duration::from_millis(100), 0, 0));
        let v = velocity(&samples);
        assert!((v - 3000.0).abs() < 1.0, "expected ~3000 px/s, got {v}");
    }
}
