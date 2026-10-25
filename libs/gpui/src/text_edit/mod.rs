//! Editing conventions for hand-rolled text fields: how far a horizontal
//! motion or delete reaches for the held modifiers, and the word-boundary
//! scans behind it.
//!
//! Cursors are char indices into `text`. A word is a run of alphanumerics
//! and underscores; everything else separates words.

mod field;
mod platform;

pub use field::{
    marked, needs_mark, single_line, visible_char_count, CaretStyle, TextField, TextFieldElement,
};

use std::cell::RefCell;

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

pub trait Clipboard {
    fn read(&self) -> Option<String>;
    fn write(&self, text: String);
}

impl Clipboard for gpui::App {
    fn read(&self) -> Option<String> {
        self.read_from_clipboard().and_then(|item| item.text())
    }

    fn write(&self, text: String) {
        self.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }
}

impl<T> Clipboard for gpui::Context<'_, T> {
    fn read(&self) -> Option<String> {
        <gpui::App as Clipboard>::read(self)
    }

    fn write(&self, text: String) {
        <gpui::App as Clipboard>::write(self, text);
    }
}

#[derive(Debug, Default)]
pub struct MemoryClipboard(RefCell<Option<String>>);

impl MemoryClipboard {
    pub fn holding(text: impl Into<String>) -> Self {
        Self(RefCell::new(Some(text.into())))
    }
}

impl Clipboard for MemoryClipboard {
    fn read(&self) -> Option<String> {
        self.0.borrow().clone()
    }

    fn write(&self, text: String) {
        *self.0.borrow_mut() = Some(text);
    }
}

pub fn apply_edit_key(
    field: &mut TextField,
    keystroke: &gpui::Keystroke,
    clipboard: &dyn Clipboard,
) -> EditKey {
    let modifiers = &keystroke.modifiers;
    if modifiers.secondary() {
        match keystroke.key.as_str() {
            "z" if modifiers.shift => return changed_if(field.redo()),
            "z" => return changed_if(field.undo()),
            "y" => return changed_if(field.redo()),
            "c" => {
                if let Some(text) = field.selection_text() {
                    clipboard.write(text);
                }
                field.end_typing();
                return EditKey::Handled;
            }
            _ => {}
        }
    }
    let before = field.snapshot();
    let outcome = edit(field, keystroke, clipboard);
    match outcome {
        EditKey::Changed => field.remember(before, typed(keystroke)),
        EditKey::Handled => field.end_typing(),
        EditKey::Ignored => {}
    }
    outcome
}

fn changed_if(changed: bool) -> EditKey {
    if changed {
        EditKey::Changed
    } else {
        EditKey::Handled
    }
}

fn typed(keystroke: &gpui::Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    !(modifiers.control || modifiers.alt || modifiers.platform)
        && keystroke
            .key_char
            .as_deref()
            .is_some_and(|text| !text.chars().any(|ch| ch.is_control() || ch.is_whitespace()))
}

fn edit(field: &mut TextField, keystroke: &gpui::Keystroke, clipboard: &dyn Clipboard) -> EditKey {
    let key = keystroke.key.as_str();
    let modifiers = &keystroke.modifiers;
    if modifiers.secondary() && key == "v" {
        return match clipboard.read() {
            Some(text) => changed_if(field.paste(&text)),
            None => EditKey::Handled,
        };
    }
    if modifiers.secondary() && key == "x" {
        return match field.cut_selection() {
            Some(text) => {
                clipboard.write(text);
                EditKey::Changed
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
        apply_edit_key, span, word_end_after, word_start_before, Clipboard, EditKey,
        MemoryClipboard, Span, TextField,
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
            apply_edit_key(
                &mut field,
                &key("a", Some("a"), Modifiers::none()),
                &MemoryClipboard::default()
            ),
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
                &MemoryClipboard::default()
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
                &MemoryClipboard::default()
            ),
            EditKey::Handled
        );
    }

    #[test]
    fn a_character_replaces_the_selection_from_shift_left() {
        let mut field = TextField::with_text("a");
        assert_eq!(
            apply_edit_key(
                &mut field,
                &key("left", None, Modifiers::shift()),
                &MemoryClipboard::default()
            ),
            EditKey::Handled
        );
        assert!(field.selected_range().is_some());
        assert_eq!(
            apply_edit_key(
                &mut field,
                &key("b", Some("b"), Modifiers::none()),
                &MemoryClipboard::default()
            ),
            EditKey::Changed
        );
        assert_eq!(field.text(), "b");
    }

    #[test]
    fn alt_held_keys_are_ignored() {
        let mut field = TextField::with_text("a");
        assert_eq!(
            apply_edit_key(
                &mut field,
                &key("x", Some("x"), Modifiers::alt()),
                &MemoryClipboard::default()
            ),
            EditKey::Ignored
        );
        assert_eq!(field.text(), "a");
    }

    #[test]
    fn escape_is_ignored() {
        let mut field = TextField::new();
        assert_eq!(
            apply_edit_key(
                &mut field,
                &key("escape", None, Modifiers::none()),
                &MemoryClipboard::default()
            ),
            EditKey::Ignored
        );
    }

    fn secondary(letter: &str, shift: bool) -> Keystroke {
        let mut modifiers = Modifiers::secondary_key();
        modifiers.shift = shift;
        key(letter, Some(letter), modifiers)
    }

    fn typed(field: &mut TextField, text: &str) {
        for letter in text.chars().map(String::from) {
            apply_edit_key(
                field,
                &key(&letter, Some(&letter), Modifiers::none()),
                &MemoryClipboard::default(),
            );
        }
    }

    #[test]
    fn copy_puts_the_selection_on_the_clipboard_and_keeps_the_text() {
        let clipboard = MemoryClipboard::default();
        let mut field = TextField::with_text("/home/user");
        field.select_all();
        assert_eq!(
            apply_edit_key(&mut field, &secondary("c", false), &clipboard),
            EditKey::Handled
        );
        assert_eq!(clipboard.read().as_deref(), Some("/home/user"));
        assert_eq!(field.text(), "/home/user");
    }

    #[test]
    fn copy_without_a_selection_leaves_the_clipboard_alone() {
        let clipboard = MemoryClipboard::holding("kept");
        let mut field = TextField::with_text("abc");
        apply_edit_key(&mut field, &secondary("c", false), &clipboard);
        assert_eq!(clipboard.read().as_deref(), Some("kept"));
    }

    #[test]
    fn cut_moves_the_selection_to_the_clipboard() {
        let clipboard = MemoryClipboard::default();
        let mut field = TextField::with_text("abc");
        field.select_all();
        assert_eq!(
            apply_edit_key(&mut field, &secondary("x", false), &clipboard),
            EditKey::Changed
        );
        assert_eq!(clipboard.read().as_deref(), Some("abc"));
        assert_eq!(field.text(), "");
    }

    #[test]
    fn paste_replaces_the_selection_with_the_clipboard() {
        let clipboard = MemoryClipboard::holding("xyz");
        let mut field = TextField::with_text("abc");
        field.select_all();
        assert_eq!(
            apply_edit_key(&mut field, &secondary("v", false), &clipboard),
            EditKey::Changed
        );
        assert_eq!(field.text(), "xyz");
    }

    #[test]
    fn undo_takes_back_a_typed_word_at_once_and_redo_brings_it_back() {
        let mut field = TextField::new();
        typed(&mut field, "ab cd");
        let clipboard = MemoryClipboard::default();
        let steps = [
            (secondary("z", false), "ab "),
            (secondary("z", false), "ab"),
            (secondary("z", false), ""),
            (secondary("z", false), ""),
            (secondary("z", true), "ab"),
            (secondary("y", false), "ab "),
        ];
        for (keystroke, expect) in steps {
            apply_edit_key(&mut field, &keystroke, &clipboard);
            assert_eq!(field.text(), expect, "{keystroke:?}");
        }
    }

    #[test]
    fn undo_reverses_a_paste_and_a_cut() {
        let clipboard = MemoryClipboard::holding("xyz");
        let mut field = TextField::with_text("abc");
        apply_edit_key(&mut field, &secondary("v", false), &clipboard);
        field.select_all();
        apply_edit_key(&mut field, &secondary("x", false), &clipboard);
        assert_eq!(field.text(), "");
        apply_edit_key(&mut field, &secondary("z", false), &clipboard);
        assert_eq!(field.text(), "abcxyz");
        apply_edit_key(&mut field, &secondary("z", false), &clipboard);
        assert_eq!(field.text(), "abc");
    }

    #[test]
    fn a_new_edit_drops_the_redo_steps() {
        let clipboard = MemoryClipboard::default();
        let mut field = TextField::new();
        typed(&mut field, "a");
        apply_edit_key(&mut field, &secondary("z", false), &clipboard);
        typed(&mut field, "b");
        assert_eq!(
            apply_edit_key(&mut field, &secondary("z", true), &clipboard),
            EditKey::Handled
        );
        assert_eq!(field.text(), "b");
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
