use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use std::path::PathBuf;

pub fn preview_cache_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir().or_else(|| Some(std::env::temp_dir()))?;
    let path = base
        .join("qol-tray")
        .join("plugin-alt-tab")
        .join("previews");
    std::fs::create_dir_all(&path).ok()?;
    Some(path)
}

pub fn cached_preview_path(window_id: u32) -> Option<String> {
    let cache_dir = preview_cache_dir()?;
    let path = cache_dir.join(format!("{}.png", window_id));
    if !path.is_file() {
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

/// Downscale RGBA pixel data and save as PNG. Input must be RGBA (R=0, G=1, B=2, A=3).
pub fn downscale_and_save_preview(
    window_id: u32,
    rgba_data: &[u8],
    src_w: usize,
    src_h: usize,
    max_w: usize,
    max_h: usize,
) -> Option<String> {
    let (thumb, thumb_w, thumb_h) = downscale_rgba(rgba_data, src_w, src_h, max_w, max_h);
    if thumb_w == 0 || thumb_h == 0 {
        return None;
    }

    let cache_dir = preview_cache_dir()?;
    let path = cache_dir.join(format!("{}.png", window_id));
    let file = std::fs::File::create(&path).ok()?;
    let writer = std::io::BufWriter::new(file);
    let encoder =
        PngEncoder::new_with_quality(writer, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(&thumb, thumb_w as u32, thumb_h as u32, ExtendedColorType::Rgba8)
        .ok()?;
    Some(path.to_string_lossy().to_string())
}

pub fn downscale_rgba(
    data: &[u8],
    src_w: usize,
    src_h: usize,
    max_w: usize,
    max_h: usize,
) -> (Vec<u8>, usize, usize) {
    if src_w == 0 || src_h == 0 || max_w == 0 || max_h == 0 {
        return (Vec::new(), 0, 0);
    }

    let pixels = match src_w.checked_mul(src_h) {
        Some(value) if value > 0 => value,
        _ => return (Vec::new(), 0, 0),
    };
    const BYTES_PER_PIXEL: usize = 4;
    if data.len() < pixels * BYTES_PER_PIXEL {
        return (Vec::new(), 0, 0);
    }

    let scale_w = max_w as f32 / src_w as f32;
    let scale_h = max_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h).min(1.0);

    let scaled_w = ((src_w as f32 * scale).round() as usize).max(1).min(max_w);
    let scaled_h = ((src_h as f32 * scale).round() as usize).max(1).min(max_h);

    let mut canvas = vec![0u8; max_w * max_h * 4];
    let offset_x = (max_w - scaled_w) / 2;
    let offset_y = (max_h - scaled_h) / 2;

    for y in 0..scaled_h {
        let src_y = (y * src_h) / scaled_h;
        for x in 0..scaled_w {
            let src_x = (x * src_w) / scaled_w;
            let src_base = (src_y * src_w + src_x) * BYTES_PER_PIXEL;
            if src_base + 4 > data.len() {
                continue;
            }
            let r = data[src_base];
            let g = data[src_base + 1];
            let b = data[src_base + 2];
            let a = data[src_base + 3];

            let dst_x = offset_x + x;
            let dst_y = offset_y + y;
            let dst_i = (dst_y * max_w + dst_x) * 4;
            canvas[dst_i..dst_i + 4].copy_from_slice(&[r, g, b, a]);
        }
    }

    (canvas, max_w, max_h)
}
