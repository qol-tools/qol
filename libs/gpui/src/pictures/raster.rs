use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::RenderImage;

use super::{fitted, PictureContext, Tone};

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    spec: String,
    ink: u32,
    width_px: u32,
    height_px: u32,
    context: Option<PictureContext>,
    fit: Option<(Tone, u32)>,
}

static CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<RenderImage>>>> = OnceLock::new();

pub fn image(
    spec: &str,
    ink: u32,
    width_px: u32,
    height_px: u32,
    context: &PictureContext,
) -> Option<Arc<RenderImage>> {
    let key = CacheKey {
        spec: spec.to_owned(),
        ink,
        width_px,
        height_px,
        context: Some(*context),
        fit: None,
    };
    if let Some(cached) = cached(&key) {
        return Some(cached);
    }
    let source = super::markup(spec, context)?.replace("currentColor", &format!("#{ink:06x}"));
    render_markup(key, &source, width_px, height_px, false)
}

pub fn tick(ink: u32, width_px: u32, height_px: u32) -> Option<Arc<RenderImage>> {
    let key = CacheKey {
        spec: "tick".to_owned(),
        ink,
        width_px,
        height_px,
        context: None,
        fit: None,
    };
    if let Some(cached) = cached(&key) {
        return Some(cached);
    }
    let source = super::svg::TICK_MARKUP.replace("currentColor", &format!("#{ink:06x}"));
    render_markup(key, &source, width_px, height_px, false)
}

pub fn chevron(line: u32, width_px: u32, height_px: u32) -> Option<Arc<RenderImage>> {
    let key = CacheKey {
        spec: "chevron".to_owned(),
        ink: line,
        width_px,
        height_px,
        context: None,
        fit: None,
    };
    if let Some(cached) = cached(&key) {
        return Some(cached);
    }
    let source = super::svg::CHEVRON_MARKUP.replace("currentColor", &format!("#{line:06x}"));
    render_markup(key, &source, width_px, height_px, false)
}

pub fn fitted_image(
    spec: &str,
    line: u32,
    tone: Tone,
    width: f32,
    height: f32,
    scale_factor: f32,
    context: &PictureContext,
) -> Option<Arc<RenderImage>> {
    let tone = fitted::tone_for(spec, tone);
    let width_px = (width * scale_factor).round() as u32;
    let height_px = (height * scale_factor).round() as u32;
    if width_px == 0 || height_px == 0 {
        return None;
    }
    let key = CacheKey {
        spec: spec.to_owned(),
        ink: line,
        width_px,
        height_px,
        context: Some(*context),
        fit: Some((tone, scale_factor.to_bits())),
    };
    if let Some(cached) = cached(&key) {
        return Some(cached);
    }
    let markup = fitted::markup(spec, tone, width, height, context)?;
    let source = markup.replace("currentColor", &format!("#{line:06x}"));
    if fitted::is_tile(spec) {
        let tree = parse(&source)?;
        let transform = scale_transform(&tree, width_px, height_px);
        return raster(
            key,
            &tree,
            width_px,
            height_px,
            transform,
            tone == Tone::Rest,
        );
    }
    let first = parse(&source)?;
    let bounds = first.root().abs_stroke_bounding_box();
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    let (fit, _, _) = fitted::fit_transform(
        bounds.x(),
        bounds.y(),
        bounds.width(),
        bounds.height(),
        width_px,
        height_px,
    );
    let stroke = scale_factor / fit;
    let source = fitted::with_default_stroke(&source, stroke);
    let tree = parse(&source)?;
    let bounds = tree.root().abs_stroke_bounding_box();
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    let (scale, tx, ty) = fitted::fit_transform(
        bounds.x(),
        bounds.y(),
        bounds.width(),
        bounds.height(),
        width_px,
        height_px,
    );
    let transform = resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, tx, ty);
    raster(
        key,
        &tree,
        width_px,
        height_px,
        transform,
        tone == Tone::Rest,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn stacked_image(
    front: &str,
    back: &str,
    line: u32,
    tone: Tone,
    width: f32,
    height: f32,
    scale_factor: f32,
    context: &PictureContext,
) -> Option<Arc<RenderImage>> {
    let width_px = (width * scale_factor).round() as u32;
    let height_px = (height * scale_factor).round() as u32;
    if width_px == 0 || height_px == 0 {
        return None;
    }
    let key = CacheKey {
        spec: format!("stack:{front}|{back}"),
        ink: line,
        width_px,
        height_px,
        context: Some(*context),
        fit: Some((tone, scale_factor.to_bits())),
    };
    if let Some(cached) = cached(&key) {
        return Some(cached);
    }
    let back = stack_part(back, tone, 12.0, 0.0, "back", context)?;
    let front = stack_part(front, tone, 0.0, 7.5, "front", context)?;
    let markup = fitted::stacked_markup(&back, &front);
    let source = markup.replace("currentColor", &format!("#{line:06x}"));
    let tree = parse(&source)?;
    let transform = scale_transform(&tree, width_px, height_px);
    raster(
        key,
        &tree,
        width_px,
        height_px,
        transform,
        tone == Tone::Rest,
    )
}

fn stack_part(
    spec: &str,
    tone: Tone,
    x: f32,
    y: f32,
    prefix: &str,
    context: &PictureContext,
) -> Option<String> {
    if fitted::is_tile(spec) {
        return fitted::stack_tile(spec, tone, x, y, context);
    }
    let drawing = fitted::markup(
        spec,
        tone,
        fitted::STACK_WIDTH,
        fitted::STACK_HEIGHT,
        context,
    )?;
    let tree = parse(&drawing)?;
    let bounds = tree.root().abs_stroke_bounding_box();
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    let fit = (fitted::STACK_WIDTH / bounds.width()).min(fitted::STACK_HEIGHT / bounds.height());
    let drawing = fitted::with_default_stroke(&drawing, 1.0 / fit);
    let tree = parse(&drawing)?;
    let bounds = tree.root().abs_stroke_bounding_box();
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    fitted::stack_drawing(
        &drawing,
        x,
        y,
        (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
        prefix,
    )
}

fn cached(key: &CacheKey) -> Option<Arc<RenderImage>> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let guard = cache.lock().ok()?;
    guard.get(key).cloned()
}

fn render_markup(
    key: CacheKey,
    source: &str,
    width_px: u32,
    height_px: u32,
    desaturate: bool,
) -> Option<Arc<RenderImage>> {
    let tree = parse(source)?;
    let transform = scale_transform(&tree, width_px, height_px);
    raster(key, &tree, width_px, height_px, transform, desaturate)
}

fn parse(source: &str) -> Option<resvg::usvg::Tree> {
    resvg::usvg::Tree::from_str(source, &usvg_options()).ok()
}

fn scale_transform(
    tree: &resvg::usvg::Tree,
    width_px: u32,
    height_px: u32,
) -> resvg::tiny_skia::Transform {
    let size = tree.size();
    resvg::tiny_skia::Transform::from_scale(
        width_px as f32 / size.width(),
        height_px as f32 / size.height(),
    )
}

fn raster(
    key: CacheKey,
    tree: &resvg::usvg::Tree,
    width_px: u32,
    height_px: u32,
    transform: resvg::tiny_skia::Transform,
    desaturate: bool,
) -> Option<Arc<RenderImage>> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width_px, height_px)?;
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let mut pixels = Vec::with_capacity((width_px * height_px * 4) as usize);
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        pixels.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    if desaturate {
        fitted::desaturate(&mut pixels);
    }
    let rendered = crate::image::render_image_rgba(pixels, width_px, height_px)?;
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().ok()?;
    guard.insert(key, rendered.clone());
    Some(rendered)
}

fn usvg_options() -> resvg::usvg::Options<'static> {
    resvg::usvg::Options {
        font_family: qol_theme::font_ui().to_owned(),
        fontdb: font_database(),
        ..Default::default()
    }
}

fn font_database() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut database = resvg::usvg::fontdb::Database::new();
            for face in crate::fonts::FACES {
                database.load_font_data(face.to_vec());
            }
            Arc::new(database)
        })
        .clone()
}
