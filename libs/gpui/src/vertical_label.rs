use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    img, px, App, Global, Image, ImageFormat, IntoElement, RenderOnce, SharedString, Window,
};

pub const WIDTH: f32 = qol_theme::SPACE_PAD;
pub const HEIGHT: f32 = qol_theme::LIST_ENTRY_HEIGHTS[1] - 2.0 * qol_theme::SPACE_STACK;

#[derive(Default)]
struct LabelCache(HashMap<SharedString, Arc<Image>>);
impl Global for LabelCache {}

#[derive(IntoElement)]
pub struct VerticalLabel {
    label: SharedString,
}

impl VerticalLabel {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl RenderOnce for VerticalLabel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let image = if let Some(image) = cx.default_global::<LabelCache>().0.get(&self.label) {
            image.clone()
        } else {
            let image = Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                label_svg(&self.label).into_bytes(),
            ));
            let cache = &mut cx.default_global::<LabelCache>().0;
            if cache.len() >= 64 {
                cache.clear();
            }
            cache.insert(self.label, image.clone());
            image
        };
        img(image).w(px(WIDTH)).h(px(HEIGHT))
    }
}

pub(crate) struct LabelOutline {
    pub d: String,
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub upem: f32,
}

struct OutlinePath {
    d: String,
    pen_x: f32,
}

impl ttf_parser::OutlineBuilder for OutlinePath {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "M {} {} ", x + self.pen_x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "L {} {} ", x + self.pen_x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(
            self.d,
            "Q {} {} {} {} ",
            x1 + self.pen_x,
            y1,
            x + self.pen_x,
            y
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(
            self.d,
            "C {} {} {} {} {} {} ",
            x1 + self.pen_x,
            y1,
            x2 + self.pen_x,
            y2,
            x + self.pen_x,
            y
        );
    }

    fn close(&mut self) {
        self.d.push_str("Z ");
    }
}

pub(crate) fn label_outline(label: &str) -> Option<LabelOutline> {
    let face = ttf_parser::Face::parse(crate::fonts::SANS_MEDIUM, 0).ok()?;
    let mut path = OutlinePath {
        d: String::new(),
        pen_x: 0.0,
    };
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut outlined = false;
    let mut pen_x = 0.0f32;
    for ch in label.chars() {
        let Some(glyph) = face.glyph_index(ch) else {
            continue;
        };
        path.pen_x = pen_x;
        if let Some(rect) = face.outline_glyph(glyph, &mut path) {
            outlined = true;
            min_x = min_x.min(rect.x_min as f32 + pen_x);
            min_y = min_y.min(rect.y_min as f32);
            max_x = max_x.max(rect.x_max as f32 + pen_x);
            max_y = max_y.max(rect.y_max as f32);
        }
        pen_x += face.glyph_hor_advance(glyph).unwrap_or(0) as f32;
    }
    outlined.then(|| LabelOutline {
        d: path.d,
        min_x,
        min_y,
        max_x,
        max_y,
        upem: face.units_per_em() as f32,
    })
}

fn label_svg(label: &str) -> String {
    let ink = qol_theme::LIGHT_SYSTEM.text_primary;
    let Some(outline) = label_outline(label) else {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}"></svg>"#
        );
    };
    let scale = qol_theme::TEXT_IDENTITY / outline.upem;
    let available = HEIGHT - 2.0 * qol_theme::SPACE_TIGHT;
    let natural = (outline.max_x - outline.min_x) * scale;
    let sx = if natural > 0.0 {
        (available / natural).min(1.0)
    } else {
        1.0
    };
    let cx = (outline.min_x + outline.max_x) / 2.0;
    let cy = (outline.min_y + outline.max_y) / 2.0;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}"><path transform="translate({} {}) rotate(-90) scale({} {}) translate({} {})" fill="#{ink:06x}" d="{}"/></svg>"##,
        WIDTH / 2.0,
        HEIGHT / 2.0,
        sx * scale,
        -scale,
        -cx,
        -cy,
        outline.d,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale_factors(svg: &str) -> (f32, f32) {
        let start = svg.find("scale(").expect("scale(") + "scale(".len();
        let end = svg[start..].find(')').expect(")") + start;
        let mut parts = svg[start..end].split_whitespace();
        (
            parts.next().expect("x").parse().expect("x f32"),
            parts.next().expect("y").parse().expect("y f32"),
        )
    }

    #[test]
    fn a_tool_name_outlines_every_glyph() {
        let outline = label_outline("codex").expect("outline");
        assert!(outline.d.starts_with('M'));
        assert!(outline.d.contains('Z'));
        assert!(outline.max_x > outline.min_x);
        assert!(outline.max_y > outline.min_y);
        assert!(outline.max_y - outline.min_y <= outline.upem);
    }

    #[test]
    fn an_empty_label_has_no_outline() {
        assert!(label_outline("").is_none());
    }

    #[test]
    fn a_glyphless_label_has_no_outline() {
        assert!(label_outline("\u{10FFFF}").is_none());
    }

    #[test]
    fn the_svg_names_no_font() {
        let svg = label_svg("claude");
        println!("{svg}");
        assert!(svg.contains("<path"));
        assert!(svg.contains("d=\"M"));
        assert!(!svg.contains("font-family"));
        assert!(!svg.contains("<text"));
    }

    #[test]
    fn a_long_label_is_squeezed_into_the_tab() {
        let (squeezed_x, squeezed_y) = scale_factors(&label_svg("a rather long tool name"));
        assert!(squeezed_x < squeezed_y.abs());
        let (pi_x, pi_y) = scale_factors(&label_svg("pi"));
        assert_eq!(pi_x, pi_y.abs());
    }
}
