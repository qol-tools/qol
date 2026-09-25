use crate::{TEXT_BODY, TEXT_CAPTION, TEXT_DISPLAY, TEXT_MASTHEAD, TEXT_MICRO, TEXT_NANO};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Ui,
    Display,
    Mono,
}

impl Face {
    pub fn family(self) -> &'static str {
        match self {
            Self::Ui => crate::font_ui(),
            Self::Display => crate::font_display(),
            Self::Mono => crate::font_mono(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSpec {
    pub face: Face,
    pub size: f32,
    pub weight: u16,
    pub line_height: f32,
    pub caps: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextStyle {
    Masthead,
    Heading,
    Colophon,
    Name,
    ListName,
    Value,
    Detail,
    Hint,
    Key,
    Code,
    Label,
}

pub const LINE_KEY: f32 = 1.0;
pub const LINE_DISPLAY: f32 = 1.15;
pub const LINE_CAPTION: f32 = 1.2;
pub const LINE_TEXT: f32 = 1.25;

pub const WEIGHT_REGULAR: u16 = 400;
pub const WEIGHT_MEDIUM: u16 = 500;
pub const WEIGHT_SEMIBOLD: u16 = 600;

const fn spec(face: Face, size: f32, weight: u16, line_height: f32) -> TextSpec {
    TextSpec {
        face,
        size,
        weight,
        line_height,
        caps: false,
    }
}

impl TextStyle {
    pub const ALL: [Self; 11] = [
        Self::Masthead,
        Self::Heading,
        Self::Colophon,
        Self::Name,
        Self::ListName,
        Self::Value,
        Self::Detail,
        Self::Hint,
        Self::Key,
        Self::Code,
        Self::Label,
    ];

    pub const fn spec(self) -> TextSpec {
        use Face::{Display, Mono, Ui};
        match self {
            Self::Masthead => spec(Display, TEXT_MASTHEAD, WEIGHT_SEMIBOLD, LINE_KEY),
            Self::Heading => spec(Display, TEXT_DISPLAY, WEIGHT_SEMIBOLD, LINE_DISPLAY),
            Self::Colophon => spec(Display, TEXT_NANO, WEIGHT_SEMIBOLD, LINE_CAPTION),
            Self::Name => spec(Ui, TEXT_BODY, WEIGHT_MEDIUM, LINE_TEXT),
            Self::ListName => spec(Ui, TEXT_CAPTION, WEIGHT_MEDIUM, LINE_TEXT),
            Self::Value => spec(Ui, TEXT_BODY, WEIGHT_REGULAR, LINE_TEXT),
            Self::Detail => spec(Ui, TEXT_MICRO, WEIGHT_REGULAR, LINE_TEXT),
            Self::Hint => spec(Ui, TEXT_MICRO, WEIGHT_REGULAR, LINE_KEY),
            Self::Key => spec(Mono, TEXT_MICRO, WEIGHT_REGULAR, LINE_KEY),
            Self::Code => spec(Mono, TEXT_CAPTION, WEIGHT_REGULAR, LINE_TEXT),
            Self::Label => TextSpec {
                caps: true,
                ..spec(Ui, TEXT_NANO, WEIGHT_SEMIBOLD, LINE_CAPTION)
            },
        }
    }
}
