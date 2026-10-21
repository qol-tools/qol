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
    selected: Option<usize>,
    picked: bool,
    wheeled: Option<usize>,
}

impl PathEdit {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            field: TextField::with_text(text),
            suggestions: Vec::new(),
            selected: None,
            picked: false,
            wheeled: None,
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
        self.selected
            .filter(|&selected| selected < self.suggestions.len())
    }

    pub fn set_suggestions(&mut self, names: Vec<String>) {
        let kept = self
            .selected()
            .filter(|_| self.picked)
            .and_then(|selected| self.suggestions.get(selected))
            .and_then(|selected| names.iter().position(|name| name == selected));
        if names != self.suggestions {
            self.wheeled = None;
        }
        self.suggestions = names;
        let segment = self.segment();
        let auto = if segment.is_empty() || self.suggestions.is_empty() {
            None
        } else {
            let exact = self.suggestions.iter().position(|name| name == segment);
            Some(exact.unwrap_or(0))
        };
        self.selected = kept.or(auto);
    }

    pub fn select_next(&mut self) -> bool {
        let next = self.selected().map_or(0, |selected| selected + 1);
        if next >= self.suggestions.len() {
            return false;
        }
        self.selected = Some(next);
        self.picked = true;
        self.wheeled = None;
        true
    }

    pub fn select_previous(&mut self) -> bool {
        let Some(selected) = self.selected() else {
            return false;
        };
        self.selected = selected.checked_sub(1);
        self.picked = true;
        self.wheeled = None;
        true
    }

    pub fn complete(&mut self) -> bool {
        let Some(name) = self.suggestions.get(self.selected().unwrap_or(0)) else {
            return false;
        };
        let completed = format!("{}{name}{MAIN_SEPARATOR}", self.parent());
        self.field.set_text(completed);
        self.picked = false;
        true
    }

    pub fn pick(&mut self, index: usize) -> bool {
        if index >= self.suggestions.len() {
            return false;
        }
        self.selected = Some(index);
        self.complete()
    }

    pub fn shown(&self, rows: usize) -> Range<usize> {
        let total = self.suggestions.len();
        let first = self
            .wheeled
            .unwrap_or_else(|| (self.selected().unwrap_or(0) + 2).saturating_sub(rows))
            .min(total.saturating_sub(rows));
        first..(first + rows).min(total)
    }

    pub fn wheel(&mut self, rows: isize, shown: usize) -> bool {
        let first = self.shown(shown).start;
        let last = self.suggestions.len().saturating_sub(shown);
        let target = first.saturating_add_signed(rows).min(last);
        self.wheeled = Some(target);
        target != first
    }

    pub fn key(
        &mut self,
        keystroke: &Keystroke,
        paste: impl FnOnce() -> Option<String>,
    ) -> PathKey {
        match keystroke.key.as_str() {
            "enter" => {
                if self.selected().is_some() {
                    self.complete();
                }
                PathKey::Submit
            }
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
                EditKey::Changed => {
                    self.picked = false;
                    PathKey::Changed
                }
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
    fn an_empty_segment_highlights_nothing_and_an_exact_name_wins() {
        assert_eq!(edit("~/", &["Git", "Games"]).selected(), None);
        assert_eq!(edit("~/b", &["B", "b", "beta"]).selected(), Some(1));
    }

    #[test]
    fn typing_drops_the_highlight_at_the_folder_level_and_an_arrow_pick_sticks() {
        let mut edit = edit("~/one/x", &["x"]);
        assert_eq!(edit.selected(), Some(0));
        assert_eq!(edit.key(&key("backspace", None), || None), PathKey::Changed);
        edit.set_suggestions(names(&["x"]));
        assert_eq!(edit.selected(), None);
        assert!(edit.select_next());
        edit.set_suggestions(names(&["w", "x"]));
        assert_eq!(edit.selected(), Some(1));
    }

    #[test]
    fn new_suggestions_keep_the_selected_name_when_it_is_still_there() {
        let mut edit = edit("~/G", &["Git", "Games"]);
        edit.select_next();
        edit.set_suggestions(names(&["Gadgets", "Games", "Git"]));
        assert_eq!(edit.selected(), Some(1));
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
        assert!(edit.select_next());
        assert!(!edit.select_next());
        assert_eq!(edit.selected(), Some(2));
        assert!(edit.select_previous());
        assert_eq!(edit.selected(), Some(1));
        assert!(edit.select_previous());
        assert!(edit.select_previous());
        assert_eq!(edit.selected(), None);
        assert!(!edit.select_previous());
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
    fn picking_a_suggestion_completes_it() {
        let mut edit = edit("~/G", &["Git", "Games"]);
        assert!(edit.pick(1));
        assert_eq!(edit.text(), format!("~/Games{MAIN_SEPARATOR}"));
        assert!(!edit.pick(7));
    }

    #[test]
    fn tab_without_suggestions_is_handled_and_changes_nothing() {
        let mut edit = edit("~/Git/qo", &[]);
        assert_eq!(edit.key(&key("tab", None), || None), PathKey::Handled);
        assert_eq!(edit.text(), "~/Git/qo");
    }

    #[test]
    fn enter_takes_the_highlighted_suggestion_and_escape_cancels() {
        let mut edit = edit("~/Gi", &["Git"]);
        assert_eq!(edit.key(&key("enter", None), || None), PathKey::Submit);
        assert_eq!(edit.text(), format!("~/Git{MAIN_SEPARATOR}"));
        assert_eq!(edit.key(&key("escape", None), || None), PathKey::Cancel);
        assert_eq!(edit.text(), format!("~/Git{MAIN_SEPARATOR}"));
    }

    #[test]
    fn enter_without_a_highlight_submits_the_text_as_typed() {
        let mut bare = edit("~/", &["Git", "Games"]);
        assert_eq!(bare.key(&key("enter", None), || None), PathKey::Submit);
        assert_eq!(bare.text(), "~/");

        let mut unhighlighted = edit("~/Gi", &["Git"]);
        assert_eq!(
            unhighlighted.key(&key("up", None), || None),
            PathKey::Handled
        );
        assert_eq!(unhighlighted.selected(), None);
        assert_eq!(
            unhighlighted.key(&key("enter", None), || None),
            PathKey::Submit
        );
        assert_eq!(unhighlighted.text(), "~/Gi");
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
        for _ in 0..4 {
            edit.select_next();
        }
        assert_eq!(edit.selected(), Some(3));
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

    #[test]
    fn the_wheel_moves_the_shown_rows_and_keeps_the_selection() {
        let all = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let mut edit = edit("~/a", &all);
        assert_eq!(edit.selected(), Some(0));
        assert!(edit.wheel(3, 4));
        assert_eq!(edit.shown(4), 3..7);
        assert!(edit.wheel(10, 4));
        assert_eq!(edit.shown(4), 6..10);
        assert!(!edit.wheel(1, 4));
        assert!(edit.wheel(-2, 4));
        assert_eq!(edit.shown(4), 4..8);
        assert!(edit.wheel(-10, 4));
        assert_eq!(edit.shown(4), 0..4);
        assert!(!edit.wheel(-1, 4));
        assert_eq!(edit.selected(), Some(0));
    }

    #[test]
    fn an_arrow_key_after_the_wheel_brings_the_selection_back_into_view() {
        let all = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let mut edit = edit("~/a", &all);
        edit.wheel(6, 4);
        assert_eq!(edit.shown(4), 6..10);
        edit.select_next();
        assert_eq!(edit.selected(), Some(1));
        assert_eq!(edit.shown(4), 0..4);
    }

    #[test]
    fn new_suggestions_drop_the_wheel_position_and_the_same_ones_keep_it() {
        let all = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let mut edit = edit("~/", &all);
        edit.wheel(5, 4);
        edit.set_suggestions(names(&all));
        assert_eq!(edit.shown(4), 5..9);
        edit.set_suggestions(names(&all[1..]));
        assert_eq!(edit.shown(4), 0..4);
    }

    #[test]
    fn the_wheel_does_nothing_when_every_suggestion_fits() {
        let mut edit = edit("~/", &["a", "b"]);
        assert!(!edit.wheel(3, 4));
        assert_eq!(edit.shown(4), 0..2);
    }
}
