use gpui::*;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{css_rgba_milli, TextStyle, RADIUS_TIGHT, SPACE_SNUG};

pub const HEIGHT: f32 = 20.0;
pub const PAD: f32 = SPACE_SNUG;
pub const WHAT: u32 = 0xd4a57c;
pub const FACT: u32 = 0x86b0de;
pub const LAW: u32 = 0xa9c48a;
pub const USE: u32 = 0x7cc4b8;
pub const DIM: u32 = 0x9a978f;
pub const NEG: u32 = 0xe0897d;

pub fn tag(text: impl Into<SharedString>, hue: u32, alpha: u16, ink: u32) -> Div {
    div()
        .flex_none()
        .h(px(HEIGHT))
        .px(px(PAD))
        .flex()
        .items_center()
        .rounded(px(RADIUS_TIGHT))
        .bg(rgba(css_rgba_milli(hue, alpha).packed()))
        .text(TextStyle::Detail)
        .text_color(rgb(ink))
        .child(text.into())
}
