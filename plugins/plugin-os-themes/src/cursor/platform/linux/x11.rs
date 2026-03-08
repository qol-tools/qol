use std::ptr;

use anyhow::{ensure, Result};
use x11::{xcursor, xlib};

use super::scale::scale_bilinear;

const MAX_CURSOR_DIMENSION: u32 = 512;

pub struct CursorSession {
    display: *mut xlib::Display,
    root: xlib::Window,
    base: BaseCursor,
    active_cursor: Option<xlib::Cursor>,
}

struct PointerState {
    x: i32,
    y: i32,
    child: xlib::Window,
}

struct BaseCursor {
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    pixels: Vec<u32>,
    default_size: u32,
}

impl CursorSession {
    pub fn open(scale_factor: u32) -> Result<Self> {
        let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
        ensure!(!display.is_null(), "failed to open X11 display");

        let Some(base) = load_base_cursor(display, scale_factor) else {
            unsafe { xlib::XCloseDisplay(display) };
            ensure!(false, "failed to load base cursor pixels");
            unreachable!();
        };

        Ok(Self {
            display,
            root: unsafe { xlib::XDefaultRootWindow(display) },
            base,
            active_cursor: None,
        })
    }

    pub fn set_scale(&mut self, scale: f32) -> bool {
        if scale <= 1.0 + f32::EPSILON {
            self.restore();
            return true;
        }
        let Some(cursor) = make_cursor_at_scale(self.display, self.root, &self.base, scale) else {
            return false;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
        if let Some(old_cursor) = self.active_cursor.replace(cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        true
    }

    pub fn refresh(&mut self) -> bool {
        false
    }

    pub fn restore(&mut self) {
        if self.active_cursor.is_none() {
            return;
        }
        let pointer = query_pointer(self.display, self.root);
        clear_tree(self.display, self.root);
        restore_root_cursor(self.display, self.root, &self.base);
        force_cursor_recompute(self.display, self.root, pointer);
        self.flush();
        if let Some(cursor) = self.active_cursor.take() {
            unsafe { xlib::XFreeCursor(self.display, cursor) };
        }
    }

    fn flush(&self) {
        unsafe { xlib::XFlush(self.display) };
    }
}

impl Drop for CursorSession {
    fn drop(&mut self) {
        self.restore();
        unsafe { xlib::XCloseDisplay(self.display) };
    }
}

fn load_base_cursor(display: *mut xlib::Display, scale_factor: u32) -> Option<BaseCursor> {
    let raw_size = unsafe { xcursor::XcursorGetDefaultSize(display) };
    let default_size = if raw_size > 0 { raw_size as u32 } else { 24 };
    let request_size = default_size.saturating_mul(scale_factor.max(1));
    let theme = unsafe { xcursor::XcursorGetTheme(display) };
    let images = unsafe {
        xcursor::XcursorLibraryLoadImages(c"left_ptr".as_ptr(), theme, request_size as i32)
    };
    if images.is_null() {
        return None;
    }

    let image = unsafe { &**(*images).images };
    let pixel_count = checked_pixel_count(image.width, image.height)?;
    let pixels = unsafe { std::slice::from_raw_parts(image.pixels, pixel_count).to_vec() };
    let base = BaseCursor {
        width: image.width,
        height: image.height,
        xhot: sanitize_hotspot(image.xhot, image.width),
        yhot: sanitize_hotspot(image.yhot, image.height),
        pixels,
        default_size,
    };

    unsafe { xcursor::XcursorImagesDestroy(images) };
    Some(base)
}

fn make_cursor_at_scale(
    display: *mut xlib::Display,
    root: xlib::Window,
    base: &BaseCursor,
    scale: f32,
) -> Option<xlib::Cursor> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let target_size = base.default_size as f32 * scale;
    let factor = target_size / base.width as f32;
    if !factor.is_finite() || factor <= 0.0 {
        return None;
    }
    let requested_width = scaled_dimension(base.width, factor)?;
    let requested_height = scaled_dimension(base.height, factor)?;
    let (max_width, max_height) = best_cursor_size(display, root, requested_width, requested_height);
    let width = requested_width.min(max_width.max(1));
    let height = requested_height.min(max_height.max(1));
    let pixel_count = checked_pixel_count(width, height)?;
    let image =
        unsafe { xcursor::XcursorImageCreate(width.try_into().ok()?, height.try_into().ok()?) };
    if image.is_null() {
        return None;
    }

    let cursor = unsafe {
        (*image).xhot = scaled_hotspot(base.xhot, factor, width);
        (*image).yhot = scaled_hotspot(base.yhot, factor, height);
        let pixels = std::slice::from_raw_parts_mut((*image).pixels, pixel_count);
        scale_bilinear(&base.pixels, base.width, base.height, pixels, width, height);
        let cursor = xcursor::XcursorImageLoadCursor(display, image);
        xcursor::XcursorImageDestroy(image);
        cursor
    };
    if cursor == 0 {
        return None;
    }
    Some(cursor)
}

fn best_cursor_size(
    display: *mut xlib::Display,
    root: xlib::Window,
    width: u32,
    height: u32,
) -> (u32, u32) {
    let mut best_width = width;
    let mut best_height = height;
    unsafe {
        xlib::XQueryBestCursor(
            display,
            root,
            width,
            height,
            &mut best_width,
            &mut best_height,
        );
    }
    (
        sanitize_dimension(best_width),
        sanitize_dimension(best_height),
    )
}

fn apply_to_tree(display: *mut xlib::Display, window: xlib::Window, cursor: xlib::Cursor) {
    let mut stack = vec![window];
    while let Some(window) = stack.pop() {
        unsafe { xlib::XDefineCursor(display, window, cursor) };
        for child in window_children(display, window) {
            stack.push(child);
        }
    }
}

fn clear_tree(display: *mut xlib::Display, window: xlib::Window) {
    let mut stack = vec![window];
    while let Some(window) = stack.pop() {
        unsafe { xlib::XUndefineCursor(display, window) };
        for child in window_children(display, window) {
            stack.push(child);
        }
    }
}

fn restore_root_cursor(display: *mut xlib::Display, root: xlib::Window, base: &BaseCursor) {
    let Some(cursor) = make_cursor_at_scale(display, root, base, 1.0) else {
        return;
    };
    unsafe { xlib::XDefineCursor(display, root, cursor) };
    unsafe { xlib::XFreeCursor(display, cursor) };
}

fn force_cursor_recompute(
    display: *mut xlib::Display,
    root: xlib::Window,
    pointer: Option<PointerState>,
) {
    let Some(pointer) = pointer else {
        return;
    };
    let outside = outside_window_point(display, pointer.child);
    if let Some((x, y)) = outside {
        unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, x, y) };
        unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, pointer.x, pointer.y) };
        return;
    }
    let x2 = nudged_coordinate(pointer.x);
    unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, x2, pointer.y) };
    unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, pointer.x, pointer.y) };
}

fn query_pointer(display: *mut xlib::Display, root: xlib::Window) -> Option<PointerState> {
    let mut root_out = 0;
    let mut child_out = 0;
    let mut root_x = 0;
    let mut root_y = 0;
    let mut window_x = 0;
    let mut window_y = 0;
    let mut mask = 0;
    let status = unsafe {
        xlib::XQueryPointer(
            display,
            root,
            &mut root_out,
            &mut child_out,
            &mut root_x,
            &mut root_y,
            &mut window_x,
            &mut window_y,
            &mut mask,
        )
    };
    if status == 0 {
        return None;
    }
    Some(PointerState {
        x: root_x,
        y: root_y,
        child: child_out,
    })
}

fn outside_window_point(display: *mut xlib::Display, window: xlib::Window) -> Option<(i32, i32)> {
    if window == 0 {
        return None;
    }
    let mut attributes = std::mem::MaybeUninit::<xlib::XWindowAttributes>::uninit();
    let status = unsafe { xlib::XGetWindowAttributes(display, window, attributes.as_mut_ptr()) };
    if status == 0 {
        return None;
    }
    let attributes = unsafe { attributes.assume_init() };
    let screen = unsafe { xlib::XDefaultScreen(display) };
    let screen_width = unsafe { xlib::XDisplayWidth(display, screen) };
    let screen_height = unsafe { xlib::XDisplayHeight(display, screen) };
    if attributes.x > 0 {
        return Some((attributes.x - 1, attributes.y.max(0)));
    }
    let right = attributes.x + attributes.width;
    if right < screen_width {
        return Some((right + 1, attributes.y.max(0)));
    }
    if attributes.y > 0 {
        return Some((attributes.x.max(0), attributes.y - 1));
    }
    let bottom = attributes.y + attributes.height;
    if bottom < screen_height {
        return Some((attributes.x.max(0), bottom + 1));
    }
    None
}

fn nudged_coordinate(value: i32) -> i32 {
    if value > 0 {
        return value - 1;
    }
    value.saturating_add(1)
}

fn window_children(display: *mut xlib::Display, window: xlib::Window) -> Vec<xlib::Window> {
    let mut root = 0;
    let mut parent = 0;
    let mut children: *mut xlib::Window = ptr::null_mut();
    let mut child_count = 0;
    let status = unsafe {
        xlib::XQueryTree(
            display,
            window,
            &mut root,
            &mut parent,
            &mut children,
            &mut child_count,
        )
    };
    if status == 0 || children.is_null() {
        return Vec::new();
    }
    let windows = unsafe { std::slice::from_raw_parts(children, child_count as usize).to_vec() };
    unsafe { xlib::XFree(children as *mut _) };
    windows
}

fn checked_pixel_count(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    width.checked_mul(height)
}

fn scaled_dimension(base: u32, factor: f32) -> Option<u32> {
    let scaled = (base as f32 * factor).round();
    if !scaled.is_finite() || scaled < 1.0 || scaled > i32::MAX as f32 {
        return None;
    }
    Some((scaled as u32).min(MAX_CURSOR_DIMENSION).max(1))
}

fn scaled_hotspot(hotspot: u32, factor: f32, bound: u32) -> u32 {
    let scaled = (hotspot as f32 * factor).round();
    if !scaled.is_finite() || scaled < 0.0 {
        return 0;
    }
    (scaled as u32).min(bound.saturating_sub(1))
}

fn sanitize_hotspot(hotspot: u32, bound: u32) -> u32 {
    hotspot.min(bound.saturating_sub(1))
}

fn sanitize_dimension(value: u32) -> u32 {
    if value == 0 {
        return 1;
    }
    value.min(MAX_CURSOR_DIMENSION)
}
