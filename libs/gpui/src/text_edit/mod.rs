//! Editing conventions for hand-rolled text fields: how far a horizontal
//! motion or delete reaches for the held modifiers, and the word-boundary
//! scans behind it.
//!
//! Cursors are char indices into `text`. A word is a run of alphanumerics
//! and underscores; everything else separates words.

mod field;
mod platform;

pub use field::{single_line, visible_char_count, CaretStyle, TextField, TextFieldElement};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Changed,
    Handled,
    Ignored,
}

pub fn apply_edit_key(
    field: &mut TextField,
    keystroke: &gpui::Keystroke,
    paste: impl FnOnce() -> Option<String>,
) -> EditKey {
    let key = keystroke.key.as_str();
    let modifiers = &keystroke.modifiers;
    if modifiers.secondary() && key == "v" {
        return match paste() {
            Some(text) => {
                if field.paste(&text) {
                    EditKey::Changed
                } else {
                    EditKey::Handled
                }
            }
            None => EditKey::Handled,
        };
    }
    let span = span(modifiers);
    match key {
        "backspace" => {
            if field.backspace(span) {
                EditKey::Changed
            } else {
                EditKey::Handled
            }
        }
        "delete" => {
            if field.delete_forward(span) {
                EditKey::Changed
            } else {
                EditKey::Handled
            }
        }
        "left" => {
            field.move_left(modifiers.shift, span);
            EditKey::Handled
        }
        "right" => {
            field.move_right(modifiers.shift, span);
            EditKey::Handled
        }
        "home" => {
            field.move_home(modifiers.shift);
            EditKey::Handled
        }
        "end" => {
            field.move_end(modifiers.shift);
            EditKey::Handled
        }
        "a" if modifiers.secondary() => {
            field.select_all();
            EditKey::Handled
        }
        _ => {
            if modifiers.control || modifiers.alt || modifiers.platform {
                return EditKey::Ignored;
            }
            match keystroke
                .key_char
                .as_deref()
                .filter(|text| !text.chars().any(char::is_control))
            {
                Some(character) => {
                    field.insert_str(character);
                    EditKey::Changed
                }
                None => EditKey::Ignored,
            }
        }
    }
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
    use super::{
        apply_edit_key, span, word_end_after, word_start_before, EditKey, Span, TextField,
    };
    use gpui::{Keystroke, Modifiers};

    fn key(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    #[test]
    fn typing_a_character_changes_the_text() {
        let mut field = TextField::new();
        assert_eq!(
            apply_edit_key(&mut field, &key("a", Some("a"), Modifiers::none()), || None),
            EditKey::Changed
        );
        assert_eq!(field.text(), "a");
    }

    #[test]
    fn backspace_empties_a_typed_field() {
        let mut field = TextField::with_text("a");
        assert_eq!(
            apply_edit_key(
                &mut field,
                &key("backspace", None, Modifiers::none()),
                || None
            ),
            EditKey::Changed
        );
        assert_eq!(field.text(), "");
    }

    #[test]
    fn backspace_on_an_empty_field_is_handled() {
        let mut field = TextField::new();
        assert_eq!(
            apply_edit_key(
                &mut field,
                &key("backspace", None, Modifiers::none()),
                || None
            ),
            EditKey::Handled
        );
    }

    #[test]
    fn a_character_replaces_the_selection_from_shift_left() {
        let mut field = TextField::with_text("a");
        assert_eq!(
            apply_edit_key(&mut field, &key("left", None, Modifiers::shift()), || None),
            EditKey::Handled
        );
        assert!(field.selected_range().is_some());
        assert_eq!(
            apply_edit_key(&mut field, &key("b", Some("b"), Modifiers::none()), || None),
            EditKey::Changed
        );
        assert_eq!(field.text(), "b");
    }

    #[test]
    fn alt_held_keys_are_ignored() {
        let mut field = TextField::with_text("a");
        assert_eq!(
            apply_edit_key(&mut field, &key("x", Some("x"), Modifiers::alt()), || None),
            EditKey::Ignored
        );
        assert_eq!(field.text(), "a");
    }

    #[test]
    fn escape_is_ignored() {
        let mut field = TextField::new();
        assert_eq!(
            apply_edit_key(&mut field, &key("escape", None, Modifiers::none()), || None),
            EditKey::Ignored
        );
    }

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
