use super::super::Span;

pub(super) fn span(modifiers: &gpui::Modifiers) -> Span {
    if modifiers.control {
        Span::Word
    } else {
        Span::Char
    }
}
