use super::*;

pub(super) const CATALOG_SHAPE_NAMES: [&CStr; 46] = [
    c"left_ptr",
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

pub(super) struct ShapeCatalog {
    frame_tables: HashMap<u32, HashMap<u64, usize>>,
    sources: HashMap<usize, Vec<CursorRaster>>,
}

impl ShapeCatalog {
    pub(super) fn new() -> Self {
        Self {
            frame_tables: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    pub(super) fn source_for(
        &mut self,
        display: *mut xlib::Display,
        image: &CursorImage,
        preferred_source_size: u32,
    ) -> Vec<CursorRaster> {
        let request_size = image.width.max(image.height);
        if self.frame_tables.len() >= FRAME_TABLE_CAP
            && !self.frame_tables.contains_key(&request_size)
        {
            return Vec::new();
        }
        let table = self
            .frame_tables
            .entry(request_size)
            .or_insert_with(|| build_frame_table(display, request_size));
        let Some(name_index) = table.get(&cursor_hash(image)).copied() else {
            return Vec::new();
        };
        self.sources
            .entry(name_index)
            .or_insert_with(|| {
                load_named_cursor_frames(
                    display,
                    CATALOG_SHAPE_NAMES[name_index],
                    preferred_source_size,
                )
            })
            .clone()
    }
}

pub(super) fn build_frame_table(
    display: *mut xlib::Display,
    request_size: u32,
) -> HashMap<u64, usize> {
    let mut table = HashMap::new();
    let theme = unsafe { xcursor::XcursorGetTheme(display) };
    for (name_index, name) in CATALOG_SHAPE_NAMES.iter().enumerate() {
        let images =
            unsafe { xcursor::XcursorLibraryLoadImages(name.as_ptr(), theme, request_size as i32) };
        if images.is_null() {
            continue;
        }
        for raster in cursor_rasters_from_images(images) {
            table.entry(raster_hash(&raster)).or_insert(name_index);
        }
        unsafe { xcursor::XcursorImagesDestroy(images) };
    }
    table
}

pub(super) fn cursor_rasters_from_images(images: *mut xcursor::XcursorImages) -> Vec<CursorRaster> {
    let images_ref = unsafe { &*images };
    let Ok(image_count) = usize::try_from(images_ref.nimage) else {
        return Vec::new();
    };
    let image_pointers = unsafe { std::slice::from_raw_parts(images_ref.images, image_count) };
    image_pointers
        .iter()
        .copied()
        .filter(|pointer| !pointer.is_null())
        .filter_map(|pointer| cursor_raster_from_xcursor_image(unsafe { &*pointer }))
        .collect()
}

pub(super) fn load_base_cursor(
    display: *mut xlib::Display,
    scale_factor: u32,
) -> Option<BaseCursor> {
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

pub(super) fn preferred_source_size(default_size: u32, scale_factor: u32) -> u32 {
    default_size
        .saturating_mul(scale_factor.max(1))
        .max(default_size)
}

pub(super) fn with_best_source(
    display: *mut xlib::Display,
    base: &BaseCursor,
    catalog: &mut ShapeCatalog,
    mut image: CursorImage,
    preferred_source_size: u32,
) -> CursorImage {
    if preferred_source_size <= image.default_size {
        return image;
    }
    let mut source = named_cursor_frames(display, &image, preferred_source_size);
    if source.is_empty() {
        source = fallback_base_source(base, &image);
    }
    if source.is_empty() {
        source = catalog.source_for(display, &image, preferred_source_size);
    }
    let improves = source
        .first()
        .is_some_and(|first| source_improves_cursor(image.width, image.height, first));
    if !improves {
        return image;
    }
    image.source = source;
    image
}

pub(super) fn named_cursor_frames(
    display: *mut xlib::Display,
    image: &CursorImage,
    preferred_source_size: u32,
) -> Vec<CursorRaster> {
    let Some(name) = image.name.as_deref() else {
        return Vec::new();
    };
    let Ok(name) = CString::new(name) else {
        return Vec::new();
    };
    load_named_cursor_frames(display, name.as_c_str(), preferred_source_size)
}

pub(super) fn fallback_base_source(base: &BaseCursor, image: &CursorImage) -> Vec<CursorRaster> {
    if !matches_base_cursor(base, image) {
        return Vec::new();
    }
    base.source.clone().into_iter().collect()
}

pub(super) fn matches_base_cursor(base: &BaseCursor, image: &CursorImage) -> bool {
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

pub(super) fn source_improves_cursor(width: u32, height: u32, source: &CursorRaster) -> bool {
    source.width > width || source.height > height
}

pub(super) fn load_named_cursor_raster(
    display: *mut xlib::Display,
    name: &CStr,
    request_size: u32,
) -> Option<CursorRaster> {
    load_named_cursor_frames(display, name, request_size)
        .into_iter()
        .next()
}

pub(super) fn load_named_cursor_frames(
    display: *mut xlib::Display,
    name: &CStr,
    request_size: u32,
) -> Vec<CursorRaster> {
    let theme = unsafe { xcursor::XcursorGetTheme(display) };
    let images =
        unsafe { xcursor::XcursorLibraryLoadImages(name.as_ptr(), theme, request_size as i32) };
    if images.is_null() {
        return Vec::new();
    }
    let frames = cursor_rasters_from_images(images);
    unsafe { xcursor::XcursorImagesDestroy(images) };
    thin_frames(frames)
}

pub(super) fn thin_frames(frames: Vec<CursorRaster>) -> Vec<CursorRaster> {
    if frames.len() <= MAX_SOURCE_FRAMES {
        return frames;
    }
    let stride = frames.len().div_ceil(MAX_SOURCE_FRAMES);
    frames
        .into_iter()
        .enumerate()
        .filter(|(index, _)| index % stride == 0)
        .map(|(_, mut frame)| {
            frame.delay_ms = frame.delay_ms.saturating_mul(stride as u32);
            frame
        })
        .collect()
}
