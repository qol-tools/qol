use std::ptr;

use anyhow::{ensure, Result};
use x11::{xcursor, xlib};

pub struct CursorSession {
    display: *mut xlib::Display,
    root: xlib::Window,
    base: BaseCursor,
    active_cursor: Option<xlib::Cursor>,
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
    pub fn open(scale_factor: u32) -> Result<Option<Self>> {
        let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
        ensure!(!display.is_null(), "failed to open X11 display");

        let Some(base) = load_base_cursor(display, scale_factor) else {
            unsafe { xlib::XCloseDisplay(display) };
            return Ok(None);
        };

        Ok(Some(Self {
            display,
            root: unsafe { xlib::XDefaultRootWindow(display) },
            base,
            active_cursor: None,
        }))
    }

    pub fn pointer_position(&self) -> (i32, i32) {
        query_pointer(self.display, self.root)
    }

    pub fn set_scale(&mut self, scale: f32) {
        let Some(cursor) = make_cursor_at_scale(self.display, &self.base, scale) else {
            return;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
        if let Some(old_cursor) = self.active_cursor.replace(cursor) {
            unsafe { xlib::XFreeCursor(self.display, old_cursor) };
        }
    }

    pub fn reapply_active(&self) {
        let Some(cursor) = self.active_cursor else {
            return;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
    }

    pub fn restore(&mut self) {
        let had_active_cursor = self.active_cursor.is_some();
        self.free_active_cursor();
        if !had_active_cursor {
            return;
        }
        let Some(cursor) = make_cursor_at_scale(self.display, &self.base, 1.0) else {
            return;
        };
        apply_to_tree(self.display, self.root, cursor);
        self.flush();
        unsafe { xlib::XFreeCursor(self.display, cursor) };
    }

    fn flush(&self) {
        unsafe { xlib::XFlush(self.display) };
    }

    fn free_active_cursor(&mut self) {
        let Some(cursor) = self.active_cursor.take() else {
            return;
        };
        unsafe { xlib::XFreeCursor(self.display, cursor) };
    }
}

impl Drop for CursorSession {
    fn drop(&mut self) {
        self.free_active_cursor();
        unsafe { xlib::XCloseDisplay(self.display) };
    }
}

fn query_pointer(display: *mut xlib::Display, root: xlib::Window) -> (i32, i32) {
    let (mut root_out, mut child_out): (xlib::Window, xlib::Window) = (0, 0);
    let (mut root_x, mut root_y, mut window_x, mut window_y): (i32, i32, i32, i32) = (0, 0, 0, 0);
    let mut mask: u32 = 0;
    unsafe {
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
        );
    }
    (root_x, root_y)
}

fn load_base_cursor(display: *mut xlib::Display, scale_factor: u32) -> Option<BaseCursor> {
    let raw_size = unsafe { xcursor::XcursorGetDefaultSize(display) };
    let default_size = if raw_size > 0 { raw_size as u32 } else { 24 };
    let theme = unsafe { xcursor::XcursorGetTheme(display) };
    let images = unsafe {
        xcursor::XcursorLibraryLoadImages(
            c"left_ptr".as_ptr(),
            theme,
            (default_size * scale_factor) as i32,
        )
    };
    if images.is_null() {
        return None;
    }

    let image = unsafe { &**(*images).images };
    let pixels = unsafe {
        std::slice::from_raw_parts(image.pixels, (image.width * image.height) as usize).to_vec()
    };
    let base = BaseCursor {
        width: image.width,
        height: image.height,
        xhot: image.xhot,
        yhot: image.yhot,
        pixels,
        default_size,
    };

    unsafe { xcursor::XcursorImagesDestroy(images) };
    Some(base)
}

fn make_cursor_at_scale(
    display: *mut xlib::Display,
    base: &BaseCursor,
    scale: f32,
) -> Option<xlib::Cursor> {
    let target_size = base.default_size as f32 * scale;
    let factor = target_size / base.width as f32;
    let width = ((base.width as f32 * factor) as u32).max(1);
    let height = ((base.height as f32 * factor) as u32).max(1);
    let image = unsafe { xcursor::XcursorImageCreate(width as i32, height as i32) };
    if image.is_null() {
        return None;
    }

    let cursor = unsafe {
        (*image).xhot = (base.xhot as f32 * factor) as u32;
        (*image).yhot = (base.yhot as f32 * factor) as u32;
        let pixels = std::slice::from_raw_parts_mut((*image).pixels, (width * height) as usize);
        let source = PixelGrid::new(&base.pixels, ImageSize::new(base.width, base.height));
        let request = ScaleRequest::new(source, ImageSize::new(width, height));
        scale_bilinear(&request, pixels);
        let cursor = xcursor::XcursorImageLoadCursor(display, image);
        xcursor::XcursorImageDestroy(image);
        cursor
    };

    if cursor == 0 {
        return None;
    }
    Some(cursor)
}

fn scale_bilinear(request: &ScaleRequest<'_>, dst: &mut [u32]) {
    for dy in 0..request.dst.height {
        for dx in 0..request.dst.width {
            let point = PixelPoint { x: dx, y: dy };
            dst[(dy * request.dst.width + dx) as usize] = scaled_pixel(request, point);
        }
    }
}

fn scaled_pixel(request: &ScaleRequest<'_>, point: PixelPoint) -> u32 {
    let source = source_point(request, point);
    let corners = pixel_corners(request, &source);
    blend_pixel(corners, BlendFactors::from(&source))
}

fn source_point(request: &ScaleRequest<'_>, point: PixelPoint) -> SourcePoint {
    let sx = point.x as f32 * (request.src.width as f32 - 1.0) / scale_span(request.dst.width);
    let sy = point.y as f32 * (request.src.height as f32 - 1.0) / scale_span(request.dst.height);
    let x0 = sx as u32;
    let y0 = sy as u32;
    SourcePoint {
        x0,
        y0,
        x1: (x0 + 1).min(request.src.width - 1),
        y1: (y0 + 1).min(request.src.height - 1),
        tx: sx - x0 as f32,
        ty: sy - y0 as f32,
    }
}

fn scale_span(length: u32) -> f32 {
    (length as f32 - 1.0).max(1.0)
}

fn pixel_corners(request: &ScaleRequest<'_>, source: &SourcePoint) -> [u32; 4] {
    [
        request.src.pixel(source.x0, source.y0),
        request.src.pixel(source.x1, source.y0),
        request.src.pixel(source.x0, source.y1),
        request.src.pixel(source.x1, source.y1),
    ]
}

fn blend_pixel(corners: [u32; 4], blend: BlendFactors) -> u32 {
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        out |= blended_channel(corners, blend, shift) << shift;
    }
    out
}

fn blended_channel(corners: [u32; 4], blend: BlendFactors, shift: u32) -> u32 {
    let [p00, p10, p01, p11] = corners;
    let channel = |pixel: u32| ((pixel >> shift) & 0xFF) as f32;
    let value = channel(p00) * (1.0 - blend.tx) * (1.0 - blend.ty)
        + channel(p10) * blend.tx * (1.0 - blend.ty)
        + channel(p01) * (1.0 - blend.tx) * blend.ty
        + channel(p11) * blend.tx * blend.ty;
    value as u32 & 0xFF
}

struct ScaleRequest<'a> {
    src: PixelGrid<'a>,
    dst: ImageSize,
}

impl<'a> ScaleRequest<'a> {
    fn new(src: PixelGrid<'a>, dst: ImageSize) -> Self {
        Self { src, dst }
    }
}

#[derive(Clone, Copy)]
struct PixelPoint {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy)]
struct ImageSize {
    width: u32,
    height: u32,
}

impl ImageSize {
    fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

struct PixelGrid<'a> {
    pixels: &'a [u32],
    width: u32,
    height: u32,
}

impl<'a> PixelGrid<'a> {
    fn new(pixels: &'a [u32], size: ImageSize) -> Self {
        Self {
            pixels,
            width: size.width,
            height: size.height,
        }
    }

    fn pixel(&self, x: u32, y: u32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }
}

struct SourcePoint {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    tx: f32,
    ty: f32,
}

#[derive(Clone, Copy)]
struct BlendFactors {
    tx: f32,
    ty: f32,
}

impl From<&SourcePoint> for BlendFactors {
    fn from(source: &SourcePoint) -> Self {
        Self {
            tx: source.tx,
            ty: source.ty,
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

#[cfg(test)]
mod tests {
    use super::scale_bilinear;
    use super::{ImageSize, PixelGrid, ScaleRequest};

    #[test]
    fn scale_bilinear_2x_maps_source_corners() {
        let src = [0xFFFF0000u32, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF];
        let mut dst = [0u32; 16];
        let request = ScaleRequest::new(
            PixelGrid::new(&src, ImageSize::new(2, 2)),
            ImageSize::new(4, 4),
        );
        scale_bilinear(&request, &mut dst);
        assert_eq!(dst[0], 0xFFFF0000, "top-left corner");
        assert_eq!(dst[3], 0xFF00FF00, "top-right corner");
        assert_eq!(dst[12], 0xFF0000FF, "bottom-left corner");
        assert_eq!(dst[15], 0xFFFFFFFF, "bottom-right corner");
    }
}
