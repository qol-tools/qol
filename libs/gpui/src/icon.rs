use gpui::{div, img, px, App, IntoElement, ParentElement, RenderOnce, Styled, Window};
use qol_hotkeys::chord::Glyph;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Enter,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Control,
    Option,
    Shift,
    Command,
    Close,
    Tick,
    More,
    Prompt,
    Copy,
    CopyPath,
    Reveal,
    Edit,
    Pin,
    Undo,
    Redo,
    Ring,
    Square,
    Triangle,
    Home,
}

impl Icon {
    pub const ALL: [Self; 26] = [
        Self::Enter,
        Self::Tab,
        Self::Backspace,
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::Control,
        Self::Option,
        Self::Shift,
        Self::Command,
        Self::Close,
        Self::Tick,
        Self::More,
        Self::Prompt,
        Self::Copy,
        Self::CopyPath,
        Self::Reveal,
        Self::Edit,
        Self::Pin,
        Self::Undo,
        Self::Redo,
        Self::Ring,
        Self::Square,
        Self::Triangle,
        Self::Home,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Enter => "enter",
            Self::Tab => "tab",
            Self::Backspace => "backspace",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::Control => "control",
            Self::Option => "option",
            Self::Shift => "shift",
            Self::Command => "command",
            Self::Close => "close",
            Self::Tick => "tick",
            Self::More => "more",
            Self::Prompt => "prompt",
            Self::Copy => "copy",
            Self::CopyPath => "copy-path",
            Self::Reveal => "reveal",
            Self::Edit => "edit",
            Self::Pin => "pin",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Ring => "ring",
            Self::Square => "square",
            Self::Triangle => "triangle",
            Self::Home => "home",
        }
    }

    fn body(self) -> &'static str {
        match self {
            Self::Enter => r#"<path d="M10 2.5v4.5H3"/><path d="M5.5 4.5L3 7l2.5 2.5"/>"#,
            Self::Tab => r#"<path d="M1.5 6h8"/><path d="M7 3.5L9.5 6 7 8.5"/><path d="M11 3v6"/>"#,
            Self::Backspace => {
                r#"<path d="M4.5 3h7v6h-7L1.5 6z"/><path d="M6.5 4.8l2.4 2.4M8.9 4.8L6.5 7.2"/>"#
            }
            Self::Up => r#"<path d="M6 10V2"/><path d="M3.5 4.5L6 2l2.5 2.5"/>"#,
            Self::Down => r#"<path d="M6 2v8"/><path d="M3.5 7.5L6 10l2.5-2.5"/>"#,
            Self::Left => r#"<path d="M10 6H2"/><path d="M4.5 3.5L2 6l2.5 2.5"/>"#,
            Self::Right => r#"<path d="M2 6h8"/><path d="M7.5 3.5L10 6l-2.5 2.5"/>"#,
            Self::Control => r#"<path d="M3 7.5L6 4.5l3 3"/>"#,
            Self::Option => r#"<path d="M1.5 3h2.8l3.4 6h2.8"/><path d="M7.5 3h3"/>"#,
            Self::Shift => r#"<path d="M6 1.8L1.8 6.3h2.4v3.9h3.6V6.3h2.4z"/>"#,
            Self::Command => {
                r#"<path d="M4.5 4.5h3v3h-3z"/><path d="M4.5 4.5V3.2a1.3 1.3 0 1 0-1.3 1.3h1.3"/><path d="M7.5 4.5V3.2a1.3 1.3 0 1 1 1.3 1.3H7.5"/><path d="M4.5 7.5v1.3a1.3 1.3 0 1 1-1.3-1.3h1.3"/><path d="M7.5 7.5v1.3a1.3 1.3 0 1 0 1.3-1.3H7.5"/>"#
            }
            Self::Close => r#"<path d="M3 3l6 6M9 3L3 9"/>"#,
            Self::Tick => r#"<path d="M2.5 6.3l2.4 2.4 4.7-5.2"/>"#,
            Self::More => {
                r#"<circle cx="2.5" cy="6" r=".9" fill="currentColor"/><circle cx="6" cy="6" r=".9" fill="currentColor"/><circle cx="9.5" cy="6" r=".9" fill="currentColor"/>"#
            }
            Self::Prompt => r#"<path d="M4 2.5L7.5 6 4 9.5"/>"#,
            Self::Copy => {
                r#"<rect x="4.2" y="4.2" width="5.8" height="5.8" rx="1"/><path d="M7.8 4.2V2.8a.8.8 0 0 0-.8-.8H2.8a.8.8 0 0 0-.8.8V7a.8.8 0 0 0 .8.8h1.4"/>"#
            }
            Self::CopyPath => r#"<path d="M8.5 2L3.5 10"/>"#,
            Self::Reveal => r#"<path d="M3.5 8.5l5-5"/><path d="M4.5 3.5h4v4"/>"#,
            Self::Edit => r#"<path d="M2.5 9.5l.5-2.3 5.2-5.2 1.8 1.8L4.8 9z"/>"#,
            Self::Pin => {
                r#"<circle cx="6" cy="6" r="3.8"/><circle cx="6" cy="6" r="1.3" fill="currentColor"/>"#
            }
            Self::Undo => {
                r#"<path d="M4.5 2.5L2 5l2.5 2.5"/><path d="M2 5h5a2.5 2.5 0 0 1 0 5H5"/>"#
            }
            Self::Redo => {
                r#"<path d="M7.5 2.5L10 5 7.5 7.5"/><path d="M10 5H5a2.5 2.5 0 0 0 0 5h2"/>"#
            }
            Self::Ring => r#"<circle cx="6" cy="6" r="3.6"/>"#,
            Self::Square => r#"<rect x="2.6" y="2.6" width="6.8" height="6.8" rx=".6"/>"#,
            Self::Triangle => r#"<path d="M6 2.2L10 9.3H2z"/>"#,
            Self::Home => r#"<path d="M2 6.2L6 2.5l4 3.7"/><path d="M3.3 5.2v4.3h5.4V5.2"/>"#,
        }
    }

    pub fn markup(self) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }
}

impl From<Glyph> for Icon {
    fn from(glyph: Glyph) -> Self {
        match glyph {
            Glyph::Enter => Self::Enter,
            Glyph::Tab => Self::Tab,
            Glyph::Backspace => Self::Backspace,
            Glyph::Up => Self::Up,
            Glyph::Down => Self::Down,
            Glyph::Left => Self::Left,
            Glyph::Right => Self::Right,
            Glyph::Control => Self::Control,
            Glyph::Option => Self::Option,
            Glyph::Shift => Self::Shift,
            Glyph::Command => Self::Command,
        }
    }
}

#[derive(IntoElement)]
pub struct IconView {
    icon: Icon,
    size: f32,
    ink: u32,
}

pub fn icon(icon: Icon, size: f32, ink: u32) -> IconView {
    IconView { icon, size, ink }
}

impl RenderOnce for IconView {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size_px = (self.size * window.scale_factor()).round() as u32;
        let image = crate::pictures::icon(self.icon.name(), &self.icon.markup(), self.ink, size_px);
        div()
            .flex_none()
            .size(px(self.size))
            .children(image.map(|image| img(image).size(px(self.size))))
    }
}

#[cfg(test)]
mod tests {
    use super::Icon;

    #[test]
    fn every_icon_parses_as_svg() {
        for icon in Icon::ALL {
            let tree = resvg::usvg::Tree::from_str(&icon.markup(), &Default::default());
            assert!(tree.is_ok(), "{icon:?}");
        }
    }
}
