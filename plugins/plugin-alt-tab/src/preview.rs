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

pub(crate) fn bgra_to_render_image(data: &[u8], w: usize, h: usize) -> Option<Arc<RenderImage>> {
    let buf =
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w as u32, h as u32, data.to_vec())?;
    let frame = image::Frame::new(buf);
    Some(Arc::new(RenderImage::new(smallvec::smallvec![frame])))
}
