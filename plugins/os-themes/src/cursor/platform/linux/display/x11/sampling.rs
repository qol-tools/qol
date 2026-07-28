use super::*;

pub(super) fn load_live_cursor_image(
    display: *mut xlib::Display,
    default_size: u32,
) -> Option<CursorImage> {
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
        source: Vec::new(),
    };
    unsafe { xlib::XFree(image as *mut _) };
    Some(cursor)
}

pub(super) fn same_cursor_image(left: &CursorImage, right: &CursorImage) -> bool {
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

pub(super) fn is_our_enlarged_cursor(
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

pub(super) fn applied_cursor_is_scaled_variant(
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

pub(super) fn is_empty_cursor(image: &CursorImage) -> bool {
    image.pixels.iter().all(|pixel| ((pixel >> 24) & 0xFF) == 0)
}

pub(super) fn log_cursor_image(prefix: &str, image: &CursorImage) {
    let source = image
        .source
        .first()
        .map(|source| format!("{}x{}x{}", source.width, source.height, image.source.len()))
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

pub(super) fn cursor_hash(image: &CursorImage) -> u64 {
    pixel_signature(
        image.width,
        image.height,
        image.xhot,
        image.yhot,
        &image.pixels,
    )
}

pub(super) fn raster_hash(raster: &CursorRaster) -> u64 {
    pixel_signature(
        raster.width,
        raster.height,
        raster.xhot,
        raster.yhot,
        &raster.pixels,
    )
}

pub(super) fn pixel_signature(
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    pixels: &[u32],
) -> u64 {
    let mut hash = 1469598103934665603u64;
    hash = hash_cursor_value(hash, u64::from(width));
    hash = hash_cursor_value(hash, u64::from(height));
    hash = hash_cursor_value(hash, u64::from(xhot));
    hash = hash_cursor_value(hash, u64::from(yhot));
    for pixel in pixels {
        hash = hash_cursor_value(hash, u64::from(*pixel));
    }
    hash
}

pub(super) fn hash_cursor_value(hash: u64, value: u64) -> u64 {
    let hash = hash ^ value;
    hash.wrapping_mul(1099511628211)
}

pub(super) fn checked_pixel_count(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    width.checked_mul(height)
}

pub(super) fn cursor_raster_from_xcursor_image(
    image: &xcursor::XcursorImage,
) -> Option<CursorRaster> {
    let pixel_count = checked_pixel_count(image.width, image.height)?;
    let pixels = unsafe { std::slice::from_raw_parts(image.pixels, pixel_count).to_vec() };
    Some(CursorRaster {
        width: image.width,
        height: image.height,
        xhot: sanitize_hotspot(image.xhot, image.width),
        yhot: sanitize_hotspot(image.yhot, image.height),
        delay_ms: image.delay,
        pixels,
    })
}

pub(super) fn copy_cursor_name(
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
