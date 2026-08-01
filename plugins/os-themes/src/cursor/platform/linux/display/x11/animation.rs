use super::*;

pub(super) fn make_cursor_at_scale(
    display: *mut xlib::Display,
    root: xlib::Window,
    base: &BaseCursor,
    scale: f32,
) -> Option<xlib::Cursor> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let factor = scale;
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

pub(super) fn best_cursor_size(
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

pub(super) fn make_cursor_from_frames(
    display: *mut xlib::Display,
    frames: &[CursorRaster],
) -> Option<xlib::Cursor> {
    if frames.is_empty() {
        return None;
    }
    let images = unsafe { xcursor::XcursorImagesCreate(frames.len().try_into().ok()?) };
    if images.is_null() {
        return None;
    }
    for frame in frames {
        let Some(pixel_count) = checked_pixel_count(frame.width, frame.height) else {
            break;
        };
        let Ok(width) = frame.width.try_into() else {
            break;
        };
        let Ok(height) = frame.height.try_into() else {
            break;
        };
        let image = unsafe { xcursor::XcursorImageCreate(width, height) };
        if image.is_null() {
            break;
        }
        unsafe {
            (*image).xhot = frame.xhot;
            (*image).yhot = frame.yhot;
            (*image).delay = frame.delay_ms;
            let pixels = std::slice::from_raw_parts_mut((*image).pixels, pixel_count);
            pixels.copy_from_slice(&frame.pixels);
            let slot = (*images).images.add((*images).nimage as usize);
            *slot = image;
            (*images).nimage += 1;
        }
    }
    let complete = unsafe { (*images).nimage as usize } == frames.len();
    let cursor = if complete {
        unsafe { xcursor::XcursorImagesLoadCursor(display, images) }
    } else {
        0
    };
    unsafe { xcursor::XcursorImagesDestroy(images) };
    if cursor == 0 {
        return None;
    }
    Some(cursor)
}

pub(super) fn scale_cursor_for_display(
    display: *mut xlib::Display,
    root: xlib::Window,
    image: &CursorImage,
    scale: f32,
) -> Option<ScaledCursor> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let requested_width = scaled_dimension(image.width, scale)?;
    let requested_height = scaled_dimension(image.height, scale)?;
    let (max_width, max_height) =
        best_cursor_size(display, root, requested_width, requested_height);
    let width = requested_width.min(max_width.max(1));
    let height = requested_height.min(max_height.max(1));
    let pixel_count = checked_pixel_count(width, height)?;
    let raw_source = CursorRaster {
        width: image.width,
        height: image.height,
        xhot: image.xhot,
        yhot: image.yhot,
        delay_ms: 0,
        pixels: image.pixels.clone(),
    };
    let sources: &[CursorRaster] = if image.source.is_empty() {
        std::slice::from_ref(&raw_source)
    } else {
        &image.source
    };
    let mut frames = Vec::with_capacity(sources.len());
    for source in sources {
        let mut pixels = vec![0; pixel_count];
        scale_bilinear(
            &source.pixels,
            source.width,
            source.height,
            &mut pixels,
            width,
            height,
        );
        frames.push(CursorRaster {
            width,
            height,
            xhot: scaled_raster_hotspot(source.xhot, source.width, width),
            yhot: scaled_raster_hotspot(source.yhot, source.height, height),
            delay_ms: source.delay_ms,
            pixels,
        });
    }
    let first = frames.first()?;
    let applied = CursorImage {
        width,
        height,
        xhot: first.xhot,
        yhot: first.yhot,
        pixels: first.pixels.clone(),
        default_size: image.default_size,
        name: image.name.clone(),
        source: Vec::new(),
    };
    Some(ScaledCursor { frames, applied })
}

pub(super) fn scaled_dimension(base: u32, factor: f32) -> Option<u32> {
    let scaled = (base as f32 * factor).round();
    if !scaled.is_finite() || scaled < 1.0 || scaled > i32::MAX as f32 {
        return None;
    }
    Some((scaled as u32).clamp(1, MAX_CURSOR_DIMENSION))
}

pub(super) fn scaled_raster_hotspot(hotspot: u32, source_bound: u32, target_bound: u32) -> u32 {
    if source_bound == 0 {
        return 0;
    }
    let scaled = hotspot as f32 * target_bound as f32 / source_bound as f32;
    if !scaled.is_finite() || scaled < 0.0 {
        return 0;
    }
    (scaled.round() as u32).min(target_bound.saturating_sub(1))
}

pub(super) fn sanitize_hotspot(hotspot: u32, bound: u32) -> u32 {
    hotspot.min(bound.saturating_sub(1))
}

pub(super) fn sanitize_dimension(value: u32) -> u32 {
    if value == 0 {
        return 1;
    }
    value.min(MAX_CURSOR_DIMENSION)
}
