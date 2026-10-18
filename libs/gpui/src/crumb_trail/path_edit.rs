use std::ops::Range;
use std::path::{is_separator, MAIN_SEPARATOR};

use gpui::Keystroke;

use crate::text_edit::{apply_edit_key, EditKey, TextField};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKey {
    Changed,
    Handled,
    Submit,
    Cancel,
    Ignored,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathEdit {
    field: TextField,
    suggestions: Vec<String>,
    selected: usize,
}

impl PathEdit {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            field: TextField::with_text(text),
            suggestions: Vec::new(),
            selected: 0,
        }
    }

    pub fn field(&self) -> &TextField {
        &self.field
    }

    pub fn text(&self) -> &str {
        self.field.text()
    }

    pub fn parent(&self) -> &str {
        &self.text()[..segment_start(self.text())]
    }

    pub fn segment(&self) -> &str {
        &self.text()[segment_start(self.text())..]
    }

    pub fn suggestions(&self) -> &[String] {
        &self.suggestions
    }

    pub fn selected(&self) -> Option<usize> {
        (self.selected < self.suggestions.len()).then_some(self.selected)
    }

    pub fn set_suggestions(&mut self, names: Vec<String>) {
        self.suggestions = names;
        self.selected = 0;
    }

    pub fn select_next(&mut self) -> bool {
        if self.selected + 1 >= self.suggestions.len() {
            return false;
        }
        self.selected += 1;
        true
    }

    pub fn select_previous(&mut self) -> bool {
        if self.selected == 0 {
            return false;
        }
        self.selected -= 1;
        true
    }

    pub fn complete(&mut self) -> bool {
        let Some(name) = self.suggestions.get(self.selected) else {
            return false;
        };
        let completed = format!("{}{name}{MAIN_SEPARATOR}", self.parent());
        self.field.set_text(completed);
        true
    }

    pub fn shown(&self, rows: usize) -> Range<usize> {
        let total = self.suggestions.len();
        let first = (self.selected + 2)
            .saturating_sub(rows)
            .min(total.saturating_sub(rows));
        first..(first + rows).min(total)
    }

    pub fn key(
        &mut self,
        keystroke: &Keystroke,
        paste: impl FnOnce() -> Option<String>,
    ) -> PathKey {
        match keystroke.key.as_str() {
            "enter" => PathKey::Submit,
            "escape" => PathKey::Cancel,
            "tab" if self.complete() => PathKey::Changed,
            "tab" => PathKey::Handled,
            "up" => {
                self.select_previous();
                PathKey::Handled
            }
            "down" => {
                self.select_next();
                PathKey::Handled
            }
            _ => match apply_edit_key(&mut self.field, keystroke, paste) {
                EditKey::Changed => PathKey::Changed,
                EditKey::Handled => PathKey::Handled,
                EditKey::Ignored => PathKey::Ignored,
            },
        }
    }
}

fn segment_start(text: &str) -> usize {
    text.char_indices()
        .rfind(|(_, ch)| is_separator(*ch))
        .map_or(0, |(index, ch)| index + ch.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::{PathEdit, PathKey};
    use gpui::{Keystroke, Modifiers};
    use std::path::MAIN_SEPARATOR;

    fn key(key: &str, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers: Modifiers::none(),
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn edit(text: &str, suggestions: &[&str]) -> PathEdit {
        let mut edit = PathEdit::new(text);
        edit.set_suggestions(names(suggestions));
        edit
    }

    #[test]
    fn the_last_segment_follows_the_last_separator() {
        for (text, parent, segment) in [
            ("~/Git/qo", "~/Git/", "qo"),
            ("~/Git/", "~/Git/", ""),
            ("Git", "", "Git"),
            ("", "", ""),
            ("/", "/", ""),
            ("~/héllo/wö", "~/héllo/", "wö"),
        ] {
            let edit = PathEdit::new(text);
            assert_eq!(edit.parent(), parent, "{text:?}");
            assert_eq!(edit.segment(), segment, "{text:?}");
        }
    }

    #[test]
    fn the_top_suggestion_is_selected_first() {
        assert_eq!(edit("~/G", &["Git", "Games"]).selected(), Some(0));
        assert_eq!(edit("~/G", &[]).selected(), None);
    }

    #[test]
    fn new_suggestions_reset_the_selection_to_the_top() {
        let mut edit = edit("~/G", &["Git", "Games"]);
        edit.select_next();
        edit.set_suggestions(names(&["Go"]));
        assert_eq!(edit.selected(), Some(0));
    }

    #[test]
    fn up_and_down_stop_at_the_edges() {
        let mut edit = edit("~/", &["a", "b", "c"]);
        assert!(!edit.select_previous());
        assert!(edit.select_next());
        assert!(edit.select_next());
        assert!(!edit.select_next());
        assert_eq!(edit.selected(), Some(2));
        assert!(edit.select_previous());
        assert_eq!(edit.selected(), Some(1));
    }

    #[test]
    fn tab_completes_the_selected_suggestion_after_the_parent() {
        let mut edit = edit("~/Git/qo", &["qol-monorepo", "qol-skills"]);
        assert_eq!(edit.key(&key("down", None), || None), PathKey::Handled);
        assert_eq!(edit.key(&key("tab", None), || None), PathKey::Changed);
        assert_eq!(edit.text(), format!("~/Git/qol-skills{MAIN_SEPARATOR}"));
        assert_eq!(edit.field().cursor(), edit.text().chars().count());
    }

    #[test]
    fn tab_without_suggestions_is_handled_and_changes_nothing() {
        let mut edit = edit("~/Git/qo", &[]);
        assert_eq!(edit.key(&key("tab", None), || None), PathKey::Handled);
        assert_eq!(edit.text(), "~/Git/qo");
    }

    #[test]
    fn enter_submits_and_escape_cancels_without_editing() {
        let mut edit = edit("~/Git", &["Git"]);
        assert_eq!(edit.key(&key("enter", None), || None), PathKey::Submit);
        assert_eq!(edit.key(&key("escape", None), || None), PathKey::Cancel);
        assert_eq!(edit.text(), "~/Git");
    }

    #[test]
    fn typing_and_backspace_edit_the_text() {
        let mut edit = edit("~/Gi", &["Git"]);
        assert_eq!(edit.key(&key("t", Some("t")), || None), PathKey::Changed);
        assert_eq!(edit.text(), "~/Git");
        assert_eq!(edit.key(&key("backspace", None), || None), PathKey::Changed);
        assert_eq!(edit.text(), "~/Gi");
        assert_eq!(edit.key(&key("left", None), || None), PathKey::Handled);
        assert_eq!(edit.key(&key("f1", None), || None), PathKey::Ignored);
    }

    #[test]
    fn the_shown_rows_keep_the_selection_in_view() {
        let all = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let mut edit = edit("~/", &all);
        assert_eq!(edit.shown(4), 0..4);
        for _ in 0..3 {
            edit.select_next();
        }
        assert_eq!(edit.shown(4), 1..5);
        for _ in 0..6 {
            edit.select_next();
        }
        assert_eq!(edit.selected(), Some(9));
        assert_eq!(edit.shown(4), 6..10);
        assert_eq!(PathEdit::new("~/").shown(4), 0..0);
        let mut few = PathEdit::new("~/");
        few.set_suggestions(names(&["a", "b"]));
        assert_eq!(few.shown(4), 0..2);
    }
}
