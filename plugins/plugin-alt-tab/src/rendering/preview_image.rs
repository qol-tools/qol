use gpui::RenderImage;
use std::sync::Arc;

/// Sample ~1KB of evenly-spaced pixels for a fast content-change check.
pub(crate) fn fast_pixel_hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let stride = (data.len() / 256).max(1);
    let mut i = 0;
    while i < data.len() {
        let end = (i + 4).min(data.len());
        data[i..end].hash(&mut hasher);
        i += stride;
    }
    hasher.finish()
}

pub(crate) fn bgra_to_render_image(data: Vec<u8>, w: usize, h: usize) -> Option<Arc<RenderImage>> {
    let buf = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w as u32, h as u32, data)?;
    let frame = image::Frame::new(buf);
    Some(Arc::new(RenderImage::new(smallvec::smallvec![frame])))
}

pub(crate) fn shot_request_dims(window_w: f32, window_h: f32) -> (usize, usize) {
    use crate::picker::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
    if window_w <= 0.0 || window_h <= 0.0 {
        return (PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
    }
    let aspect = window_w / window_h;
    let box_aspect = PREVIEW_MAX_WIDTH as f32 / PREVIEW_MAX_HEIGHT as f32;
    if aspect >= box_aspect {
        let width = (PREVIEW_MAX_HEIGHT as f32 * aspect).round() as usize;
        (width.max(1), PREVIEW_MAX_HEIGHT)
    } else {
        let height = (PREVIEW_MAX_WIDTH as f32 / aspect).round() as usize;
        (PREVIEW_MAX_WIDTH, height.max(1))
    }
}
