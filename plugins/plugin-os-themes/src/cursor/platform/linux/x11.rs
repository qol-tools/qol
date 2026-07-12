use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr;

use anyhow::{ensure, Result};
use x11::{xcursor, xfixes, xlib};

use super::scale::scale_bilinear;

const MAX_CURSOR_DIMENSION: u32 = 512;
const XFIXES_CURSOR_NOTIFY: i32 = 1;
const XFIXES_DISPLAY_CURSOR_NOTIFY: i32 = 0;
const XFIXES_DISPLAY_CURSOR_NOTIFY_MASK: libc::c_ulong = 1;
const SHAPE_CACHE_CAP: usize = 64;

const CATALOG_SHAPE_NAMES: [&CStr; 45] = [
    c"text",
    c"xterm",
    c"pointer",
    c"hand2",
    c"hand1",
    c"grabbing",
    c"openhand",
    c"closedhand",
    c"fleur",
    c"move",
    c"all-scroll",
    c"crosshair",
    c"cross",
    c"watch",
    c"progress",
    c"left_ptr_watch",
    c"help",
    c"question_arrow",
    c"not-allowed",
    c"crossed_circle",
    c"col-resize",
    c"row-resize",
    c"ew-resize",
    c"ns-resize",
    c"nesw-resize",
    c"nwse-resize",
    c"sb_h_double_arrow",
    c"sb_v_double_arrow",
    c"size_hor",
    c"size_ver",
    c"size_fdiag",
    c"size_bdiag",
    c"top_side",
    c"bottom_side",
    c"left_side",
    c"right_side",
    c"top_left_corner",
    c"top_right_corner",
    c"bottom_left_corner",
    c"bottom_right_corner",
    c"cell",
    c"vertical-text",
    c"zoom-in",
    c"zoom-out",
    c"pencil",
];

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
    xfixes_event_base: Option<i32>,
    catalog: ShapeCatalog,
}

struct ShapeCatalog {
    cache: HashMap<u64, Option<CursorRaster>>,
}

impl ShapeCatalog {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    fn source_for(
        &mut self,
        display: *mut xlib::Display,
        image: &CursorImage,
        preferred_source_size: u32,
    ) -> Option<CursorRaster> {
        let key = cursor_hash(image);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let source = identify_shape_source(display, image, preferred_source_size);
        if self.cache.len() >= SHAPE_CACHE_CAP {
            self.cache.clear();
        }
        self.cache.insert(key, source.clone());
        source
    }
}

fn identify_shape_source(
    display: *mut xlib::Display,
    image: &CursorImage,
    preferred_source_size: u32,
) -> Option<CursorRaster> {
    let request_size = image.width.max(image.height);
    for name in CATALOG_SHAPE_NAMES {
        let Some(candidate) = load_named_cursor_raster(display, name, request_size) else {
            continue;
        };
        if raster_matches_image(&candidate, image) {
            return load_named_cursor_raster(display, name, preferred_source_size);
        }
    }
    None
}

fn raster_matches_image(raster: &CursorRaster, image: &CursorImage) -> bool {
    raster.width == image.width
        && raster.height == image.height
        && raster.xhot == image.xhot
        && raster.yhot == image.yhot
        && raster.pixels == image.pixels
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
            xfixes_event_base: subscribe_cursor_notifications(display, root),
            catalog: ShapeCatalog::new(),
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
        if !self.take_cursor_notification() {
            return false;
        }
        if !live_refresh_enabled() {
            return false;
        }
        let Some(sample) = load_live_cursor_image(self.display, self.base.default_size) else {
            return false;
        };
        if is_empty_cursor(&sample) {
            return false;
        }
        if self
            .applied_cursor
            .as_ref()
            .is_some_and(|applied| same_cursor_image(applied, &sample))
        {
            return false;
        }
        if is_our_enlarged_cursor(
            self.grow_cursor.as_ref(),
            self.applied_cursor.as_ref(),
            &sample,
        ) {
            return false;
        }
        if self
            .grow_cursor
            .as_ref()
            .is_some_and(|grow| same_cursor_image(grow, &sample))
        {
            return self.reapply_active_cursor();
        }
        let sample = with_best_source(
            self.display,
            &self.base,
            &mut self.catalog,
            sample,
            self.preferred_source_size,
        );
        log_cursor_image("live refresh adopt", &sample);
        self.grow_cursor = Some(sample);
        self.apply_grow_cursor()
    }

    pub fn restore(&mut self) {
        if self.active_cursor.is_none() {
            return;
        }
        clear_tree(self.display, self.root);
        restore_root_cursor(self.display, self.root, &self.base);
        self.flush();
        if let Some(cursor) = self.active_cursor.take() {
            unsafe { xlib::XFreeCursor(self.display, cursor) };
        }
        self.current_scale = 1.0;
        self.applied_cursor = None;
        self.grow_cursor = None;
    }

    fn flush(&self) {
        sync(self.display);
    }

    fn capture_live_cursors(&mut self) {
        let live_cursor =
            load_live_cursor_image(self.display, self.base.default_size).map(|cursor| {
                with_best_source(
                    self.display,
                    &self.base,
                    &mut self.catalog,
                    cursor,
                    self.preferred_source_size,
                )
            });
        let Some(live_cursor) = live_cursor else {
            eprintln!("[shake-to-grow] failed to capture live cursor at grow-start");
            return;
        };
        log_cursor_image("captured live cursor", &live_cursor);
        self.grow_cursor = Some(live_cursor);
    }

    fn apply_grow_cursor(&mut self) -> bool {
        let expected_applied_cursor = self.grow_cursor.as_ref().and_then(|image| {
            scale_cursor_image_for_display(self.display, self.root, image, self.current_scale)
        });
        let next_cursor = expected_applied_cursor
            .as_ref()
            .and_then(|image| make_cursor_from_image(self.display, image));
        let Some(next_cursor) = next_cursor else {
            eprintln!("[shake-to-grow] live refresh failed to build scaled cursor");
            return false;
        };
        apply_to_tree(self.display, self.root, next_cursor);
        self.flush();
        self.applied_cursor = expected_applied_cursor;
        if let Some(old_cursor) = self.active_cursor.replace(next_cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
        true
    }

    fn reapply_active_cursor(&mut self) -> bool {
        let Some(cursor) = self.active_cursor else {
            return false;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
        true
    }

    fn take_cursor_notification(&mut self) -> bool {
        let Some(event_base) = self.xfixes_event_base else {
            return false;
        };
        let mut pending = false;
        while unsafe { xlib::XPending(self.display) } > 0 {
            let mut event = std::mem::MaybeUninit::<xlib::XEvent>::uninit();
            unsafe { xlib::XNextEvent(self.display, event.as_mut_ptr()) };
            let event = unsafe { event.assume_init() };
            if event.get_type() != event_base + XFIXES_CURSOR_NOTIFY {
                continue;
            }
            let notify = unsafe {
                *(&event as *const xlib::XEvent as *const xfixes::XFixesCursorNotifyEvent)
            };
            if notify.subtype != XFIXES_DISPLAY_CURSOR_NOTIFY {
                continue;
            }
            pending = true;
        }
        pending
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

fn restore_root_cursor(display: *mut xlib::Display, root: xlib::Window, base: &BaseCursor) {
    let Some(cursor) = make_cursor_at_scale(display, root, base, 1.0) else {
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
        name: copy_cursor_name(display, image_ref.atom, image_ref.name),
        source: None,
    };
    unsafe { xlib::XFree(image as *mut _) };
    Some(cursor)
}

fn sync(display: *mut xlib::Display) {
    unsafe { xlib::XSync(display, xlib::False) };
}

fn live_refresh_enabled() -> bool {
    std::env::var_os("QOL_OS_THEMES_DISABLE_LIVE_REFRESH").is_none()
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

fn is_empty_cursor(image: &CursorImage) -> bool {
    image.pixels.iter().all(|pixel| ((pixel >> 24) & 0xFF) == 0)
}

fn log_cursor_image(prefix: &str, image: &CursorImage) {
    let source = image
        .source
        .as_ref()
        .map(|source| format!("{}x{}", source.width, source.height))
        .unwrap_or_else(|| "none".to_string());
    eprintln!(
        "[shake-to-grow] {prefix}: size={}x{} hot=({}, {}) name={:?} source={source} hash={:016x}",
        image.width,
        image.height,
        image.xhot,
        image.yhot,
        image.name.as_deref().unwrap_or("-"),
        cursor_hash(image),
    );
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
    catalog: &mut ShapeCatalog,
    mut image: CursorImage,
    preferred_source_size: u32,
) -> CursorImage {
    if preferred_source_size <= image.default_size {
        return image;
    }
    let source = named_cursor_source(display, &image, preferred_source_size)
        .or_else(|| fallback_base_source(base, &image))
        .or_else(|| catalog.source_for(display, &image, preferred_source_size));
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
        let owned = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        if !owned.is_empty() {
            return Some(owned);
        }
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
