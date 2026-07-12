use anyhow::{Context, Result};
use gpui::RenderImage;
use std::path::Path;
use std::sync::Arc;

use crate::Rect;

#[derive(Clone)]
pub(crate) struct FrozenFrame {
    bounds: Rect,
    image: Arc<RenderImage>,
}

pub(crate) struct RgbaCrop {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl FrozenFrame {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn from_bgra(bounds: Rect, pixels: Vec<u8>) -> Option<Self> {
        let width = u32::try_from(bounds.w).ok()?;
        let height = u32::try_from(bounds.h).ok()?;
        let buffer =
            image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels)?;
        Some(Self {
            bounds,
            image: Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
                buffer
            )])),
        })
    }

    pub(crate) fn bounds(&self) -> Rect {
        self.bounds
    }

    pub(crate) fn render_image(&self, rect: Rect) -> Option<Arc<RenderImage>> {
        if rect == self.bounds {
            return Some(self.image.clone());
        }
        let (pixels, width, height) = self.crop_pixels(rect)?;
        let buffer =
            image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels)?;
        Some(Arc::new(RenderImage::new(smallvec::smallvec![
            image::Frame::new(buffer)
        ])))
    }

    pub(crate) fn rgba_crop(&self, rect: Rect) -> Option<RgbaCrop> {
        let (mut pixels, width, height) = self.crop_pixels(rect)?;
        bgra_to_rgba(&mut pixels);
        Some(RgbaCrop {
            pixels,
            width,
            height,
        })
    }

    fn crop_pixels(&self, rect: Rect) -> Option<(Vec<u8>, u32, u32)> {
        if rect.w <= 0 || rect.h <= 0 {
            return None;
        }
        let x = usize::try_from(rect.x.checked_sub(self.bounds.x)?).ok()?;
        let y = usize::try_from(rect.y.checked_sub(self.bounds.y)?).ok()?;
        let width = usize::try_from(rect.w).ok()?;
        let height = usize::try_from(rect.h).ok()?;
        let frame_width = usize::try_from(self.bounds.w).ok()?;
        let frame_height = usize::try_from(self.bounds.h).ok()?;
        if x.checked_add(width)? > frame_width || y.checked_add(height)? > frame_height {
            return None;
        }

        let row_bytes = width.checked_mul(4)?;
        let mut pixels = Vec::with_capacity(row_bytes.checked_mul(height)?);
        let source = self.image.as_bytes(0)?;
        for row in y..y.checked_add(height)? {
            let start = row
                .checked_mul(frame_width)?
                .checked_add(x)?
                .checked_mul(4)?;
            pixels.extend_from_slice(source.get(start..start.checked_add(row_bytes)?)?);
        }
        Some((pixels, rect.w as u32, rect.h as u32))
    }
}

fn bgra_to_rgba(data: &mut [u8]) {
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

impl RgbaCrop {
    pub(crate) fn save_png(&self, path: &Path) -> Result<()> {
        image::save_buffer_with_format(
            path,
            &self.pixels,
            self.width,
            self.height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .with_context(|| format!("failed to save frozen screenshot: {}", path.display()))
    }

    pub(crate) fn into_parts(self) -> (Vec<u8>, u32, u32) {
        (self.pixels, self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> FrozenFrame {
        let pixels: Vec<u8> = (0..12u8)
            .flat_map(|value| [value + 40, value + 20, value, 255])
            .collect();
        FrozenFrame::from_bgra(
            Rect {
                x: 10,
                y: 20,
                w: 4,
                h: 3,
            },
            pixels,
        )
        .unwrap()
    }

    #[test]
    fn crop_uses_global_bounds_and_preserves_rows() {
        let crop = frame()
            .rgba_crop(Rect {
                x: 11,
                y: 21,
                w: 2,
                h: 2,
            })
            .unwrap();
        let (pixels, width, height) = crop.into_parts();

        assert_eq!((width, height), (2, 2));
        assert_eq!(
            pixels,
            vec![5, 25, 45, 255, 6, 26, 46, 255, 9, 29, 49, 255, 10, 30, 50, 255]
        );
    }

    #[test]
    fn rgba_crop_preserves_channel_order() {
        let crop = frame()
            .rgba_crop(Rect {
                x: 10,
                y: 20,
                w: 1,
                h: 1,
            })
            .unwrap();
        let (pixels, width, height) = crop.into_parts();

        assert_eq!((width, height), (1, 1));
        assert_eq!(pixels, vec![0, 20, 40, 255]);
    }

    #[test]
    fn full_frame_render_reuses_the_captured_pixel_buffer() {
        let frame = frame();
        let image = frame.render_image(frame.bounds()).unwrap();
        assert!(Arc::ptr_eq(&frame.image, &image));
    }

    #[test]
    fn out_of_bounds_crops_are_rejected() {
        let cases = [
            Rect {
                x: 9,
                y: 20,
                w: 1,
                h: 1,
            },
            Rect {
                x: 13,
                y: 22,
                w: 2,
                h: 1,
            },
            Rect {
                x: 10,
                y: 20,
                w: 0,
                h: 1,
            },
        ];
        for rect in cases {
            assert!(frame().rgba_crop(rect).is_none(), "rect: {rect:?}");
        }
    }
}
