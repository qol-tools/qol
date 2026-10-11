use resvg::{tiny_skia, usvg};

pub(super) fn svg_png(svg: &str, pixels: u32) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(pixels, pixels)?;
    let scale = pixels as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_theme::Mark;

    #[test]
    fn every_mark_rasterizes_to_a_png() {
        for mark in Mark::ALL {
            let png = svg_png(&mark.svg(0), 32).unwrap_or_else(|| panic!("{}", mark.name()));
            assert!(png.starts_with(b"\x89PNG"), "{}", mark.name());
        }
    }
}
