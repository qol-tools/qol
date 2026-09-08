use std::ops::Range;

use gpui::prelude::*;
use gpui::{div, px, Div, HighlightStyle, Hsla, SharedString, StyledText};

use super::{word_end_after, word_start_before, Span};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextField {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretStyle {
    pub color: Hsla,
    pub width: f32,
    pub height: f32,
    pub top: f32,
    pub radius: f32,
}

pub struct TextFieldElement<'a> {
    field: &'a TextField,
    visible: usize,
    advance: f32,
    selection: Option<(Hsla, Option<Hsla>)>,
    caret: Option<CaretStyle>,
    placeholder: Option<(SharedString, Hsla)>,
}

pub fn visible_char_count(available_px: f32, advance_px: f32) -> usize {
    let fits = if advance_px > 0.0 {
        (available_px / advance_px).floor() as usize
    } else {
        0
    };
    fits.max(8)
}

fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn caret_element(caret: CaretStyle, x: f32) -> Div {
    div()
        .absolute()
        .left(px((x - caret.width / 2.0).max(0.0)))
        .top(px(caret.top))
        .w(px(caret.width))
        .h(px(caret.height))
        .rounded(px(caret.radius))
        .bg(caret.color)
}

impl TextField {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            anchor: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
        self.anchor = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.char_len());
    }

    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    pub fn selected_range(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            None
        } else {
            Some((anchor.min(self.cursor), anchor.max(self.cursor)))
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        let start_byte = char_to_byte(&self.text, start);
        let end_byte = char_to_byte(&self.text, end);
        Some(self.text[start_byte..end_byte].to_string())
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.char_len();
    }

    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        let index = char_to_byte(&self.text, self.cursor);
        self.text.insert(index, ch);
        self.cursor += 1;
        self.anchor = None;
    }

    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.delete_selection();
        let index = char_to_byte(&self.text, self.cursor);
        self.text.insert_str(index, text);
        self.cursor += text.chars().count();
        self.anchor = None;
    }

    pub fn paste(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        self.insert_str(text);
        true
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selected_range() else {
            return false;
        };
        let start_byte = char_to_byte(&self.text, start);
        let end_byte = char_to_byte(&self.text, end);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = start;
        self.anchor = None;
        true
    }

    pub fn cut_selection(&mut self) -> Option<String> {
        let selected = self.selection_text()?;
        self.delete_selection();
        Some(selected)
    }

    pub fn backspace(&mut self, span: Span) -> bool {
        if self.delete_selection() {
            return true;
        }
        let start = self.span_start(span);
        self.delete_chars(start, self.cursor)
    }

    pub fn delete_forward(&mut self, span: Span) -> bool {
        if self.delete_selection() {
            return true;
        }
        let end = self.span_end(span);
        self.delete_chars(self.cursor, end)
    }

    pub fn move_left(&mut self, selecting: bool, span: Span) {
        let previous = self.cursor;
        self.cursor = self.span_start(span);
        self.update_selection_anchor(selecting, previous);
    }

    pub fn move_right(&mut self, selecting: bool, span: Span) {
        let previous = self.cursor;
        self.cursor = self.span_end(span);
        self.update_selection_anchor(selecting, previous);
    }

    pub fn move_home(&mut self, selecting: bool) {
        let previous = self.cursor;
        self.cursor = 0;
        self.update_selection_anchor(selecting, previous);
    }

    pub fn move_end(&mut self, selecting: bool) {
        let previous = self.cursor;
        self.cursor = self.char_len();
        self.update_selection_anchor(selecting, previous);
    }

    pub fn visible_window(&self, visible: usize) -> (usize, usize) {
        let char_count = self.char_len();
        let view_start = if char_count <= visible {
            0
        } else {
            self.cursor
                .saturating_sub(visible.saturating_sub(2))
                .min(char_count.saturating_sub(visible))
        };
        let view_end = (view_start + visible).min(char_count);
        (view_start, view_end)
    }

    pub fn cursor_offset(&self, visible: usize) -> usize {
        let (view_start, view_end) = self.visible_window(visible);
        self.cursor
            .saturating_sub(view_start)
            .min(view_end - view_start)
    }

    pub fn cursor_x(&self, visible: usize, advance: f32) -> f32 {
        self.cursor_offset(visible) as f32 * advance
    }

    fn span_start(&self, span: Span) -> usize {
        match span {
            Span::Char => self.cursor.saturating_sub(1),
            Span::Word => word_start_before(&self.text, self.cursor),
            Span::Line => 0,
        }
    }

    fn span_end(&self, span: Span) -> usize {
        match span {
            Span::Char => (self.cursor + 1).min(self.char_len()),
            Span::Word => word_end_after(&self.text, self.cursor),
            Span::Line => self.char_len(),
        }
    }

    fn delete_chars(&mut self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }
        let start_byte = char_to_byte(&self.text, start);
        let end_byte = char_to_byte(&self.text, end);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = start;
        self.anchor = None;
        true
    }

    fn update_selection_anchor(&mut self, selecting: bool, previous: usize) {
        if !selecting {
            self.anchor = None;
            return;
        }
        if self.anchor.is_none() {
            self.anchor = Some(previous);
        }
    }
}

impl<'a> TextFieldElement<'a> {
    pub fn new(field: &'a TextField, visible: usize, advance: f32) -> Self {
        Self {
            field,
            visible,
            advance,
            selection: None,
            caret: None,
            placeholder: None,
        }
    }

    pub fn selection(mut self, background: Hsla, foreground: Option<Hsla>) -> Self {
        self.selection = Some((background, foreground));
        self
    }

    pub fn caret(mut self, caret: CaretStyle) -> Self {
        self.caret = Some(caret);
        self
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>, color: Hsla) -> Self {
        self.placeholder = Some((text.into(), color));
        self
    }

    pub fn render(self) -> Div {
        let caret = self.caret.filter(|_| self.field.selected_range().is_none());
        if self.field.is_empty() {
            let mut row = div().relative();
            if let Some((placeholder, color)) = self.placeholder {
                row = row.text_color(color).child(placeholder);
            }
            if let Some(caret) = caret {
                row = row.child(caret_element(caret, 0.0));
            }
            return row;
        }

        let text = self.field.text();
        let (view_start, view_end) = self.field.visible_window(self.visible);
        let start_byte = char_to_byte(text, view_start);
        let end_byte = char_to_byte(text, view_end);
        let windowed = &text[start_byte..end_byte];

        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        if let (Some((background, foreground)), Some((selection_start, selection_end))) =
            (self.selection, self.field.selected_range())
        {
            let width = view_end - view_start;
            let adjusted_start = selection_start.saturating_sub(view_start).min(width);
            let adjusted_end = selection_end.saturating_sub(view_start).min(width);
            let start = char_to_byte(windowed, adjusted_start);
            let end = char_to_byte(windowed, adjusted_end);
            if start < end {
                highlights.push((
                    start..end,
                    HighlightStyle {
                        color: foreground,
                        background_color: Some(background),
                        ..HighlightStyle::default()
                    },
                ));
            }
        }

        let styled =
            StyledText::new(SharedString::from(windowed.to_owned())).with_highlights(highlights);
        let mut row = div().relative().child(styled);
        if let Some(caret) = caret {
            let x = self.field.cursor_offset(self.visible) as f32 * self.advance;
            row = row.child(caret_element(caret, x));
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::{visible_char_count, TextField};
    use crate::text_edit::Span;

    fn field(text: &str, cursor: usize) -> TextField {
        let mut field = TextField::with_text(text);
        field.set_cursor(cursor);
        field
    }

    #[test]
    fn with_text_places_the_cursor_at_the_end() {
        let field = TextField::with_text("qol memory");
        assert_eq!(field.text(), "qol memory");
        assert_eq!(field.cursor(), 10);
        assert_eq!(field.anchor(), None);
        assert_eq!(field.selected_range(), None);
    }

    #[test]
    fn insert_replaces_the_selection_and_collapses_it() {
        let mut field = field("qol memory", 10);
        field.select_all();
        assert_eq!(field.selected_range(), Some((0, 10)));
        field.insert_char('x');
        assert_eq!(field.text(), "x");
        assert_eq!(field.cursor(), 1);
        assert_eq!(field.selected_range(), None);
    }

    #[test]
    fn insert_str_lands_at_the_cursor() {
        let mut field = field("qol", 1);
        field.insert_str("xy");
        assert_eq!(field.text(), "qxyol");
        assert_eq!(field.cursor(), 3);
    }

    #[test]
    fn paste_rejects_empty_and_replaces_the_selection() {
        let mut field = field("qol memory", 10);
        assert!(!field.paste(""));
        field.select_all();
        assert!(field.paste("shot"));
        assert_eq!(field.text(), "shot");
        assert_eq!(field.cursor(), 4);
    }

    #[test]
    fn backspace_reaches_char_and_word() {
        for (text, cursor, span, expect_text, expect_cursor, changed) in [
            ("qol memory", 10, Span::Char, "qol memor", 9, true),
            ("qol memory", 10, Span::Word, "qol ", 4, true),
            ("qol memory", 0, Span::Char, "qol memory", 0, false),
            ("qol memory", 0, Span::Line, "qol memory", 0, false),
        ] {
            let mut field = field(text, cursor);
            assert_eq!(
                field.backspace(span),
                changed,
                "{text:?} at {cursor} {span:?}"
            );
            assert_eq!(field.text(), expect_text, "{text:?} at {cursor} {span:?}");
            assert_eq!(
                field.cursor(),
                expect_cursor,
                "{text:?} at {cursor} {span:?}"
            );
        }
    }

    #[test]
    fn backspace_removes_the_selection_first() {
        let mut field = field("qol memory", 10);
        field.select_all();
        assert!(field.backspace(Span::Char));
        assert_eq!(field.text(), "");
        assert_eq!(field.cursor(), 0);
    }

    #[test]
    fn delete_forward_reaches_char_and_word() {
        for (text, cursor, span, expect_text, expect_cursor) in [
            ("qol", 1, Span::Char, "ql", 1),
            ("qol memory", 0, Span::Word, " memory", 0),
            ("qol memory", 10, Span::Char, "qol memory", 10),
            ("qol memory", 10, Span::Line, "qol memory", 10),
        ] {
            let mut field = field(text, cursor);
            field.delete_forward(span);
            assert_eq!(field.text(), expect_text, "{text:?} at {cursor} {span:?}");
            assert_eq!(
                field.cursor(),
                expect_cursor,
                "{text:?} at {cursor} {span:?}"
            );
        }
    }

    #[test]
    fn motion_collapses_or_extends_the_selection() {
        let mut field = field("qol memory", 10);
        field.move_home(true);
        assert_eq!(field.cursor(), 0);
        assert_eq!(field.selected_range(), Some((0, 10)));
        field.move_right(true, Span::Char);
        assert_eq!(field.cursor(), 1);
        assert_eq!(field.selected_range(), Some((1, 10)));
        field.move_end(false);
        assert_eq!(field.cursor(), 10);
        assert_eq!(field.selected_range(), None);
    }

    #[test]
    fn word_motion_matches_the_launcher() {
        let mut field = field("qol memory", 10);
        field.move_left(false, Span::Word);
        assert_eq!(field.cursor(), 4);
        field.move_left(false, Span::Word);
        assert_eq!(field.cursor(), 0);
        field.move_right(false, Span::Word);
        assert_eq!(field.cursor(), 3);
        field.move_right(false, Span::Word);
        assert_eq!(field.cursor(), 10);
    }

    #[test]
    fn delete_after_collapsed_selection_leaves_no_phantom_range() {
        let mut field = field("qol", 1);
        field.move_left(true, Span::Char);
        field.move_right(true, Span::Char);
        assert_eq!(field.selected_range(), None);
        assert!(field.backspace(Span::Char));
        assert_eq!(field.text(), "ol");
        assert_eq!(field.cursor(), 0);
        assert_eq!(field.selected_range(), None);
    }

    #[test]
    fn cut_selection_returns_the_removed_text() {
        let mut field = field("qol memory", 10);
        field.select_all();
        assert_eq!(field.cut_selection(), Some("qol memory".to_string()));
        assert_eq!(field.text(), "");
        assert_eq!(field.cursor(), 0);
        assert_eq!(field.cut_selection(), None);
    }

    #[test]
    fn visible_window_matches_the_launcher_policy() {
        let cases = [
            (10usize, 3usize, 25usize, (0, 10)),
            (25, 25, 25, (0, 25)),
            (60, 5, 25, (0, 25)),
            (60, 30, 25, (7, 32)),
            (60, 60, 25, (35, 60)),
        ];
        for (len, cursor, visible, expect) in cases {
            let text = "a".repeat(len);
            let mut field = TextField::with_text(text);
            field.set_cursor(cursor);
            assert_eq!(
                field.visible_window(visible),
                expect,
                "len={len} cursor={cursor} visible={visible}"
            );
        }
    }

    #[test]
    fn cursor_x_tracks_the_window() {
        let mut field = TextField::with_text("a".repeat(60));
        field.set_cursor(30);
        assert_eq!(field.cursor_offset(25), 23);
        assert_eq!(field.cursor_x(25, 10.0), 230.0);
        assert_eq!(TextField::new().cursor_x(25, 10.0), 0.0);
    }

    #[test]
    fn visible_char_count_matches_the_launcher_policy() {
        assert_eq!(visible_char_count(300.0, 10.0), 30);
        assert_eq!(visible_char_count(309.0, 10.0), 30);
        assert_eq!(visible_char_count(79.0, 10.0), 8);
        assert_eq!(visible_char_count(0.0, 10.0), 8);
        assert_eq!(visible_char_count(300.0, 0.0), 8);
    }
}
