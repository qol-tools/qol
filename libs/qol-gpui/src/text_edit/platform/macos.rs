use super::super::Span;

pub(super) fn span(modifiers: &gpui::Modifiers) -> Span {
    if modifiers.platform {
        Span::Line
    } else if modifiers.alt {
        Span::Word
    } else {
        Span::Char
    }
}
