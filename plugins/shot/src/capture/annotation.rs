use std::io::Cursor;
use std::path::Path;

use anyhow::{Context as _, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NormalizedPoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PenStroke {
    pub(crate) color: u32,
    pub(crate) width: f32,
    pub(crate) points: Vec<NormalizedPoint>,
}

pub(crate) fn save_strokes(path: &Path, strokes: &[PenStroke]) -> Result<()> {
    let mut image = image::open(path)
        .with_context(|| format!("failed to load screenshot: {}", path.display()))?
        .to_rgba8();
    apply_strokes(&mut image, strokes);
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .context("failed to encode edited screenshot")?;
    qol_fs::atomic_write(path, encoded.get_ref())
        .with_context(|| format!("failed to save edited screenshot: {}", path.display()))
}

fn apply_strokes(image: &mut image::RgbaImage, strokes: &[PenStroke]) {
    let width = image.width() as f32;
    let height = image.height() as f32;
    let extent = width.min(height);
    for stroke in strokes {
        let Some(first) = stroke.points.first().copied() else {
            continue;
        };
        let radius = (stroke.width * extent / 2.0).max(0.5);
        let color = image::Rgba([
            ((stroke.color >> 16) & 0xff) as u8,
            ((stroke.color >> 8) & 0xff) as u8,
            (stroke.color & 0xff) as u8,
            255,
        ]);
        let first = pixel_point(first, width, height);
        draw_disc(image, first, radius, color);
        for points in stroke.points.windows(2) {
            draw_segment(
                image,
                pixel_point(points[0], width, height),
                pixel_point(points[1], width, height),
                radius,
                color,
            );
        }
    }
}

fn pixel_point(point: NormalizedPoint, width: f32, height: f32) -> (f32, f32) {
    (point.x * (width - 1.0), point.y * (height - 1.0))
}

fn draw_segment(
    image: &mut image::RgbaImage,
    from: (f32, f32),
    to: (f32, f32),
    radius: f32,
    color: image::Rgba<u8>,
) {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let steps = dx.hypot(dy).ceil().max(1.0) as u32;
    for step in 0..=steps {
        let progress = step as f32 / steps as f32;
        draw_disc(
            image,
            (from.0 + dx * progress, from.1 + dy * progress),
            radius,
            color,
        );
    }
}

fn draw_disc(
    image: &mut image::RgbaImage,
    center: (f32, f32),
    radius: f32,
    color: image::Rgba<u8>,
) {
    let min_x = (center.0 - radius).floor().max(0.0) as u32;
    let min_y = (center.1 - radius).floor().max(0.0) as u32;
    let max_x = (center.0 + radius)
        .ceil()
        .min(image.width().saturating_sub(1) as f32) as u32;
    let max_y = (center.1 + radius)
        .ceil()
        .min(image.height().saturating_sub(1) as f32) as u32;
    let radius_squared = radius * radius;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x as f32 - center.0;
            let dy = y as f32 - center.1;
            if dx * dx + dy * dy <= radius_squared {
                image.put_pixel(x, y, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_strokes, NormalizedPoint, PenStroke};

    #[test]
    fn raster_pen_paints_endpoints_and_preserves_other_pixels() {
        let mut image = image::RgbaImage::from_pixel(20, 20, image::Rgba([1, 2, 3, 255]));
        let strokes = [PenStroke {
            color: 0xff0000,
            width: 0.1,
            points: vec![
                NormalizedPoint { x: 0.25, y: 0.25 },
                NormalizedPoint { x: 0.75, y: 0.75 },
            ],
        }];

        apply_strokes(&mut image, &strokes);

        assert_eq!(*image.get_pixel(5, 5), image::Rgba([255, 0, 0, 255]));
        assert_eq!(*image.get_pixel(14, 14), image::Rgba([255, 0, 0, 255]));
        assert_eq!(*image.get_pixel(0, 19), image::Rgba([1, 2, 3, 255]));
    }
}
