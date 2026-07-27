use anyhow::{Context, Result};
use gpui::RenderImage;
use std::path::Path;
use std::sync::Arc;

use crate::Rect;

#[derive(Clone)]
pub(crate) struct FrozenFrame {
    pub(crate) bounds: Rect,
    segments: Arc<[FrozenSegment]>,
}

#[derive(Clone)]
struct FrozenSegment {
    bounds: Rect,
    image: Arc<RenderImage>,
    pixel_width: u32,
    pixel_height: u32,
}

#[derive(Clone)]
pub(crate) struct FrozenCrop {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl FrozenFrame {
    pub(crate) fn from_bgra_segments(segments: Vec<(Rect, Vec<u8>, u32, u32)>) -> Option<Self> {
        let images = segments
            .into_iter()
            .map(|(bounds, pixels, pixel_width, pixel_height)| {
                Some((
                    bounds,
                    render_image(pixels, pixel_width, pixel_height)?,
                    pixel_width,
                    pixel_height,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        Self::from_images(images)
    }

    fn from_images(images: Vec<(Rect, Arc<RenderImage>, u32, u32)>) -> Option<Self> {
        let segments = images
            .into_iter()
            .map(|(bounds, image, pixel_width, pixel_height)| FrozenSegment {
                bounds,
                image,
                pixel_width,
                pixel_height,
            })
            .collect::<Vec<_>>();
        if segments
            .iter()
            .any(|segment| !segment.dimensions_are_valid())
        {
            return None;
        }
        let bounds = union_bounds(segments.iter().map(|segment| segment.bounds))?;
        Some(Self {
            bounds,
            segments: segments.into(),
        })
    }

    pub(crate) fn bounds(&self) -> Rect {
        self.bounds
    }

    pub(crate) fn render_image(&self, rect: Rect) -> Option<Arc<RenderImage>> {
        if let Some(segment) = self.segments.iter().find(|segment| segment.bounds == rect) {
            return Some(segment.image.clone());
        }
        let crop = self.crop(rect)?;
        let (pixels, width, height) = crop.into_bgra_parts();
        render_image(pixels, width, height)
    }

    pub(crate) fn crop(&self, rect: Rect) -> Option<FrozenCrop> {
        if !rect_within(rect, self.bounds) {
            return None;
        }
        let intersecting = self
            .segments
            .iter()
            .filter_map(|segment| {
                rect_intersection(rect, segment.bounds).map(|area| (segment, area))
            })
            .collect::<Vec<_>>();
        if intersecting.len() == 1 && intersecting[0].1 == rect {
            return intersecting[0].0.crop(rect);
        }
        compose_crop(rect, &intersecting)
    }
}

impl FrozenSegment {
    fn dimensions_are_valid(&self) -> bool {
        self.bounds.w > 0
            && self.bounds.h > 0
            && self.pixel_width > 0
            && self.pixel_height > 0
            && self.image.as_bytes(0).is_some_and(|pixels| {
                pixels.len() == self.pixel_width as usize * self.pixel_height as usize * 4
            })
    }

    fn scale(&self) -> u32 {
        let x = self.pixel_width as f64 / self.bounds.w as f64;
        let y = self.pixel_height as f64 / self.bounds.h as f64;
        x.max(y).ceil().max(1.0) as u32
    }

    fn crop(&self, rect: Rect) -> Option<FrozenCrop> {
        if !rect_within(rect, self.bounds) {
            return None;
        }
        let left = scaled_edge(rect.x - self.bounds.x, self.bounds.w, self.pixel_width)?;
        let top = scaled_edge(rect.y - self.bounds.y, self.bounds.h, self.pixel_height)?;
        let right = scaled_edge(
            rect.x - self.bounds.x + rect.w,
            self.bounds.w,
            self.pixel_width,
        )?;
        let bottom = scaled_edge(
            rect.y - self.bounds.y + rect.h,
            self.bounds.h,
            self.pixel_height,
        )?;
        let width = right.checked_sub(left)?;
        let height = bottom.checked_sub(top)?;
        let row_bytes = width as usize * 4;
        let mut pixels = Vec::with_capacity(row_bytes.checked_mul(height as usize)?);
        let source = self.image.as_bytes(0)?;
        for row in top..bottom {
            let start = (row as usize * self.pixel_width as usize + left as usize) * 4;
            pixels.extend_from_slice(source.get(start..start.checked_add(row_bytes)?)?);
        }
        Some(FrozenCrop {
            pixels,
            width,
            height,
        })
    }
}

fn compose_crop(rect: Rect, segments: &[(&FrozenSegment, Rect)]) -> Option<FrozenCrop> {
    let scale = segments.iter().map(|(segment, _)| segment.scale()).max()?;
    let width = u32::try_from(rect.w).ok()?.checked_mul(scale)?;
    let height = u32::try_from(rect.h).ok()?.checked_mul(scale)?;
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    for (segment, area) in segments {
        let crop = segment.crop(*area)?;
        let target_width = u32::try_from(area.w).ok()?.checked_mul(scale)?;
        let target_height = u32::try_from(area.h).ok()?.checked_mul(scale)?;
        let source = image::RgbaImage::from_raw(crop.width, crop.height, crop.pixels)?;
        let resized = image::imageops::resize(
            &source,
            target_width,
            target_height,
            image::imageops::FilterType::Triangle,
        );
        image::imageops::overlay(
            &mut canvas,
            &resized,
            i64::from(area.x - rect.x) * i64::from(scale),
            i64::from(area.y - rect.y) * i64::from(scale),
        );
    }
    Some(FrozenCrop {
        pixels: canvas.into_raw(),
        width,
        height,
    })
}

fn render_image(pixels: Vec<u8>, width: u32, height: u32) -> Option<Arc<RenderImage>> {
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels)?;
    Some(Arc::new(RenderImage::new(smallvec::smallvec![
        image::Frame::new(buffer)
    ])))
}

fn scaled_edge(offset: i32, logical_size: i32, pixel_size: u32) -> Option<u32> {
    let offset = u64::try_from(offset).ok()?;
    let logical_size = u64::try_from(logical_size).ok()?;
    let pixel_size = u64::from(pixel_size);
    let scaled = offset
        .checked_mul(pixel_size)?
        .checked_add(logical_size / 2)?
        / logical_size;
    u32::try_from(scaled).ok()
}

fn rect_within(rect: Rect, bounds: Rect) -> bool {
    rect.w > 0
        && rect.h > 0
        && rect.x >= bounds.x
        && rect.y >= bounds.y
        && rect.x.checked_add(rect.w).is_some_and(|right| {
            bounds
                .x
                .checked_add(bounds.w)
                .is_some_and(|bound| right <= bound)
        })
        && rect.y.checked_add(rect.h).is_some_and(|bottom| {
            bounds
                .y
                .checked_add(bounds.h)
                .is_some_and(|bound| bottom <= bound)
        })
}

fn rect_intersection(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left
        .x
        .checked_add(left.w)?
        .min(right.x.checked_add(right.w)?);
    let bottom_edge = left
        .y
        .checked_add(left.h)?
        .min(right.y.checked_add(right.h)?);
    let w = right_edge.checked_sub(x)?;
    let h = bottom_edge.checked_sub(y)?;
    (w > 0 && h > 0).then_some(Rect { x, y, w, h })
}

fn union_bounds(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    let mut rects = rects.into_iter();
    let first = rects.next()?;
    let (left, top, right, bottom) = rects.fold(
        (
            first.x,
            first.y,
            first.x.checked_add(first.w)?,
            first.y.checked_add(first.h)?,
        ),
        |(left, top, right, bottom), rect| {
            (
                left.min(rect.x),
                top.min(rect.y),
                right.max(rect.x.saturating_add(rect.w)),
                bottom.max(rect.y.saturating_add(rect.h)),
            )
        },
    );
    Some(Rect {
        x: left,
        y: top,
        w: right.checked_sub(left)?,
        h: bottom.checked_sub(top)?,
    })
}

fn bgra_to_rgba(data: &mut [u8]) {
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

impl FrozenCrop {
    pub(crate) fn save_png(mut self, path: &Path) -> Result<()> {
        bgra_to_rgba(&mut self.pixels);
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

    pub(crate) fn into_bgra_parts(self) -> (Vec<u8>, u32, u32) {
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
        let bounds = Rect {
            x: 10,
            y: 20,
            w: 4,
            h: 3,
        };
        FrozenFrame::from_bgra_segments(vec![(bounds, pixels, 4, 3)]).unwrap()
    }

    #[test]
    fn crop_uses_global_bounds_and_preserves_rows() {
        let crop = frame()
            .crop(Rect {
                x: 11,
                y: 21,
                w: 2,
                h: 2,
            })
            .unwrap();
        let (pixels, width, height) = crop.into_bgra_parts();

        assert_eq!((width, height), (2, 2));
        assert_eq!(
            pixels,
            vec![45, 25, 5, 255, 46, 26, 6, 255, 49, 29, 9, 255, 50, 30, 10, 255]
        );
    }

    #[test]
    fn crop_preserves_render_channel_order() {
        let crop = frame()
            .crop(Rect {
                x: 10,
                y: 20,
                w: 1,
                h: 1,
            })
            .unwrap();
        let (pixels, width, height) = crop.into_bgra_parts();

        assert_eq!((width, height), (1, 1));
        assert_eq!(pixels, vec![40, 20, 0, 255]);
    }

    #[test]
    fn saved_crop_converts_render_pixels_to_png_channels() {
        let path = std::env::temp_dir().join(format!(
            "qol-shot-frozen-crop-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        frame()
            .crop(Rect {
                x: 10,
                y: 20,
                w: 1,
                h: 1,
            })
            .unwrap()
            .save_png(&path)
            .unwrap();

        let saved = image::open(&path).unwrap().to_rgba8();
        let _ = std::fs::remove_file(path);

        assert_eq!(saved.dimensions(), (1, 1));
        assert_eq!(saved.into_raw(), vec![0, 20, 40, 255]);
    }

    #[test]
    fn full_frame_render_reuses_the_captured_pixel_buffer() {
        let frame = frame();
        let image = frame.render_image(frame.bounds()).unwrap();
        assert!(Arc::ptr_eq(&frame.segments[0].image, &image));
    }

    #[test]
    fn crop_preserves_native_pixel_scale() {
        let bounds = Rect {
            x: 10,
            y: 20,
            w: 2,
            h: 1,
        };
        let pixels = (0..2u8)
            .flat_map(|y| (0..4u8).flat_map(move |x| [x, y, 0, 255]))
            .collect();
        let frame = FrozenFrame::from_bgra_segments(vec![(bounds, pixels, 4, 2)]).unwrap();
        let crop = frame
            .crop(Rect {
                x: 11,
                y: 20,
                w: 1,
                h: 1,
            })
            .unwrap();
        let (pixels, width, height) = crop.into_bgra_parts();

        assert_eq!((width, height), (2, 2));
        assert_eq!(
            pixels,
            vec![2, 0, 0, 255, 3, 0, 0, 255, 2, 1, 0, 255, 3, 1, 0, 255]
        );
    }

    #[test]
    fn cross_display_crop_composes_at_the_highest_scale() {
        let left = Rect {
            x: -1,
            y: 0,
            w: 1,
            h: 1,
        };
        let right = Rect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        };
        let left_pixels = [10, 0, 0, 255].repeat(4);
        let right_pixels = vec![20, 0, 0, 255];
        let frame = FrozenFrame::from_bgra_segments(vec![
            (left, left_pixels, 2, 2),
            (right, right_pixels, 1, 1),
        ])
        .unwrap();
        let crop = frame.crop(frame.bounds()).unwrap();
        let (pixels, width, height) = crop.into_bgra_parts();

        assert_eq!((width, height), (4, 2));
        assert_eq!(
            pixels,
            [
                [10, 0, 0, 255],
                [10, 0, 0, 255],
                [20, 0, 0, 255],
                [20, 0, 0, 255],
                [10, 0, 0, 255],
                [10, 0, 0, 255],
                [20, 0, 0, 255],
                [20, 0, 0, 255],
            ]
            .concat()
        );
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
            assert!(frame().crop(rect).is_none(), "rect: {rect:?}");
        }
    }
}
