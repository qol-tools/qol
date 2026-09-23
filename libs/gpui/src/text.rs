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
