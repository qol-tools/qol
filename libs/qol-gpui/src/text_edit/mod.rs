//! Editing conventions for hand-rolled text fields: how far a horizontal
//! motion or delete reaches for the held modifiers, and the word-boundary
//! scans behind it.
//!
//! Cursors are char indices into `text`. A word is a run of alphanumerics
//! and underscores; everything else separates words.

mod platform;

use gpui::Modifiers;

/// How far a horizontal cursor motion or a delete reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Char,
    Word,
    Line,
}

/// The span the host desktop's text fields use for these modifiers.
pub fn span(modifiers: &Modifiers) -> Span {
    platform::span(modifiers)
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Start of the word before `cursor`, skipping any separators in between.
pub fn word_start_before(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut idx = cursor.min(chars.len());
    while idx > 0 && !is_word_char(chars[idx - 1]) {
        idx -= 1;
    }
    while idx > 0 && is_word_char(chars[idx - 1]) {
        idx -= 1;
    }
    idx
}

/// End of the word after `cursor`, skipping any separators in between.
pub fn word_end_after(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut idx = cursor.min(chars.len());
    while idx < chars.len() && !is_word_char(chars[idx]) {
        idx += 1;
    }
    while idx < chars.len() && is_word_char(chars[idx]) {
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::{span, word_end_after, word_start_before, Span};
    use gpui::Modifiers;

    #[test]
    fn plain_and_shift_keys_move_by_char_everywhere() {
        assert_eq!(span(&Modifiers::none()), Span::Char);
        assert_eq!(span(&Modifiers::shift()), Span::Char);
    }

    #[test]
    fn some_modifier_reaches_a_word_everywhere() {
        let reaches_word = [Modifiers::control(), Modifiers::alt(), Modifiers::command()]
            .iter()
            .any(|modifiers| span(modifiers) == Span::Word);
        assert!(reaches_word);
    }

    #[test]
    fn word_start_skips_separators_then_the_word() {
        for (text, cursor, expect) in [
            ("qol memory", 10, 4),
            ("qol memory  ", 12, 4),
            ("qol-shot", 8, 4),
            ("qol memory", 5, 4),
            ("qol", 0, 0),
            ("héllo wörld", 11, 6),
            ("qol", 99, 0),
        ] {
            assert_eq!(
                word_start_before(text, cursor),
                expect,
                "{text:?} at {cursor}"
            );
        }
    }

    #[test]
    fn word_end_skips_separators_then_the_word() {
        for (text, cursor, expect) in [
            ("qol memory", 0, 3),
            ("qol memory", 3, 10),
            ("qol  memory", 3, 11),
            ("qol-shot", 3, 8),
            ("qol", 3, 3),
            ("héllo wörld", 0, 5),
            ("qol", 99, 3),
        ] {
            assert_eq!(word_end_after(text, cursor), expect, "{text:?} at {cursor}");
        }
    }
}
