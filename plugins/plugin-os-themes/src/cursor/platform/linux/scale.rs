pub fn scale_bilinear(src_pixels: &[u32], src_width: u32, src_height: u32, dst: &mut [u32], dst_width: u32, dst_height: u32) {
    let Some(src_len) = checked_pixel_count(src_width, src_height) else {
        return;
    };
    if src_pixels.len() < src_len {
        return;
    }
    let Some(dst_len) = checked_pixel_count(dst_width, dst_height) else {
        return;
    };
    if dst.len() < dst_len {
        return;
    }
    let request = ScaleRequest::new(
        PixelGrid::new(src_pixels, ImageSize::new(src_width, src_height)),
        ImageSize::new(dst_width, dst_height),
    );
    for dy in 0..request.dst.height {
        for dx in 0..request.dst.width {
            let point = PixelPoint { x: dx, y: dy };
            dst[(dy * request.dst.width + dx) as usize] = scaled_pixel(&request, point);
        }
    }
}

struct ScaleRequest<'a> {
    src: PixelGrid<'a>,
    dst: ImageSize,
}

impl<'a> ScaleRequest<'a> {
    fn new(src: PixelGrid<'a>, dst: ImageSize) -> Self {
        Self { src, dst }
    }
}

#[derive(Clone, Copy)]
struct PixelPoint {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy)]
struct ImageSize {
    width: u32,
    height: u32,
}

impl ImageSize {
    fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

struct PixelGrid<'a> {
    pixels: &'a [u32],
    width: u32,
    height: u32,
}

impl<'a> PixelGrid<'a> {
    fn new(pixels: &'a [u32], size: ImageSize) -> Self {
        Self {
            pixels,
            width: size.width,
            height: size.height,
        }
    }

    fn pixel(&self, x: u32, y: u32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }
}

struct SourcePoint {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    tx: f32,
    ty: f32,
}

#[derive(Clone, Copy)]
struct BlendFactors {
    tx: f32,
    ty: f32,
}

impl From<&SourcePoint> for BlendFactors {
    fn from(source: &SourcePoint) -> Self {
        Self {
            tx: source.tx,
            ty: source.ty,
        }
    }
}

fn scaled_pixel(request: &ScaleRequest<'_>, point: PixelPoint) -> u32 {
    let source = source_point(request, point);
    let corners = pixel_corners(request, &source);
    blend_pixel(corners, BlendFactors::from(&source))
}

fn source_point(request: &ScaleRequest<'_>, point: PixelPoint) -> SourcePoint {
    let sx = point.x as f32 * (request.src.width as f32 - 1.0) / scale_span(request.dst.width);
    let sy = point.y as f32 * (request.src.height as f32 - 1.0) / scale_span(request.dst.height);
    let x0 = sx as u32;
    let y0 = sy as u32;
    SourcePoint {
        x0,
        y0,
        x1: (x0 + 1).min(request.src.width - 1),
        y1: (y0 + 1).min(request.src.height - 1),
        tx: sx - x0 as f32,
        ty: sy - y0 as f32,
    }
}

fn scale_span(length: u32) -> f32 {
    (length as f32 - 1.0).max(1.0)
}

fn checked_pixel_count(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    width.checked_mul(height)
}

fn pixel_corners(request: &ScaleRequest<'_>, source: &SourcePoint) -> [u32; 4] {
    [
        request.src.pixel(source.x0, source.y0),
        request.src.pixel(source.x1, source.y0),
        request.src.pixel(source.x0, source.y1),
        request.src.pixel(source.x1, source.y1),
    ]
}

fn blend_pixel(corners: [u32; 4], blend: BlendFactors) -> u32 {
    let weights = bilinear_weights(blend);
    let alpha = blended_channel(corners, weights, 24);
    if alpha <= f32::EPSILON {
        return 0;
    }
    let red = blended_premultiplied_channel(corners, weights, 16) * 255.0 / alpha;
    let green = blended_premultiplied_channel(corners, weights, 8) * 255.0 / alpha;
    let blue = blended_premultiplied_channel(corners, weights, 0) * 255.0 / alpha;
    pack_pixel(alpha, red, green, blue)
}

fn bilinear_weights(blend: BlendFactors) -> [f32; 4] {
    [
        (1.0 - blend.tx) * (1.0 - blend.ty),
        blend.tx * (1.0 - blend.ty),
        (1.0 - blend.tx) * blend.ty,
        blend.tx * blend.ty,
    ]
}

fn blended_channel(corners: [u32; 4], weights: [f32; 4], shift: u32) -> f32 {
    let [p00, p10, p01, p11] = corners;
    let [w00, w10, w01, w11] = weights;
    let channel = |pixel: u32| ((pixel >> shift) & 0xFF) as f32;
    channel(p00) * w00 + channel(p10) * w10 + channel(p01) * w01 + channel(p11) * w11
}

fn blended_premultiplied_channel(corners: [u32; 4], weights: [f32; 4], shift: u32) -> f32 {
    let [p00, p10, p01, p11] = corners;
    let [w00, w10, w01, w11] = weights;
    let channel = |pixel: u32| ((pixel >> shift) & 0xFF) as f32;
    let alpha = |pixel: u32| ((pixel >> 24) & 0xFF) as f32 / 255.0;
    channel(p00) * alpha(p00) * w00
        + channel(p10) * alpha(p10) * w10
        + channel(p01) * alpha(p01) * w01
        + channel(p11) * alpha(p11) * w11
}

fn pack_pixel(alpha: f32, red: f32, green: f32, blue: f32) -> u32 {
    (rounded_byte(alpha) << 24)
        | (rounded_byte(red) << 16)
        | (rounded_byte(green) << 8)
        | rounded_byte(blue)
}

fn rounded_byte(value: f32) -> u32 {
    value.round().clamp(0.0, 255.0) as u32
}

#[cfg(test)]
mod tests {
    use super::scale_bilinear;

    #[test]
    fn scale_bilinear_2x_maps_source_corners() {
        let src = [0xFFFF0000u32, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF];
        let mut dst = [0u32; 16];
        scale_bilinear(&src, 2, 2, &mut dst, 4, 4);
        assert_eq!(dst[0], 0xFFFF0000, "top-left corner");
        assert_eq!(dst[3], 0xFF00FF00, "top-right corner");
        assert_eq!(dst[12], 0xFF0000FF, "bottom-left corner");
        assert_eq!(dst[15], 0xFFFFFFFF, "bottom-right corner");
    }
}
