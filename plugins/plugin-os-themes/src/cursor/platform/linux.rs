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

struct BaseCursor {
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    pixels: Vec<u32>,
    default_size: u32,
}

pub fn run() -> Result<()> {
    let config = crate::config::load();
    let calm_duration = Duration::from_millis(config.calm_duration_ms);
    let scale_factor = config.scale_factor as f32;
    let grow_step = (scale_factor - 1.0) / (config.restore_steps as f32 / 2.0).max(1.0);
    let shrink_step = (scale_factor - 1.0) / (config.restore_steps as f32).max(1.0);
    unsafe {
        libc::signal(libc::SIGTERM, handle_signal as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_signal as libc::sighandler_t);
    }
    let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
    anyhow::ensure!(!display.is_null(), "failed to open X11 display");
    let root = unsafe { xlib::XDefaultRootWindow(display) };
    eprintln!("[shake-to-grow] started");

    let base = match load_base_pixels(display, config.scale_factor) {
        Some(b) => b,
        None => {
            eprintln!("[shake-to-grow] warn: failed to load base cursor pixels");
            unsafe { xlib::XCloseDisplay(display) };
            return Ok(());
        }
    };

    let mut samples: VecDeque<(Instant, i32, i32)> = VecDeque::new();
    let mut last_pos = query_pointer(display, root);
    let mut current_scale: f32 = 1.0;
    let mut last_shake: Option<Instant> = None;
    let mut active_cursor: Option<xlib::Cursor> = None;

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
        let is_shake = v > config.velocity_threshold && shakiness(&samples) > config.shakiness_threshold;
        let target = if is_shake {
            last_shake = Some(now);
            scale_factor
        } else if current_scale > 1.0 + f32::EPSILON {
            if v > config.post_trigger_threshold {
                last_shake = Some(now);
                scale_factor
            } else if last_shake.map_or(false, |t| now - t > calm_duration) {
                1.0
            } else {
                current_scale
            }
        } else {
            1.0
        };

        let new_scale = if target > current_scale {
            (current_scale + grow_step).min(target)
        } else if target < current_scale {
            (current_scale - shrink_step).max(target)
        } else {
            current_scale
        };

        if (new_scale - current_scale).abs() > f32::EPSILON {
            let was_grown = current_scale > 1.0 + f32::EPSILON;
            current_scale = new_scale;
            let is_grown = current_scale > 1.0 + f32::EPSILON;
            if !was_grown && is_grown {
                eprintln!("[shake-to-grow] grow velocity={v:.0} px/s");
            } else if was_grown && !is_grown {
                eprintln!("[shake-to-grow] restore");
            }
            if let Some(cursor) = make_cursor_at_scale(display, &base, current_scale) {
                apply_to_tree(display, root, cursor);
                unsafe { xlib::XFlush(display) };
                if let Some(old) = active_cursor.replace(cursor) {
                    unsafe { xlib::XFreeCursor(display, old) };
                }
            }
        } else if current_scale > 1.0 + f32::EPSILON {
            if let Some(cursor) = active_cursor {
                apply_to_tree(display, root, cursor);
                unsafe { xlib::XFlush(display) };
            }
        }
    }

    if let Some(old) = active_cursor.take() {
        unsafe { xlib::XFreeCursor(display, old) };
    }
    if current_scale > 1.0 + f32::EPSILON {
        if let Some(cursor) = make_cursor_at_scale(display, &base, 1.0) {
            apply_to_tree(display, root, cursor);
            unsafe {
                xlib::XFlush(display);
                xlib::XFreeCursor(display, cursor);
            }
        }
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

fn shakiness(samples: &VecDeque<(Instant, i32, i32)>) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let total: f64 = samples
        .iter()
        .map(|(_, dx, dy)| ((*dx as f64).powi(2) + (*dy as f64).powi(2)).sqrt())
        .sum();
    if total < 1.0 {
        return 0.0;
    }
    let net_x: f64 = samples.iter().map(|(_, dx, _)| *dx as f64).sum();
    let net_y: f64 = samples.iter().map(|(_, _, dy)| *dy as f64).sum();
    let net = (net_x.powi(2) + net_y.powi(2)).sqrt();
    total / (net + 1.0)
}

fn load_base_pixels(display: *mut xlib::Display, scale_factor: u32) -> Option<BaseCursor> {
    let raw_size = unsafe { xcursor::XcursorGetDefaultSize(display) };
    let default_size = if raw_size > 0 { raw_size as u32 } else { 24 };
    let theme = unsafe { xcursor::XcursorGetTheme(display) };
    let images = unsafe {
        xcursor::XcursorLibraryLoadImages(c"left_ptr".as_ptr(), theme, (default_size * scale_factor) as i32)
    };
    if images.is_null() {
        return None;
    }
    let base = unsafe {
        let img = &**(*images).images;
        let pixels = std::slice::from_raw_parts(img.pixels, (img.width * img.height) as usize).to_vec();
        BaseCursor { width: img.width, height: img.height, xhot: img.xhot, yhot: img.yhot, pixels, default_size }
    };
    unsafe { xcursor::XcursorImagesDestroy(images) };
    Some(base)
}

fn make_cursor_at_scale(display: *mut xlib::Display, base: &BaseCursor, scale: f32) -> Option<xlib::Cursor> {
    let target_px = base.default_size as f32 * scale;
    let factor = target_px / base.width as f32;
    let dw = ((base.width as f32 * factor) as u32).max(1);
    let dh = ((base.height as f32 * factor) as u32).max(1);
    let img = unsafe { xcursor::XcursorImageCreate(dw as i32, dh as i32) };
    if img.is_null() {
        return None;
    }
    let cursor = unsafe {
        (*img).xhot = (base.xhot as f32 * factor) as u32;
        (*img).yhot = (base.yhot as f32 * factor) as u32;
        let dst = std::slice::from_raw_parts_mut((*img).pixels, (dw * dh) as usize);
        scale_bilinear(&base.pixels, base.width, base.height, dst, dw, dh);
        let cursor = xcursor::XcursorImageLoadCursor(display, img);
        xcursor::XcursorImageDestroy(img);
        cursor
    };
    if cursor == 0 { None } else { Some(cursor) }
}

fn scale_bilinear(src: &[u32], sw: u32, sh: u32, dst: &mut [u32], dw: u32, dh: u32) {
    let sw_f = sw as f32;
    let sh_f = sh as f32;
    for dy in 0..dh {
        for dx in 0..dw {
            let sx = dx as f32 * (sw_f - 1.0) / (dw as f32 - 1.0).max(1.0);
            let sy = dy as f32 * (sh_f - 1.0) / (dh as f32 - 1.0).max(1.0);
            let x0 = sx as u32;
            let y0 = sy as u32;
            let x1 = (x0 + 1).min(sw - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let tx = sx - x0 as f32;
            let ty = sy - y0 as f32;
            let p00 = src[(y0 * sw + x0) as usize];
            let p10 = src[(y0 * sw + x1) as usize];
            let p01 = src[(y1 * sw + x0) as usize];
            let p11 = src[(y1 * sw + x1) as usize];
            let mut out = 0u32;
            for shift in [0u32, 8, 16, 24] {
                let c = |p: u32| ((p >> shift) & 0xFF) as f32;
                let v = c(p00) * (1.0 - tx) * (1.0 - ty)
                      + c(p10) * tx * (1.0 - ty)
                      + c(p01) * (1.0 - tx) * ty
                      + c(p11) * tx * ty;
                out |= (v as u32 & 0xFF) << shift;
            }
            dst[(dy * dw + dx) as usize] = out;
        }
    }
}

fn apply_to_tree(display: *mut xlib::Display, window: xlib::Window, cursor: xlib::Cursor) {
    unsafe { xlib::XDefineCursor(display, window, cursor) };
    for child in window_children(display, window) {
        apply_to_tree(display, child, cursor);
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

    #[test]
    fn shakiness_glide_is_low() {
        let mut samples = VecDeque::new();
        let t0 = Instant::now();
        for i in 0..9 {
            samples.push_back((t0 + Duration::from_millis(i * 16), 100, 0));
        }
        assert!(shakiness(&samples) < 1.5, "straight glide should have low shakiness");
    }

    #[test]
    fn shakiness_back_and_forth_is_high() {
        let mut samples = VecDeque::new();
        let t0 = Instant::now();
        for i in 0..9 {
            let dx = if i % 2 == 0 { 100 } else { -100 };
            samples.push_back((t0 + Duration::from_millis(i * 16), dx, 0));
        }
        assert!(shakiness(&samples) > 3.0, "back-and-forth should have high shakiness");
    }

    #[test]
    fn scale_bilinear_2x_maps_source_corners() {
        let src = [0xFFFF0000u32, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF];
        let mut dst = [0u32; 16];
        scale_bilinear(&src, 2, 2, &mut dst, 4, 4);
        assert_eq!(dst[0], 0xFFFF0000, "top-left corner");
        assert_eq!(dst[3], 0xFF00FF00, "top-right corner");
        assert_eq!(dst[12], 0xFF0000FF, "bottom-left corner");
        assert_eq!(dst[15], 0xFFFFFFFF, "bottom-right corner");
    }
}
