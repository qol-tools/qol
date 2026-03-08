use std::ptr;
use std::time::Duration;

use anyhow::{ensure, Result};
use x11::{xcursor, xfixes, xlib};

use super::scale::scale_bilinear;

const MAX_CURSOR_DIMENSION: u32 = 512;
const LIVE_REFRESH_DELAY: Duration = Duration::from_millis(8);
const LIVE_REFRESH_ATTEMPTS: u8 = 3;
const XFIXES_CURSOR_NOTIFY: i32 = 1;
const XFIXES_DISPLAY_CURSOR_NOTIFY: i32 = 0;
const XFIXES_DISPLAY_CURSOR_NOTIFY_MASK: libc::c_ulong = 1;
const MAX_IGNORED_CURSOR_SERIALS: usize = 8;

unsafe extern "C" {
    #[link_name = "XFixesSelectCursorInput"]
    fn xfixes_select_cursor_input_raw(
        display: *mut xlib::Display,
        window: xlib::Window,
        event_mask: libc::c_ulong,
    );
}

pub struct CursorSession {
    display: *mut xlib::Display,
    root: xlib::Window,
    base: BaseCursor,
    active_cursor: Option<xlib::Cursor>,
    current_scale: f32,
    grow_cursor: Option<CursorImage>,
    last_pointer: Option<PointerState>,
    restore_cursor: Option<CursorImage>,
    refresh_needs_recompute: bool,
    xfixes_event_base: Option<i32>,
    last_notified_cursor_serial: Option<u64>,
    ignored_cursor_serials: Vec<u64>,
}

#[derive(Clone, Copy)]
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

#[derive(Clone)]
struct CursorImage {
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
        let root = unsafe { xlib::XDefaultRootWindow(display) };

        let Some(base) = load_base_cursor(display, scale_factor) else {
            unsafe { xlib::XCloseDisplay(display) };
            ensure!(false, "failed to load base cursor pixels");
            unreachable!();
        };

        Ok(Self {
            display,
            root,
            base,
            active_cursor: None,
            current_scale: 1.0,
            grow_cursor: None,
            last_pointer: None,
            restore_cursor: None,
            refresh_needs_recompute: false,
            xfixes_event_base: subscribe_cursor_notifications(display, root),
            last_notified_cursor_serial: load_live_cursor_serial(display),
            ignored_cursor_serials: Vec::new(),
        })
    }

    pub fn set_scale(&mut self, scale: f32) -> bool {
        self.current_scale = scale;
        if scale <= 1.0 + f32::EPSILON {
            self.restore();
            return true;
        }
        if self.active_cursor.is_none() {
            self.capture_live_cursors();
        }
        if let Some(grow_cursor) = self.grow_cursor.as_ref() {
            log_scale_plan(
                "grow apply plan",
                self.display,
                self.root,
                grow_cursor.width,
                grow_cursor.height,
                grow_cursor.xhot,
                grow_cursor.yhot,
                grow_cursor.default_size,
                scale,
            );
        }
        if self.grow_cursor.is_none() {
            log_scale_plan(
                "grow apply plan",
                self.display,
                self.root,
                self.base.width,
                self.base.height,
                self.base.xhot,
                self.base.yhot,
                self.base.default_size,
                scale,
            );
        }
        let cursor = if let Some(grow_cursor) = self.grow_cursor.as_ref() {
            make_cursor_from_image_at_scale(self.display, self.root, grow_cursor, scale)
        } else {
            make_cursor_at_scale(self.display, self.root, &self.base, scale)
        };
        let Some(cursor) = cursor else {
            return false;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
        log_raw_live_cursor_state(
            "grow apply raw sample",
            self.display,
            self.base.default_size,
            self.grow_cursor.as_ref(),
            self.restore_cursor.as_ref(),
        );
        log_live_refresh_sample_state(
            "grow apply live sample",
            load_live_cursor_image(self.display, self.base.default_size).as_ref(),
            self.grow_cursor.as_ref(),
            self.restore_cursor.as_ref(),
        );
        self.remember_current_cursor_serial();
        if let Some(old_cursor) = self.active_cursor.replace(cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        true
    }

    pub fn refresh(&mut self) -> bool {
        if self.active_cursor.is_none() {
            return false;
        }
        if self.current_scale <= 1.0 + f32::EPSILON {
            return false;
        }
        let pointer = query_pointer(self.display, self.root);
        let child_changed = pointer.is_some_and(|pointer| pointer_changed_child(self.last_pointer, pointer));
        let cursor_notify_pending = self.poll_cursor_notifications();
        if !child_changed && !cursor_notify_pending {
            self.last_pointer = pointer;
            return false;
        }
        eprintln!(
            "[shake-to-grow] live refresh gate child_changed={child_changed} cursor_notify_pending={cursor_notify_pending}"
        );
        log_refresh_pointer(self.last_pointer, pointer);
        hide_cursor(self.display, self.root);
        if !self.arm_live_refresh(pointer, child_changed || cursor_notify_pending) {
            show_cursor(self.display, self.root);
            self.last_pointer = pointer;
            return false;
        }
        log_raw_live_cursor_state(
            "live refresh pre-hide",
            self.display,
            self.base.default_size,
            self.grow_cursor.as_ref(),
            self.restore_cursor.as_ref(),
        );
        let refreshed = self.finish_live_refresh(query_pointer(self.display, self.root));
        show_cursor(self.display, self.root);
        log_raw_live_cursor_state(
            "live refresh post-show",
            self.display,
            self.base.default_size,
            self.grow_cursor.as_ref(),
            self.restore_cursor.as_ref(),
        );
        self.remember_current_cursor_serial();
        refreshed
    }

    pub fn restore(&mut self) {
        if self.active_cursor.is_none() {
            return;
        }
        let pointer = query_pointer(self.display, self.root);
        if pointer.is_some_and(|pointer| pointer.child != 0) {
            clear_descendants(self.display, self.root);
            restore_root_cursor(self.display, self.root, &self.base, self.restore_cursor.as_ref());
            if let Some(pointer) = pointer {
                nudge_pointer(self.display, self.root, pointer);
            }
            wait_for_cursor_recompute(self.display);
        }
        if pointer.is_none() || pointer.is_some_and(|pointer| pointer.child == 0) {
            clear_tree(self.display, self.root);
            restore_root_cursor(self.display, self.root, &self.base, self.restore_cursor.as_ref());
        }
        self.flush();
        self.remember_current_cursor_serial();
        if let Some(cursor) = self.active_cursor.take() {
            unsafe { xlib::XFreeCursor(self.display, cursor) };
        }
        self.current_scale = 1.0;
        self.grow_cursor = None;
        self.last_pointer = None;
        self.restore_cursor = None;
        self.refresh_needs_recompute = false;
    }

    fn flush(&self) {
        unsafe { xlib::XFlush(self.display) };
    }

    fn capture_live_cursors(&mut self) {
        let live_cursor = load_live_cursor_image(self.display, self.base.default_size);
        let Some(live_cursor) = live_cursor else {
            eprintln!("[shake-to-grow] failed to capture live cursor at grow-start");
            return;
        };
        log_cursor_image("captured live cursor", &live_cursor);
        self.grow_cursor = Some(live_cursor.clone());
        self.restore_cursor = Some(live_cursor);
        self.last_pointer = query_pointer(self.display, self.root);
        self.remember_current_cursor_serial();
    }

    fn arm_live_refresh(&mut self, pointer: Option<PointerState>, recompute_needed: bool) -> bool {
        let Some(pointer) = pointer else {
            eprintln!("[shake-to-grow] live refresh skipped: pointer unavailable");
            return false;
        };
        if pointer.child == 0 {
            eprintln!("[shake-to-grow] live refresh skipped: pointer on root");
            return false;
        }
        self.refresh_needs_recompute = recompute_needed;
        eprintln!(
            "[shake-to-grow] live refresh armed child={} pos=({}, {}) recompute_needed={}",
            pointer.child,
            pointer.x,
            pointer.y,
            self.refresh_needs_recompute,
        );
        log_optional_cursor_image("live refresh root mask", self.restore_cursor.as_ref());
        log_optional_cursor_image("live refresh current grow", self.grow_cursor.as_ref());
        restore_root_cursor(self.display, self.root, &self.base, self.restore_cursor.as_ref());
        clear_descendants(self.display, self.root);
        self.flush();
        log_raw_live_cursor_state(
            "live refresh armed sample",
            self.display,
            self.base.default_size,
            self.grow_cursor.as_ref(),
            self.restore_cursor.as_ref(),
        );
        self.remember_current_cursor_serial();
        true
    }

    fn finish_live_refresh(&mut self, pointer: Option<PointerState>) -> bool {
        let previous_cursor = self.active_cursor;
        if pointer.is_some_and(|pointer| pointer.child == 0) {
            self.refresh_needs_recompute = false;
            self.last_pointer = pointer;
            self.reapply_existing_cursor(previous_cursor);
            return false;
        }
        let mut settled_pointer = pointer;
        if let Some(pointer) = pointer {
            let child_changed =
                self.refresh_needs_recompute || pointer_changed_child(self.last_pointer, pointer);
            eprintln!(
                "[shake-to-grow] live refresh sampling child={} pos=({}, {})",
                pointer.child,
                pointer.x,
                pointer.y,
            );
            eprintln!("[shake-to-grow] live refresh child_changed={child_changed}");
            if child_changed {
                eprintln!("[shake-to-grow] live refresh mode=recompute");
                force_cursor_recompute(self.display, self.root, pointer);
                wait_for_cursor_recompute(self.display);
            }
            if !child_changed {
                eprintln!("[shake-to-grow] live refresh mode=notify");
                wait_for_cursor_recompute(self.display);
            }
            settled_pointer = query_pointer(self.display, self.root);
            log_refresh_pointer_state("live refresh post-recompute", settled_pointer);
            let immediate_sample = load_live_cursor_image(self.display, self.base.default_size);
            log_live_refresh_sample_state(
                "live refresh immediate sample",
                immediate_sample.as_ref(),
                self.grow_cursor.as_ref(),
                self.restore_cursor.as_ref(),
            );
        }
        eprintln!("[shake-to-grow] live refresh sampling");
        let sample = stable_live_cursor_sample(
            self.display,
            self.base.default_size,
            self.grow_cursor.as_ref(),
        );
        let Some(sample) = sample else {
            self.last_pointer = settled_pointer;
            return self.handle_failed_refresh_sample(previous_cursor);
        };
        if sampled_our_scaled_cursor(self.grow_cursor.as_ref(), &sample) {
            log_cursor_image("live refresh rejected scaled sample", &sample);
            self.last_pointer = settled_pointer;
            return self.handle_failed_refresh_sample(previous_cursor);
        }
        self.refresh_needs_recompute = false;
        if self.grow_cursor.as_ref().is_some_and(|current| same_cursor_image(current, &sample)) {
            log_cursor_image("live refresh unchanged", &sample);
            self.last_pointer = settled_pointer;
            self.reapply_existing_cursor(previous_cursor);
            return false;
        }
        log_cursor_image("live refresh apply", &sample);
        self.restore_cursor = Some(sample.clone());
        self.grow_cursor = Some(sample);
        let next_cursor = self
            .grow_cursor
            .as_ref()
            .and_then(|image| {
                make_cursor_from_image_at_scale(self.display, self.root, image, self.current_scale)
            });
        let Some(next_cursor) = next_cursor else {
            eprintln!("[shake-to-grow] live refresh failed to build scaled cursor");
            self.last_pointer = settled_pointer;
            self.reapply_existing_cursor(previous_cursor);
            return false;
        };
        apply_to_tree(self.display, self.root, next_cursor);
        self.flush();
        if let Some(old_cursor) = self.active_cursor.replace(next_cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        self.last_pointer = settled_pointer;
        true
    }

    fn handle_failed_refresh_sample(&mut self, cursor: Option<xlib::Cursor>) -> bool {
        self.refresh_needs_recompute = false;
        self.reapply_existing_cursor(cursor);
        false
    }

    fn reapply_existing_cursor(&mut self, cursor: Option<xlib::Cursor>) {
        let Some(cursor) = cursor else {
            return;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
        log_raw_live_cursor_state(
            "live refresh reapply raw sample",
            self.display,
            self.base.default_size,
            self.grow_cursor.as_ref(),
            self.restore_cursor.as_ref(),
        );
        log_live_refresh_sample_state(
            "live refresh reapply sample",
            load_live_cursor_image(self.display, self.base.default_size).as_ref(),
            self.grow_cursor.as_ref(),
            self.restore_cursor.as_ref(),
        );
        self.remember_current_cursor_serial();
    }

    fn poll_cursor_notifications(&mut self) -> bool {
        let Some(event_base) = self.xfixes_event_base else {
            return false;
        };
        let mut pending = false;
        while unsafe { xlib::XPending(self.display) } > 0 {
            let mut event = std::mem::MaybeUninit::<xlib::XEvent>::uninit();
            unsafe { xlib::XNextEvent(self.display, event.as_mut_ptr()) };
            let event = unsafe { event.assume_init() };
            let event_type = event.get_type();
            if event_type != event_base + XFIXES_CURSOR_NOTIFY {
                continue;
            }
            let notify =
                unsafe { *(&event as *const xlib::XEvent as *const xfixes::XFixesCursorNotifyEvent) };
            if notify.subtype != XFIXES_DISPLAY_CURSOR_NOTIFY {
                continue;
            }
            let serial = notify.cursor_serial as u64;
            if self.ignored_cursor_serials.contains(&serial) {
                eprintln!("[shake-to-grow] live refresh cursor-notify ignored serial={serial} reason=self");
                continue;
            }
            if self
                .last_notified_cursor_serial
                .is_some_and(|last| last == serial)
            {
                eprintln!(
                    "[shake-to-grow] live refresh cursor-notify ignored serial={serial} reason=duplicate"
                );
                continue;
            }
            eprintln!("[shake-to-grow] live refresh cursor-notify pending serial={serial}");
            self.last_notified_cursor_serial = Some(serial);
            pending = true;
        }
        pending
    }

    fn remember_current_cursor_serial(&mut self) {
        let Some(serial) = load_live_cursor_serial(self.display) else {
            return;
        };
        remember_cursor_serial(&mut self.ignored_cursor_serials, serial);
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

fn clear_descendants(display: *mut xlib::Display, window: xlib::Window) {
    let mut stack = window_children(display, window);
    while let Some(window) = stack.pop() {
        unsafe { xlib::XUndefineCursor(display, window) };
        for child in window_children(display, window) {
            stack.push(child);
        }
    }
}

fn restore_root_cursor(
    display: *mut xlib::Display,
    root: xlib::Window,
    base: &BaseCursor,
    restore_cursor: Option<&CursorImage>,
) {
    let cursor = if let Some(restore_cursor) = restore_cursor {
        make_cursor_from_image(display, restore_cursor)
    } else {
        make_cursor_at_scale(display, root, base, 1.0)
    };
    let Some(cursor) = cursor else {
        return;
    };
    unsafe { xlib::XDefineCursor(display, root, cursor) };
    unsafe { xlib::XFreeCursor(display, cursor) };
}

fn make_cursor_from_image(
    display: *mut xlib::Display,
    image: &CursorImage,
) -> Option<xlib::Cursor> {
    let pixel_count = checked_pixel_count(image.width, image.height)?;
    let cursor_image = unsafe {
        xcursor::XcursorImageCreate(image.width.try_into().ok()?, image.height.try_into().ok()?)
    };
    if cursor_image.is_null() {
        return None;
    }
    let cursor = unsafe {
        (*cursor_image).xhot = image.xhot;
        (*cursor_image).yhot = image.yhot;
        let pixels = std::slice::from_raw_parts_mut((*cursor_image).pixels, pixel_count);
        pixels.copy_from_slice(&image.pixels);
        let cursor = xcursor::XcursorImageLoadCursor(display, cursor_image);
        xcursor::XcursorImageDestroy(cursor_image);
        cursor
    };
    if cursor == 0 {
        return None;
    }
    Some(cursor)
}

fn make_cursor_from_image_at_scale(
    display: *mut xlib::Display,
    root: xlib::Window,
    image: &CursorImage,
    scale: f32,
) -> Option<xlib::Cursor> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let target_size = image.default_size as f32 * scale;
    let factor = target_size / image.width as f32;
    if !factor.is_finite() || factor <= 0.0 {
        return None;
    }
    let requested_width = scaled_dimension(image.width, factor)?;
    let requested_height = scaled_dimension(image.height, factor)?;
    let (max_width, max_height) =
        best_cursor_size(display, root, requested_width, requested_height);
    let width = requested_width.min(max_width.max(1));
    let height = requested_height.min(max_height.max(1));
    let pixel_count = checked_pixel_count(width, height)?;
    let cursor_image =
        unsafe { xcursor::XcursorImageCreate(width.try_into().ok()?, height.try_into().ok()?) };
    if cursor_image.is_null() {
        return None;
    }
    let cursor = unsafe {
        (*cursor_image).xhot = scaled_hotspot(image.xhot, factor, width);
        (*cursor_image).yhot = scaled_hotspot(image.yhot, factor, height);
        let pixels = std::slice::from_raw_parts_mut((*cursor_image).pixels, pixel_count);
        scale_bilinear(&image.pixels, image.width, image.height, pixels, width, height);
        let cursor = xcursor::XcursorImageLoadCursor(display, cursor_image);
        xcursor::XcursorImageDestroy(cursor_image);
        cursor
    };
    if cursor == 0 {
        return None;
    }
    Some(cursor)
}

fn load_live_cursor_image(display: *mut xlib::Display, default_size: u32) -> Option<CursorImage> {
    let image = unsafe { xfixes::XFixesGetCursorImage(display) };
    if image.is_null() {
        return None;
    }
    let image_ref = unsafe { &*image };
    let width = u32::from(image_ref.width);
    let height = u32::from(image_ref.height);
    let Some(pixel_count) = checked_pixel_count(width, height) else {
        unsafe { xlib::XFree(image as *mut _) };
        return None;
    };
    let pixels = unsafe { std::slice::from_raw_parts(image_ref.pixels, pixel_count) }
        .iter()
        .map(|pixel| *pixel as u32)
        .collect::<Vec<_>>();
    let cursor = CursorImage {
        width,
        height,
        xhot: sanitize_hotspot(u32::from(image_ref.xhot), width),
        yhot: sanitize_hotspot(u32::from(image_ref.yhot), height),
        pixels,
        default_size,
    };
    unsafe { xlib::XFree(image as *mut _) };
    Some(cursor)
}

fn load_live_cursor_serial(display: *mut xlib::Display) -> Option<u64> {
    let image = unsafe { xfixes::XFixesGetCursorImage(display) };
    if image.is_null() {
        return None;
    }
    let serial = unsafe { (*image).cursor_serial as u64 };
    unsafe { xlib::XFree(image as *mut _) };
    Some(serial)
}

fn sync(display: *mut xlib::Display) {
    unsafe { xlib::XSync(display, xlib::False) };
}

fn stable_live_cursor_sample(
    display: *mut xlib::Display,
    default_size: u32,
    current_grow_cursor: Option<&CursorImage>,
) -> Option<CursorImage> {
    let mut previous = None;
    let mut attempts = 0;
    while attempts < LIVE_REFRESH_ATTEMPTS {
        sync(display);
        std::thread::sleep(LIVE_REFRESH_DELAY);
        sync(display);
        let current = load_live_cursor_image(display, default_size);
        let Some(current) = current else {
            eprintln!(
                "[shake-to-grow] live refresh attempt={} sample=none",
                attempts + 1,
            );
            attempts += 1;
            continue;
        };
        if is_empty_cursor(&current) {
            eprintln!(
                "[shake-to-grow] live refresh attempt={} ignored=empty",
                attempts + 1,
            );
            attempts += 1;
            continue;
        }
        if sampled_our_scaled_cursor(current_grow_cursor, &current) {
            eprintln!(
                "[shake-to-grow] live refresh attempt={} ignored=self-sample",
                attempts + 1,
            );
            attempts += 1;
            continue;
        }
        log_cursor_image_with_attempt(
            "live refresh attempt",
            usize::from(attempts + 1),
            &current,
        );
        if previous
            .as_ref()
            .is_some_and(|older| same_cursor_image(older, &current))
        {
            log_cursor_image_with_attempt(
                "live refresh stabilized",
                usize::from(attempts + 1),
                &current,
            );
            return Some(current);
        }
        previous = Some(current);
        attempts += 1;
    }
    eprintln!("[shake-to-grow] live refresh failed to stabilize");
    None
}

fn wait_for_cursor_recompute(display: *mut xlib::Display) {
    sync(display);
    std::thread::sleep(LIVE_REFRESH_DELAY);
    sync(display);
}

fn hide_cursor(display: *mut xlib::Display, root: xlib::Window) {
    eprintln!("[shake-to-grow] live refresh cursor_visibility=hide");
    unsafe { xfixes::XFixesHideCursor(display, root) };
    sync(display);
}

fn show_cursor(display: *mut xlib::Display, root: xlib::Window) {
    eprintln!("[shake-to-grow] live refresh cursor_visibility=show");
    unsafe { xfixes::XFixesShowCursor(display, root) };
    sync(display);
}

fn force_cursor_recompute(display: *mut xlib::Display, root: xlib::Window, pointer: PointerState) {
    let outside = outside_window_point(display, pointer.child);
    if let Some((x, y)) = outside {
        eprintln!(
            "[shake-to-grow] live refresh recompute path=outside from=({}, {}) outside=({}, {})",
            pointer.x,
            pointer.y,
            x,
            y,
        );
        unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, x, y) };
        unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, pointer.x, pointer.y) };
        return;
    }
    eprintln!(
        "[shake-to-grow] live refresh recompute path=nudge from=({}, {})",
        pointer.x,
        pointer.y,
    );
    nudge_pointer(display, root, pointer);
}

fn nudge_pointer(display: *mut xlib::Display, root: xlib::Window, pointer: PointerState) {
    let x2 = nudged_coordinate(pointer.x);
    unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, x2, pointer.y) };
    unsafe { xlib::XWarpPointer(display, 0, root, 0, 0, 0, 0, pointer.x, pointer.y) };
}

fn query_pointer(display: *mut xlib::Display, root: xlib::Window) -> Option<PointerState> {
    let (x, y, mut child) = query_pointer_at(display, root)?;
    if child == 0 {
        return Some(PointerState { x, y, child: 0 });
    }
    loop {
        let next = query_pointer_at(display, child);
        let Some((_, _, next_child)) = next else {
            return Some(PointerState { x, y, child });
        };
        if next_child == 0 {
            return Some(PointerState { x, y, child });
        }
        child = next_child;
    }
}

fn query_pointer_at(
    display: *mut xlib::Display,
    window: xlib::Window,
) -> Option<(i32, i32, xlib::Window)> {
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
            window,
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
    Some((root_x, root_y, child_out))
}

fn outside_window_point(display: *mut xlib::Display, window: xlib::Window) -> Option<(i32, i32)> {
    if window == 0 {
        return None;
    }
    let root = unsafe { xlib::XDefaultRootWindow(display) };
    let mut attributes = std::mem::MaybeUninit::<xlib::XWindowAttributes>::uninit();
    let status = unsafe { xlib::XGetWindowAttributes(display, window, attributes.as_mut_ptr()) };
    if status == 0 {
        return None;
    }
    let attributes = unsafe { attributes.assume_init() };
    let mut root_x = 0;
    let mut root_y = 0;
    let mut child = 0;
    let translated = unsafe {
        xlib::XTranslateCoordinates(
            display,
            window,
            root,
            0,
            0,
            &mut root_x,
            &mut root_y,
            &mut child,
        )
    };
    if translated == 0 {
        return None;
    }
    let mut root_attributes = std::mem::MaybeUninit::<xlib::XWindowAttributes>::uninit();
    let root_status = unsafe { xlib::XGetWindowAttributes(display, root, root_attributes.as_mut_ptr()) };
    if root_status == 0 {
        return None;
    }
    let root_attributes = unsafe { root_attributes.assume_init() };
    let left = root_x;
    let top = root_y;
    let right = root_x + attributes.width;
    let bottom = root_y + attributes.height;
    if left > 0 {
        return Some((left - 1, top.max(0)));
    }
    if right < root_attributes.width {
        return Some((right + 1, top.max(0)));
    }
    if top > 0 {
        return Some((left.max(0), top - 1));
    }
    if bottom < root_attributes.height {
        return Some((left.max(0), bottom + 1));
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

fn subscribe_cursor_notifications(
    display: *mut xlib::Display,
    root: xlib::Window,
) -> Option<i32> {
    let mut event_base = 0;
    let mut error_base = 0;
    let status = unsafe { xfixes::XFixesQueryExtension(display, &mut event_base, &mut error_base) };
    if status == 0 {
        eprintln!("[shake-to-grow] live refresh cursor-notify unavailable");
        return None;
    }
    unsafe { xfixes_select_cursor_input_raw(display, root, XFIXES_DISPLAY_CURSOR_NOTIFY_MASK) };
    sync(display);
    eprintln!("[shake-to-grow] live refresh cursor-notify subscribed event_base={event_base}");
    Some(event_base)
}

fn remember_cursor_serial(serials: &mut Vec<u64>, serial: u64) {
    serials.retain(|existing| *existing != serial);
    serials.push(serial);
    if serials.len() <= MAX_IGNORED_CURSOR_SERIALS {
        return;
    }
    let drop_count = serials.len() - MAX_IGNORED_CURSOR_SERIALS;
    serials.drain(0..drop_count);
}

fn pointer_changed_child(previous: Option<PointerState>, current: PointerState) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    previous.child != current.child
}

fn same_cursor_image(left: &CursorImage, right: &CursorImage) -> bool {
    if left.width != right.width {
        return false;
    }
    if left.height != right.height {
        return false;
    }
    if left.xhot != right.xhot {
        return false;
    }
    if left.yhot != right.yhot {
        return false;
    }
    left.pixels == right.pixels
}

fn sampled_our_scaled_cursor(previous: Option<&CursorImage>, sample: &CursorImage) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if sample.width > previous.width.saturating_mul(2) {
        return true;
    }
    sample.height > previous.height.saturating_mul(2)
}

fn is_empty_cursor(image: &CursorImage) -> bool {
    image.pixels.iter().all(|pixel| ((pixel >> 24) & 0xFF) == 0)
}

fn log_refresh_pointer(previous: Option<PointerState>, current: Option<PointerState>) {
    let previous = format_pointer_state(previous);
    let current = format_pointer_state(current);
    eprintln!("[shake-to-grow] live refresh pointer previous={previous} current={current}");
}

fn log_cursor_image(prefix: &str, image: &CursorImage) {
    eprintln!(
        "[shake-to-grow] {prefix}: size={}x{} hot=({}, {}) hash={:016x}",
        image.width,
        image.height,
        image.xhot,
        image.yhot,
        cursor_hash(image),
    );
}

fn log_cursor_image_with_attempt(prefix: &str, attempt: usize, image: &CursorImage) {
    eprintln!(
        "[shake-to-grow] {prefix}={} size={}x{} hot=({}, {}) hash={:016x}",
        attempt,
        image.width,
        image.height,
        image.xhot,
        image.yhot,
        cursor_hash(image),
    );
}

fn log_optional_cursor_image(prefix: &str, image: Option<&CursorImage>) {
    let Some(image) = image else {
        eprintln!("[shake-to-grow] {prefix}: none");
        return;
    };
    log_cursor_image(prefix, image);
}

fn log_live_refresh_sample_state(
    prefix: &str,
    sample: Option<&CursorImage>,
    grow_cursor: Option<&CursorImage>,
    restore_cursor: Option<&CursorImage>,
) {
    let Some(sample) = sample else {
        eprintln!("[shake-to-grow] {prefix}: none");
        return;
    };
    eprintln!(
        "[shake-to-grow] {prefix}: size={}x{} hot=({}, {}) hash={:016x} matches_grow={} matches_restore={}",
        sample.width,
        sample.height,
        sample.xhot,
        sample.yhot,
        cursor_hash(sample),
        grow_cursor.is_some_and(|grow| same_cursor_image(grow, sample)),
        restore_cursor.is_some_and(|restore| same_cursor_image(restore, sample)),
    );
}

fn log_raw_live_cursor_state(
    prefix: &str,
    display: *mut xlib::Display,
    default_size: u32,
    grow_cursor: Option<&CursorImage>,
    restore_cursor: Option<&CursorImage>,
) {
    let image = unsafe { xfixes::XFixesGetCursorImage(display) };
    if image.is_null() {
        eprintln!("[shake-to-grow] {prefix}: none");
        return;
    }
    let image_ref = unsafe { &*image };
    let width = u32::from(image_ref.width);
    let height = u32::from(image_ref.height);
    let Some(pixel_count) = checked_pixel_count(width, height) else {
        eprintln!(
            "[shake-to-grow] {prefix}: invalid size={}x{} serial={} atom={}",
            width,
            height,
            image_ref.cursor_serial,
            image_ref.atom,
        );
        unsafe { xlib::XFree(image as *mut _) };
        return;
    };
    let pixels = unsafe { std::slice::from_raw_parts(image_ref.pixels, pixel_count) }
        .iter()
        .map(|pixel| *pixel as u32)
        .collect::<Vec<_>>();
    let cursor = CursorImage {
        width,
        height,
        xhot: sanitize_hotspot(u32::from(image_ref.xhot), width),
        yhot: sanitize_hotspot(u32::from(image_ref.yhot), height),
        pixels,
        default_size,
    };
    eprintln!(
        "[shake-to-grow] {prefix}: serial={} atom={} size={}x{} hot=({}, {}) hash={:016x} matches_grow={} matches_restore={}",
        image_ref.cursor_serial,
        image_ref.atom,
        cursor.width,
        cursor.height,
        cursor.xhot,
        cursor.yhot,
        cursor_hash(&cursor),
        grow_cursor.is_some_and(|grow| same_cursor_image(grow, &cursor)),
        restore_cursor.is_some_and(|restore| same_cursor_image(restore, &cursor)),
    );
    unsafe { xlib::XFree(image as *mut _) };
}

fn log_scale_plan(
    prefix: &str,
    display: *mut xlib::Display,
    root: xlib::Window,
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    default_size: u32,
    scale: f32,
) {
    if !scale.is_finite() || scale <= 0.0 {
        eprintln!("[shake-to-grow] {prefix}: invalid_scale={scale}");
        return;
    }
    let target_size = default_size as f32 * scale;
    let factor = target_size / width as f32;
    let Some(requested_width) = scaled_dimension(width, factor) else {
        eprintln!("[shake-to-grow] {prefix}: invalid_requested_width factor={factor}");
        return;
    };
    let Some(requested_height) = scaled_dimension(height, factor) else {
        eprintln!("[shake-to-grow] {prefix}: invalid_requested_height factor={factor}");
        return;
    };
    let (max_width, max_height) = best_cursor_size(display, root, requested_width, requested_height);
    let final_width = requested_width.min(max_width.max(1));
    let final_height = requested_height.min(max_height.max(1));
    let final_xhot = scaled_hotspot(xhot, factor, final_width);
    let final_yhot = scaled_hotspot(yhot, factor, final_height);
    eprintln!(
        "[shake-to-grow] {prefix}: scale={scale:.3} source={}x{} hot=({}, {}) requested={}x{} max={}x{} final={}x{} final_hot=({}, {})",
        width,
        height,
        xhot,
        yhot,
        requested_width,
        requested_height,
        max_width,
        max_height,
        final_width,
        final_height,
        final_xhot,
        final_yhot,
    );
}

fn log_refresh_pointer_state(prefix: &str, pointer: Option<PointerState>) {
    let pointer = format_pointer_state(pointer);
    eprintln!("[shake-to-grow] {prefix}={pointer}");
}

fn format_pointer_state(pointer: Option<PointerState>) -> String {
    pointer
        .map(|pointer| format!("child={} pos=({}, {})", pointer.child, pointer.x, pointer.y))
        .unwrap_or_else(|| "none".to_string())
}

fn cursor_hash(image: &CursorImage) -> u64 {
    let mut hash = 1469598103934665603u64;
    hash = hash_cursor_value(hash, u64::from(image.width));
    hash = hash_cursor_value(hash, u64::from(image.height));
    hash = hash_cursor_value(hash, u64::from(image.xhot));
    hash = hash_cursor_value(hash, u64::from(image.yhot));
    for pixel in &image.pixels {
        hash = hash_cursor_value(hash, u64::from(*pixel));
    }
    hash
}

fn hash_cursor_value(hash: u64, value: u64) -> u64 {
    let hash = hash ^ value;
    hash.wrapping_mul(1099511628211)
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
