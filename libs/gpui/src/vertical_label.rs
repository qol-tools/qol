use std::collections::HashMap;
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let image = if let Some(image) = cx.default_global::<LabelCache>().0.get(&self.label) {
            image.clone()
        } else {
            let mut font = gpui::font(qol_theme::font_ui());
            font.weight = gpui::FontWeight::MEDIUM;
            let line = window.text_system().shape_line(
                self.label.clone(),
                px(qol_theme::TEXT_IDENTITY),
                &[gpui::TextRun {
                    len: self.label.len(),
                    font,
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let length = f32::from(line.width).min(HEIGHT - 2.0 * qol_theme::SPACE_TIGHT);
            let image = Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                label_svg(&self.label, length).into_bytes(),
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

fn label_svg(label: &str, length: f32) -> String {
    let label = xml_escape(label);
    let font = xml_escape(qol_theme::font_ui());
    let font_size = qol_theme::TEXT_IDENTITY;
    let ink = qol_theme::LIGHT_SYSTEM.text_primary;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}"><text transform="translate({} {}) rotate(-90)" text-anchor="middle" dominant-baseline="central" textLength="{length}" lengthAdjust="spacingAndGlyphs" font-family="{font}" font-size="{font_size}" font-weight="500" fill="#{ink:06x}">{label}</text></svg>"##,
        WIDTH / 2.0,
        HEIGHT / 2.0,
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
