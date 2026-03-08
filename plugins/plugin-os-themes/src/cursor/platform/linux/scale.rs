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
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        out |= blended_channel(corners, blend, shift) << shift;
    }
    out
}

fn blended_channel(corners: [u32; 4], blend: BlendFactors, shift: u32) -> u32 {
    let [p00, p10, p01, p11] = corners;
    let channel = |pixel: u32| ((pixel >> shift) & 0xFF) as f32;
    let value = channel(p00) * (1.0 - blend.tx) * (1.0 - blend.ty)
        + channel(p10) * blend.tx * (1.0 - blend.ty)
        + channel(p01) * (1.0 - blend.tx) * blend.ty
        + channel(p11) * blend.tx * blend.ty;
    value as u32 & 0xFF
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
