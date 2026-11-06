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

pub(crate) fn shot_request_dims(window_w: f32, window_h: f32) -> (usize, usize) {
    use crate::picker::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
    fit_within(window_w, window_h, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
}

pub(crate) fn fit_within(src_w: f32, src_h: f32, max_w: usize, max_h: usize) -> (usize, usize) {
    if src_w <= 0.0 || src_h <= 0.0 {
        return (max_w, max_h);
    }
    let aspect = src_w / src_h;
    if aspect >= max_w as f32 / max_h as f32 {
        let height = (max_w as f32 / aspect).round() as usize;
        (max_w, height.clamp(1, max_h))
    } else {
        let width = (max_h as f32 * aspect).round() as usize;
        (width.clamp(1, max_w), max_h)
    }
}

#[cfg(test)]
mod tests {
    use super::fit_within;

    #[test]
    fn a_window_fits_inside_the_card_without_losing_any_edge() {
        let cases = [
            ("taller than the card", (1200.0, 1000.0), (415, 346)),
            ("wider than the card", (2400.0, 1000.0), (551, 230)),
            ("same shape as the card", (1102.0, 692.0), (551, 346)),
            ("no size yet", (0.0, 0.0), (551, 346)),
        ];
        for (label, (w, h), expected) in cases {
            assert_eq!(fit_within(w, h, 551, 346), expected, "{label}");
        }
    }
}
