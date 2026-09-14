use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::RenderImage;

use super::PictureContext;

type CacheKey = (String, u32, u32, u32, Option<PictureContext>);

static CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<RenderImage>>>> = OnceLock::new();

pub fn image(
    spec: &str,
    ink: u32,
    width_px: u32,
    height_px: u32,
    context: &PictureContext,
) -> Option<Arc<RenderImage>> {
    let key = (spec.to_owned(), ink, width_px, height_px, Some(*context));
    if let Some(cached) = cached(&key) {
        return Some(cached);
    }
    let source = super::markup(spec, context)?.replace("currentColor", &format!("#{ink:06x}"));
    render(key, &source, width_px, height_px)
}

pub fn tick(ink: u32, width_px: u32, height_px: u32) -> Option<Arc<RenderImage>> {
    let key = ("tick".to_owned(), ink, width_px, height_px, None);
    if let Some(cached) = cached(&key) {
        return Some(cached);
    }
    let source = super::svg::TICK_MARKUP.replace("currentColor", &format!("#{ink:06x}"));
    render(key, &source, width_px, height_px)
}

fn cached(key: &CacheKey) -> Option<Arc<RenderImage>> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let guard = cache.lock().ok()?;
    guard.get(key).cloned()
}

fn render(key: CacheKey, source: &str, width_px: u32, height_px: u32) -> Option<Arc<RenderImage>> {
    let options = usvg_options();
    let tree = resvg::usvg::Tree::from_str(source, &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width_px, height_px)?;
    let size = tree.size();
    let transform = resvg::tiny_skia::Transform::from_scale(
        width_px as f32 / size.width(),
        height_px as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut pixels = Vec::with_capacity((width_px * height_px * 4) as usize);
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        pixels.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
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
