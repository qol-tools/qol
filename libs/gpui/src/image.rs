use std::sync::Arc;

use ::image::{Frame, ImageBuffer, Rgba};
use gpui::RenderImage;

pub fn render_image(pixels: Vec<u8>, width: u32, height: u32) -> Option<Arc<RenderImage>> {
    let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels)?;
    let frame = Frame::new(buffer);
    Some(Arc::new(RenderImage::new(smallvec::smallvec![frame])))
}

pub fn render_image_rgba(mut pixels: Vec<u8>, width: u32, height: u32) -> Option<Arc<RenderImage>> {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    render_image(pixels, width, height)
}

#[cfg(test)]
mod tests {
    use super::{render_image, render_image_rgba};

    #[test]
    fn render_image_rejects_length_mismatch() {
        assert!(render_image(vec![0; 15], 2, 2).is_none());
        assert!(render_image_rgba(vec![0; 15], 2, 2).is_none());
    }

    #[test]
    fn render_image_keeps_byte_order() {
        let image = render_image(vec![1, 2, 3, 4], 1, 1).expect("one pixel");
        assert_eq!(image.as_bytes(0), Some(&[1u8, 2, 3, 4][..]));
    }

    #[test]
    fn render_image_rgba_swaps_red_and_blue() {
        let image = render_image_rgba(vec![1, 2, 3, 4], 1, 1).expect("one pixel");
        assert_eq!(image.as_bytes(0), Some(&[3u8, 2, 1, 4][..]));
    }
}
