use std::ffi::{CStr, CString};
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
const MAX_IGNORED_CURSOR_SERIALS: usize = 32;

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
    preferred_source_size: u32,
    active_cursor: Option<xlib::Cursor>,
    applied_cursor: Option<CursorImage>,
    current_scale: f32,
    grow_cursor: Option<CursorImage>,
    last_pointer: Option<PointerState>,
    refresh_mode: RefreshMode,
    xfixes_event_base: Option<i32>,
    last_notified_cursor_serial: Option<u64>,
    ignored_cursor_serials: Vec<u64>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RefreshMode {
    None,
    Wait,
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
    source: Option<CursorRaster>,
}

#[derive(Clone)]
struct CursorRaster {
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    pixels: Vec<u32>,
}

#[derive(Clone)]
struct CursorImage {
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    pixels: Vec<u32>,
    default_size: u32,
    name: Option<String>,
    source: Option<CursorRaster>,
}

unsafe extern "C" fn log_x_error(
    _display: *mut xlib::Display,
    event: *mut xlib::XErrorEvent,
) -> libc::c_int {
    let (code, request) = unsafe { ((*event).error_code, (*event).request_code) };
    eprintln!("[shake-to-grow] ignored X error code={code} request={request}");
    0
}

impl CursorSession {
    pub fn open(scale_factor: u32) -> Result<Self> {
        unsafe { xlib::XSetErrorHandler(Some(log_x_error)) };
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
            preferred_source_size: preferred_source_size(base.default_size, scale_factor),
            base,
            active_cursor: None,
            applied_cursor: None,
            current_scale: 1.0,
            grow_cursor: None,
            last_pointer: None,
            refresh_mode: RefreshMode::None,
            xfixes_event_base: subscribe_cursor_notifications(display, root),
            last_notified_cursor_serial: live_cursor_serial(display),
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
        let expected_applied_cursor = self.grow_cursor.as_ref().and_then(|grow_cursor| {
            scale_cursor_image_for_display(self.display, self.root, grow_cursor, scale)
        });
        let cursor = if let Some(expected_applied_cursor) = expected_applied_cursor.as_ref() {
            make_cursor_from_image(self.display, expected_applied_cursor)
        } else {
            make_cursor_at_scale(self.display, self.root, &self.base, scale)
        };
        let Some(cursor) = cursor else {
            return false;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
        self.applied_cursor = expected_applied_cursor
            .or_else(|| load_live_cursor_image(self.display, self.base.default_size));
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
        if !live_refresh_enabled() {
            self.drain_cursor_notifications();
            self.last_pointer = query_pointer(self.display, self.root);
            self.refresh_mode = RefreshMode::None;
            return false;
        }
        let pointer = query_pointer(self.display, self.root);
        let child_changed =
            pointer.is_some_and(|pointer| pointer_changed_child(self.last_pointer, pointer));
        let cursor_notify_pending = self.poll_cursor_notifications();
        if !child_changed && !cursor_notify_pending {
            self.last_pointer = pointer;
            return false;
        }
        if self.try_apply_visible_refresh(pointer) {
            return true;
        }
        eprintln!(
            "[shake-to-grow] live refresh gate child_changed={child_changed} cursor_notify_pending={cursor_notify_pending}"
        );
        log_refresh_pointer(self.last_pointer, pointer);
        if !self.arm_live_refresh(pointer, child_changed, cursor_notify_pending) {
            self.last_pointer = pointer;
            return false;
        }
        let refreshed = self.finish_live_refresh(query_pointer(self.display, self.root));
        self.remember_current_cursor_serial();
        refreshed
    }

    pub fn restore(&mut self) {
        if self.active_cursor.is_none() {
            return;
        }
        clear_tree(self.display, self.root);
        restore_root_cursor(self.display, self.root, &self.base);
        self.flush();
        self.remember_current_cursor_serial();
        if let Some(cursor) = self.active_cursor.take() {
            unsafe { xlib::XFreeCursor(self.display, cursor) };
        }
        self.current_scale = 1.0;
        self.applied_cursor = None;
        self.grow_cursor = None;
        self.last_pointer = None;
        self.refresh_mode = RefreshMode::None;
    }

    fn flush(&self) {
        sync(self.display);
    }

    fn capture_live_cursors(&mut self) {
        let live_cursor =
            load_live_cursor_image(self.display, self.base.default_size).map(|cursor| {
                with_best_source(self.display, &self.base, cursor, self.preferred_source_size)
            });
        let Some(live_cursor) = live_cursor else {
            eprintln!("[shake-to-grow] failed to capture live cursor at grow-start");
            return;
        };
        log_cursor_image("captured live cursor", &live_cursor);
        self.grow_cursor = Some(live_cursor);
        self.last_pointer = query_pointer(self.display, self.root);
        self.remember_current_cursor_serial();
    }

    fn arm_live_refresh(
        &mut self,
        pointer: Option<PointerState>,
        child_changed: bool,
        cursor_notify_pending: bool,
    ) -> bool {
        let Some(pointer) = pointer else {
            eprintln!("[shake-to-grow] live refresh skipped: pointer unavailable");
            return false;
        };
        if pointer.child == 0 {
            eprintln!("[shake-to-grow] live refresh skipped: pointer on root");
            return false;
        }
        mask_root_for_refresh(
            self.display,
            self.root,
            &self.base,
            self.grow_cursor.as_ref(),
        );
        clear_descendants(self.display, self.root);
        self.flush();
        let armed_sample = load_live_cursor_image(self.display, self.base.default_size);
        let armed_change = armed_sample_indicates_change(
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
            armed_sample.as_ref(),
        );
        self.refresh_mode = choose_refresh_mode(child_changed, cursor_notify_pending, armed_change);
        if self.refresh_mode == RefreshMode::None {
            self.remember_current_cursor_serial();
            return false;
        }
        eprintln!(
            "[shake-to-grow] live refresh armed child={} pos=({}, {}) mode={}",
            pointer.child,
            pointer.x,
            pointer.y,
            refresh_mode_label(self.refresh_mode),
        );
        self.remember_current_cursor_serial();
        true
    }

    fn finish_live_refresh(&mut self, pointer: Option<PointerState>) -> bool {
        let previous_cursor = self.active_cursor;
        let refresh_mode = self.refresh_mode;
        if pointer.is_some_and(|pointer| pointer.child == 0) {
            self.refresh_mode = RefreshMode::None;
            self.last_pointer = pointer;
            self.reapply_existing_cursor(previous_cursor);
            return false;
        }
        let mut settled_pointer = pointer;
        let mut immediate_sample = None;
        if let Some(pointer) = pointer {
            eprintln!(
                "[shake-to-grow] live refresh sampling child={} pos=({}, {})",
                pointer.child, pointer.x, pointer.y,
            );
            if refresh_mode == RefreshMode::Wait {
                wait_for_cursor_recompute(self.display);
            }
            settled_pointer = query_pointer(self.display, self.root);
            immediate_sample = load_live_cursor_image(self.display, self.base.default_size);
            log_live_refresh_sample_state(
                "live refresh immediate sample",
                immediate_sample.as_ref(),
                self.grow_cursor.as_ref(),
                self.applied_cursor.as_ref(),
            );
        }
        let sample = stable_live_cursor_sample(
            self.display,
            self.base.default_size,
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
        )
        .map(|sample| {
            with_best_source(self.display, &self.base, sample, self.preferred_source_size)
        });
        let Some(sample) = sample else {
            self.last_pointer = settled_pointer;
            return self.handle_failed_refresh_sample(previous_cursor);
        };
        if is_our_enlarged_cursor(
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
            &sample,
        ) {
            log_cursor_image("live refresh rejected scaled sample", &sample);
            self.last_pointer = settled_pointer;
            return self.handle_failed_refresh_sample(previous_cursor);
        }
        if !refresh_sample_persisted(
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
            immediate_sample.as_ref(),
            &sample,
        ) {
            self.last_pointer = settled_pointer;
            return self.handle_failed_refresh_sample(previous_cursor);
        }
        self.refresh_mode = RefreshMode::None;
        if self
            .grow_cursor
            .as_ref()
            .is_some_and(|current| same_cursor_image(current, &sample))
        {
            log_cursor_image("live refresh unchanged", &sample);
            self.last_pointer = settled_pointer;
            return self.handle_failed_refresh_sample(previous_cursor);
        }
        log_cursor_image("live refresh apply", &sample);
        self.grow_cursor = Some(sample);
        let expected_applied_cursor = self.grow_cursor.as_ref().and_then(|image| {
            scale_cursor_image_for_display(self.display, self.root, image, self.current_scale)
        });
        let next_cursor = expected_applied_cursor
            .as_ref()
            .and_then(|image| make_cursor_from_image(self.display, image));
        let Some(next_cursor) = next_cursor else {
            eprintln!("[shake-to-grow] live refresh failed to build scaled cursor");
            self.last_pointer = settled_pointer;
            self.reapply_existing_cursor(previous_cursor);
            return false;
        };
        apply_to_tree(self.display, self.root, next_cursor);
        self.flush();
        self.applied_cursor = expected_applied_cursor;
        if let Some(old_cursor) = self.active_cursor.replace(next_cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        self.last_pointer = settled_pointer;
        true
    }

    fn handle_failed_refresh_sample(&mut self, cursor: Option<xlib::Cursor>) -> bool {
        self.refresh_mode = RefreshMode::None;
        self.reapply_existing_cursor(cursor);
        self.remember_current_cursor_serial();
        false
    }

    fn try_apply_visible_refresh(&mut self, pointer: Option<PointerState>) -> bool {
        let sample = load_live_cursor_image(self.display, self.base.default_size).map(|sample| {
            with_best_source(self.display, &self.base, sample, self.preferred_source_size)
        });
        log_live_refresh_sample_state(
            "live refresh visible sample",
            sample.as_ref(),
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
        );
        let Some(sample) = sample else {
            return false;
        };
        if !is_distinct_live_candidate(
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
            &sample,
        ) {
            return false;
        }
        eprintln!("[shake-to-grow] live refresh fast-path=visible");
        self.apply_refresh_sample(pointer, sample)
    }

    fn apply_refresh_sample(&mut self, pointer: Option<PointerState>, sample: CursorImage) -> bool {
        self.refresh_mode = RefreshMode::None;
        log_cursor_image("live refresh apply", &sample);
        self.grow_cursor = Some(sample);
        let expected_applied_cursor = self.grow_cursor.as_ref().and_then(|image| {
            scale_cursor_image_for_display(self.display, self.root, image, self.current_scale)
        });
        let next_cursor = expected_applied_cursor
            .as_ref()
            .and_then(|image| make_cursor_from_image(self.display, image));
        let Some(next_cursor) = next_cursor else {
            eprintln!("[shake-to-grow] live refresh failed to build scaled cursor");
            self.last_pointer = pointer;
            return false;
        };
        apply_to_tree(self.display, self.root, next_cursor);
        self.flush();
        self.applied_cursor = expected_applied_cursor;
        if let Some(old_cursor) = self.active_cursor.replace(next_cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        self.last_pointer = pointer;
        self.remember_current_cursor_serial();
        true
    }

    fn reapply_existing_cursor(&mut self, cursor: Option<xlib::Cursor>) {
        let Some(cursor) = cursor else {
            return;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
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
            let notify = unsafe {
                *(&event as *const xlib::XEvent as *const xfixes::XFixesCursorNotifyEvent)
            };
            if notify.subtype != XFIXES_DISPLAY_CURSOR_NOTIFY {
                continue;
            }
            let serial = notify.cursor_serial;
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

    fn drain_cursor_notifications(&mut self) {
        let Some(event_base) = self.xfixes_event_base else {
            return;
        };
        while unsafe { xlib::XPending(self.display) } > 0 {
            let mut event = std::mem::MaybeUninit::<xlib::XEvent>::uninit();
            unsafe { xlib::XNextEvent(self.display, event.as_mut_ptr()) };
            let event = unsafe { event.assume_init() };
            let event_type = event.get_type();
            if event_type != event_base + XFIXES_CURSOR_NOTIFY {
                continue;
            }
        }
    }

    fn remember_current_cursor_serial(&mut self) {
        let Some((sample, serial)) =
            load_live_cursor_snapshot(self.display, self.base.default_size)
        else {
            return;
        };
        if !is_our_cursor_serial(
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
            &sample,
        ) {
            return;
        }
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
    let logical = load_named_cursor_raster(display, c"left_ptr", default_size)?;
    let source_size = preferred_source_size(default_size, scale_factor);
    let source = load_named_cursor_raster(display, c"left_ptr", source_size)
        .filter(|source| source_improves_cursor(logical.width, logical.height, source));
    let base = BaseCursor {
        width: logical.width,
        height: logical.height,
        xhot: logical.xhot,
        yhot: logical.yhot,
        pixels: logical.pixels,
        default_size,
        source,
    };
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
    let factor = scale;
    if factor <= 0.0 {
        return None;
    }
    let requested_width = scaled_dimension(base.width, factor)?;
    let requested_height = scaled_dimension(base.height, factor)?;
    let (max_width, max_height) =
        best_cursor_size(display, root, requested_width, requested_height);
    let width = requested_width.min(max_width.max(1));
    let height = requested_height.min(max_height.max(1));
    let pixel_count = checked_pixel_count(width, height)?;
    let (source_width, source_height, source_xhot, source_yhot, source_pixels) = base
        .source
        .as_ref()
        .map(|source| {
            (
                source.width,
                source.height,
                source.xhot,
                source.yhot,
                source.pixels.as_slice(),
            )
        })
        .unwrap_or((
            base.width,
            base.height,
            base.xhot,
            base.yhot,
            base.pixels.as_slice(),
        ));
    let image =
        unsafe { xcursor::XcursorImageCreate(width.try_into().ok()?, height.try_into().ok()?) };
    if image.is_null() {
        return None;
    }

    let cursor = unsafe {
        (*image).xhot = scaled_raster_hotspot(source_xhot, source_width, width);
        (*image).yhot = scaled_raster_hotspot(source_yhot, source_height, height);
        let pixels = std::slice::from_raw_parts_mut((*image).pixels, pixel_count);
        scale_bilinear(
            source_pixels,
            source_width,
            source_height,
            pixels,
            width,
            height,
        );
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

fn restore_root_cursor(display: *mut xlib::Display, root: xlib::Window, base: &BaseCursor) {
    let Some(cursor) = make_cursor_at_scale(display, root, base, 1.0) else {
        return;
    };
    unsafe { xlib::XDefineCursor(display, root, cursor) };
    unsafe { xlib::XFreeCursor(display, cursor) };
}

fn mask_root_for_refresh(
    display: *mut xlib::Display,
    root: xlib::Window,
    base: &BaseCursor,
    mask: Option<&CursorImage>,
) {
    let Some(mask) = mask else {
        define_transparent_root_cursor(display, root);
        return;
    };
    let Some(cursor) = make_cursor_from_image(display, mask) else {
        restore_root_cursor(display, root, base);
        return;
    };
    unsafe { xlib::XDefineCursor(display, root, cursor) };
    unsafe { xlib::XFreeCursor(display, cursor) };
}

fn define_transparent_root_cursor(display: *mut xlib::Display, root: xlib::Window) {
    let transparent = CursorImage {
        width: 1,
        height: 1,
        xhot: 0,
        yhot: 0,
        pixels: vec![0],
        default_size: 1,
        name: None,
        source: None,
    };
    let Some(cursor) = make_cursor_from_image(display, &transparent) else {
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

fn scale_cursor_image_for_display(
    display: *mut xlib::Display,
    root: xlib::Window,
    image: &CursorImage,
    scale: f32,
) -> Option<CursorImage> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let factor = scale;
    if factor <= 0.0 {
        return None;
    }
    let requested_width = scaled_dimension(image.width, factor)?;
    let requested_height = scaled_dimension(image.height, factor)?;
    let (max_width, max_height) =
        best_cursor_size(display, root, requested_width, requested_height);
    let width = requested_width.min(max_width.max(1));
    let height = requested_height.min(max_height.max(1));
    let pixel_count = checked_pixel_count(width, height)?;
    let (source_width, source_height, source_xhot, source_yhot, source_pixels) = image
        .source
        .as_ref()
        .map(|source| {
            (
                source.width,
                source.height,
                source.xhot,
                source.yhot,
                source.pixels.as_slice(),
            )
        })
        .unwrap_or((
            image.width,
            image.height,
            image.xhot,
            image.yhot,
            image.pixels.as_slice(),
        ));
    let mut pixels = vec![0; pixel_count];
    scale_bilinear(
        source_pixels,
        source_width,
        source_height,
        &mut pixels,
        width,
        height,
    );
    Some(CursorImage {
        width,
        height,
        xhot: scaled_raster_hotspot(source_xhot, source_width, width),
        yhot: scaled_raster_hotspot(source_yhot, source_height, height),
        pixels,
        default_size: image.default_size,
        name: image.name.clone(),
        source: None,
    })
}

fn load_live_cursor_snapshot(
    display: *mut xlib::Display,
    default_size: u32,
) -> Option<(CursorImage, u64)> {
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
    let serial = image_ref.cursor_serial as u64;
    let cursor = CursorImage {
        width,
        height,
        xhot: sanitize_hotspot(u32::from(image_ref.xhot), width),
        yhot: sanitize_hotspot(u32::from(image_ref.yhot), height),
        pixels,
        default_size,
        name: copy_cursor_name(display, image_ref.atom, image_ref.name),
        source: None,
    };
    unsafe { xlib::XFree(image as *mut _) };
    Some((cursor, serial))
}

fn load_live_cursor_image(display: *mut xlib::Display, default_size: u32) -> Option<CursorImage> {
    load_live_cursor_snapshot(display, default_size).map(|(image, _)| image)
}

fn live_cursor_serial(display: *mut xlib::Display) -> Option<u64> {
    load_live_cursor_snapshot(display, 1).map(|(_, serial)| serial)
}

fn sync(display: *mut xlib::Display) {
    unsafe { xlib::XSync(display, xlib::False) };
}

fn live_refresh_enabled() -> bool {
    std::env::var_os("QOL_OS_THEMES_DISABLE_LIVE_REFRESH").is_none()
}

fn stable_live_cursor_sample(
    display: *mut xlib::Display,
    default_size: u32,
    current_grow_cursor: Option<&CursorImage>,
    current_applied_cursor: Option<&CursorImage>,
) -> Option<CursorImage> {
    let mut previous = None;
    let mut samples = Vec::new();
    let mut best = None;
    let mut best_count = 0usize;
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
        if is_our_enlarged_cursor(current_grow_cursor, current_applied_cursor, &current) {
            eprintln!(
                "[shake-to-grow] live refresh attempt={} ignored=self-sample",
                attempts + 1,
            );
            attempts += 1;
            continue;
        }
        log_cursor_image_with_attempt("live refresh attempt", usize::from(attempts + 1), &current);
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
        let count = sample_recurrence_count(&samples, &current);
        if count > best_count {
            best = Some(current.clone());
            best_count = count;
        }
        samples.push(current.clone());
        previous = Some(current);
        attempts += 1;
    }
    if let Some(best) = best {
        if best_count >= 2 {
            log_cursor_image_with_attempt("live refresh stabilized", best_count, &best);
            return Some(best);
        }
        if samples.len() == 1 {
            log_cursor_image_with_attempt("live refresh stabilized", 1, &best);
            return Some(best);
        }
    }
    eprintln!("[shake-to-grow] live refresh failed to stabilize");
    None
}

fn wait_for_cursor_recompute(display: *mut xlib::Display) {
    sync(display);
    std::thread::sleep(LIVE_REFRESH_DELAY);
    sync(display);
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

fn subscribe_cursor_notifications(display: *mut xlib::Display, root: xlib::Window) -> Option<i32> {
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

fn is_our_enlarged_cursor(
    grow_cursor: Option<&CursorImage>,
    applied_cursor: Option<&CursorImage>,
    sample: &CursorImage,
) -> bool {
    // We only ignore samples that are definitely SCALEED enlarged overrides.
    // Unscaled mask/baseline cursors (even if they are ours) must NOT be ignored
    // during sampling, otherwise we can't stabilize.
    if applied_cursor_is_scaled_variant(grow_cursor, applied_cursor)
        && applied_cursor.is_some_and(|applied| same_cursor_image(applied, sample))
    {
        return true;
    }
    let baseline_width = grow_cursor
        .map(|grow_cursor| grow_cursor.width)
        .unwrap_or(sample.default_size);
    let baseline_height = grow_cursor
        .map(|grow_cursor| grow_cursor.height)
        .unwrap_or(sample.default_size);

    // If it's significantly larger than baseline, it's likely our enlarged override.
    // We use a more permissive 1/4 factor to catch early scaling steps.
    sample.width >= baseline_width.saturating_mul(5) / 4
        || sample.height >= baseline_height.saturating_mul(5) / 4
}

fn is_our_cursor_serial(
    grow_cursor: Option<&CursorImage>,
    applied_cursor: Option<&CursorImage>,
    sample: &CursorImage,
) -> bool {
    // For serial tracking (ignoring our own updates), we are MORE inclusive.
    // We ignore serials for both our enlarged overrides AND our unscaled mask cursors.
    if grow_cursor.is_some_and(|grow| same_cursor_image(grow, sample)) {
        return true;
    }
    is_our_enlarged_cursor(grow_cursor, applied_cursor, sample)
}

fn applied_cursor_is_scaled_variant(
    grow_cursor: Option<&CursorImage>,
    applied_cursor: Option<&CursorImage>,
) -> bool {
    let Some(grow_cursor) = grow_cursor else {
        return false;
    };
    let Some(applied_cursor) = applied_cursor else {
        return false;
    };
    if applied_cursor.width != grow_cursor.width {
        return true;
    }
    applied_cursor.height != grow_cursor.height
}

fn refresh_sample_persisted(
    grow_cursor: Option<&CursorImage>,
    applied_cursor: Option<&CursorImage>,
    immediate_sample: Option<&CursorImage>,
    sample: &CursorImage,
) -> bool {
    let Some(immediate_sample) = immediate_sample else {
        return true;
    };
    if is_our_enlarged_cursor(grow_cursor, applied_cursor, immediate_sample) {
        if grow_cursor.is_some_and(|grow| same_cursor_image(grow, sample)) {
            return true;
        }
        if is_distinct_live_candidate(grow_cursor, applied_cursor, sample) {
            return true;
        }
        eprintln!("[shake-to-grow] live refresh rejected immediate self-sample");
        return false;
    }
    if grow_cursor.is_some_and(|grow| same_cursor_image(grow, immediate_sample)) {
        return true;
    }
    if same_cursor_image(immediate_sample, sample) {
        return true;
    }
    if is_distinct_live_candidate(grow_cursor, applied_cursor, immediate_sample)
        && is_distinct_live_candidate(grow_cursor, applied_cursor, sample)
    {
        eprintln!(
            "[shake-to-grow] live refresh accepted settled transition immediate={:016x} stable={:016x}",
            cursor_hash(immediate_sample),
            cursor_hash(sample),
        );
        return true;
    }
    eprintln!(
        "[shake-to-grow] live refresh rejected unstable candidate immediate={:016x} stable={:016x}",
        cursor_hash(immediate_sample),
        cursor_hash(sample),
    );
    false
}

fn armed_sample_indicates_change(
    grow_cursor: Option<&CursorImage>,
    applied_cursor: Option<&CursorImage>,
    armed_sample: Option<&CursorImage>,
) -> bool {
    let Some(armed_sample) = armed_sample else {
        return false;
    };
    is_distinct_live_candidate(grow_cursor, applied_cursor, armed_sample)
}

fn choose_refresh_mode(
    child_changed: bool,
    cursor_notify_pending: bool,
    armed_change: bool,
) -> RefreshMode {
    if armed_change || child_changed || cursor_notify_pending {
        return RefreshMode::Wait;
    }
    RefreshMode::None
}

fn refresh_mode_label(mode: RefreshMode) -> &'static str {
    match mode {
        RefreshMode::None => "none",
        RefreshMode::Wait => "wait",
    }
}

fn is_distinct_live_candidate(
    grow_cursor: Option<&CursorImage>,
    applied_cursor: Option<&CursorImage>,
    sample: &CursorImage,
) -> bool {
    if is_empty_cursor(sample) {
        return false;
    }
    if is_our_enlarged_cursor(grow_cursor, applied_cursor, sample) {
        return false;
    }
    if grow_cursor.is_some_and(|grow| same_cursor_image(grow, sample)) {
        return false;
    }
    true
}

fn sample_recurrence_count(samples: &[CursorImage], current: &CursorImage) -> usize {
    let mut count = 1usize;
    for sample in samples {
        if same_cursor_image(sample, current) {
            count += 1;
        }
    }
    count
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

fn log_live_refresh_sample_state(
    prefix: &str,
    sample: Option<&CursorImage>,
    grow_cursor: Option<&CursorImage>,
    applied_cursor: Option<&CursorImage>,
) {
    let Some(sample) = sample else {
        eprintln!("[shake-to-grow] {prefix}: none");
        return;
    };
    eprintln!(
        "[shake-to-grow] {prefix}: size={}x{} hot=({}, {}) hash={:016x} matches_grow={} matches_applied={}",
        sample.width,
        sample.height,
        sample.xhot,
        sample.yhot,
        cursor_hash(sample),
        grow_cursor.is_some_and(|grow| same_cursor_image(grow, sample)),
        applied_cursor.is_some_and(|applied| same_cursor_image(applied, sample)),
    );
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

fn preferred_source_size(default_size: u32, scale_factor: u32) -> u32 {
    default_size
        .saturating_mul(scale_factor.max(1))
        .max(default_size)
}

fn with_best_source(
    display: *mut xlib::Display,
    base: &BaseCursor,
    mut image: CursorImage,
    preferred_source_size: u32,
) -> CursorImage {
    if preferred_source_size <= image.default_size {
        return image;
    }
    let source = named_cursor_source(display, &image, preferred_source_size)
        .or_else(|| fallback_base_source(base, &image));
    let Some(source) = source else {
        return image;
    };
    if !source_improves_cursor(image.width, image.height, &source) {
        return image;
    }
    image.source = Some(source);
    image
}

fn named_cursor_source(
    display: *mut xlib::Display,
    image: &CursorImage,
    preferred_source_size: u32,
) -> Option<CursorRaster> {
    let name = image.name.as_deref()?;
    let name = CString::new(name).ok()?;
    load_named_cursor_raster(display, name.as_c_str(), preferred_source_size)
}

fn fallback_base_source(base: &BaseCursor, image: &CursorImage) -> Option<CursorRaster> {
    if !matches_base_cursor(base, image) {
        return None;
    }
    base.source.clone()
}

fn matches_base_cursor(base: &BaseCursor, image: &CursorImage) -> bool {
    if base.width != image.width {
        return false;
    }
    if base.height != image.height {
        return false;
    }
    if base.xhot != image.xhot {
        return false;
    }
    if base.yhot != image.yhot {
        return false;
    }
    base.pixels == image.pixels
}

fn source_improves_cursor(width: u32, height: u32, source: &CursorRaster) -> bool {
    source.width > width || source.height > height
}

fn load_named_cursor_raster(
    display: *mut xlib::Display,
    name: &CStr,
    request_size: u32,
) -> Option<CursorRaster> {
    let theme = unsafe { xcursor::XcursorGetTheme(display) };
    let images =
        unsafe { xcursor::XcursorLibraryLoadImages(name.as_ptr(), theme, request_size as i32) };
    if images.is_null() {
        return None;
    }
    let raster = cursor_raster_from_images(images);
    unsafe { xcursor::XcursorImagesDestroy(images) };
    raster
}

fn cursor_raster_from_images(images: *mut xcursor::XcursorImages) -> Option<CursorRaster> {
    let images = unsafe { &*images };
    let image_count = usize::try_from(images.nimage).ok()?;
    if image_count == 0 {
        return None;
    }
    let image_pointers = unsafe { std::slice::from_raw_parts(images.images, image_count) };
    let image = image_pointers
        .iter()
        .copied()
        .find(|image| !image.is_null())?;
    cursor_raster_from_xcursor_image(unsafe { &*image })
}

fn cursor_raster_from_xcursor_image(image: &xcursor::XcursorImage) -> Option<CursorRaster> {
    let pixel_count = checked_pixel_count(image.width, image.height)?;
    let pixels = unsafe { std::slice::from_raw_parts(image.pixels, pixel_count).to_vec() };
    Some(CursorRaster {
        width: image.width,
        height: image.height,
        xhot: sanitize_hotspot(image.xhot, image.width),
        yhot: sanitize_hotspot(image.yhot, image.height),
        pixels,
    })
}

fn copy_cursor_name(
    display: *mut xlib::Display,
    atom: xlib::Atom,
    name: *const libc::c_char,
) -> Option<String> {
    if !name.is_null() {
        return Some(
            unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned(),
        );
    }
    if atom == 0 {
        return None;
    }
    let atom_name = unsafe { xlib::XGetAtomName(display, atom) };
    if atom_name.is_null() {
        return None;
    }
    let owned = unsafe { CStr::from_ptr(atom_name) }
        .to_string_lossy()
        .into_owned();
    unsafe { xlib::XFree(atom_name as *mut _) };
    Some(owned)
}

fn scaled_dimension(base: u32, factor: f32) -> Option<u32> {
    let scaled = (base as f32 * factor).round();
    if !scaled.is_finite() || scaled < 1.0 || scaled > i32::MAX as f32 {
        return None;
    }
    Some((scaled as u32).clamp(1, MAX_CURSOR_DIMENSION))
}

fn scaled_raster_hotspot(hotspot: u32, source_bound: u32, target_bound: u32) -> u32 {
    if source_bound == 0 {
        return 0;
    }
    let scaled = hotspot as f32 * target_bound as f32 / source_bound as f32;
    if !scaled.is_finite() || scaled < 0.0 {
        return 0;
    }
    (scaled.round() as u32).min(target_bound.saturating_sub(1))
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
