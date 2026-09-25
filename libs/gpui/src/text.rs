use gpui::{px, App, Font, FontWeight, Hsla, SharedString, TextRun, Window};

pub fn shaped_width(window: &mut Window, text: &str, run_font: Font, font_size: f32) -> f32 {
    window
        .text_system()
        .shape_line(
            SharedString::from(text.to_owned()),
            px(font_size),
            &[TextRun {
                len: text.len(),
                font: run_font,
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        )
        .width
        .into()
}

pub fn truncate_to_width(
    text: &str,
    font: Font,
    font_size: f32,
    font_weight: FontWeight,
    max_width: f32,
    ellipsis: &str,
    cx: &App,
) -> SharedString {
    let font = Font {
        weight: font_weight,
        ..font
    };
    let mut runs = vec![TextRun {
        len: text.len(),
        font: font.clone(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    cx.text_system()
        .line_wrapper(font, px(font_size))
        .truncate_line(
            SharedString::from(text.to_owned()),
            px(max_width),
            ellipsis,
            &mut runs,
        )
}

pub trait TextStyled: gpui::Styled + Sized {
    fn text(mut self, style: qol_theme::TextStyle) -> Self {
        let spec = style.spec();
        let text = self.text_style().get_or_insert_with(Default::default);
        text.font_family = Some(SharedString::from(spec.face.family()));
        text.font_size = Some(px(spec.size).into());
        text.font_weight = Some(FontWeight(f32::from(spec.weight)));
        text.line_height = Some(gpui::relative(spec.line_height));
        text.line_clamp = Some(1);
        text.text_overflow = Some(gpui::TextOverflow::Truncate(SharedString::from("…")));
        self
    }

    fn wraps(mut self) -> Self {
        self.text_style()
            .get_or_insert_with(Default::default)
            .line_clamp = Some(usize::MAX);
        self
    }
}

impl<E: gpui::Styled> TextStyled for E {}

pub fn cased(style: qol_theme::TextStyle, text: &str) -> String {
    if style.spec().caps {
        text.to_uppercase()
    } else {
        text.to_owned()
    }
}
