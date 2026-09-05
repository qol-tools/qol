use super::state::{EdgeHit, LauncherState, NavDirection};
use gpui::Modifiers;
use qol_gpui::text_edit::{self, word_end_after, word_start_before, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEffect {
    Ignore,
    Navigate,
    QueryChanged,
    Launch,
    Dismiss,
    BoostUp,
    BoostDown,
    FlowQueryChanged,
    FlowActivate,
    FlowDetail,
    FlowDetailClose,
    FlowDetailScrollUp,
    FlowDetailScrollDown,
    FlowExit,
    FlowDislike,
}

impl LauncherState {
    pub fn apply_key(
        &mut self,
        key: &str,
        modifiers: &Modifiers,
        result_count: usize,
    ) -> InputEffect {
        if self.flow.is_some() {
            return self.apply_flow_key(key, modifiers, result_count);
        }
        let secondary = modifiers.secondary();
        let control = modifiers.control;
        let shift = modifiers.shift;
        let alt = modifiers.alt;
        let span = text_edit::span(modifiers);
        let boost = (secondary || alt) && span == Span::Char;
        #[cfg(debug_assertions)]
        if matches!(key, "left" | "right") {
            eprintln!(
                "[input] key={key:?} secondary={secondary} control={control} shift={shift} alt={alt}"
            );
        }
        match key {
            "escape" | "esc" => InputEffect::Dismiss,
            "up" if secondary => {
                if self.decrease_fuzziness() {
                    InputEffect::QueryChanged
                } else {
                    InputEffect::Navigate
                }
            }
            "down" if secondary => {
                if self.increase_fuzziness() {
                    InputEffect::QueryChanged
                } else {
                    InputEffect::Navigate
                }
            }
            "tab" => {
                self.cycle_mode(shift);
                InputEffect::QueryChanged
            }
            "left" if boost => InputEffect::BoostDown,
            "right" if boost => InputEffect::BoostUp,
            "left" => {
                self.move_left(shift, span);
                InputEffect::Navigate
            }
            "right" => {
                self.move_right(shift, span);
                InputEffect::Navigate
            }
            "home" => {
                self.move_home(shift);
                InputEffect::Navigate
            }
            "end" => {
                self.move_end(shift);
                InputEffect::Navigate
            }
            "up" if !secondary => {
                if self.is_phantom_reversal(NavDirection::Up) {
                    return InputEffect::Ignore;
                }
                self.move_up();
                InputEffect::Navigate
            }
            "down" if !secondary => {
                if self.is_phantom_reversal(NavDirection::Down) {
                    return InputEffect::Ignore;
                }
                self.move_down(result_count);
                InputEffect::Navigate
            }
            "enter" => InputEffect::Launch,
            "backspace" => {
                if self.backspace(span) {
                    InputEffect::QueryChanged
                } else {
                    InputEffect::Navigate
                }
            }
            "delete" => {
                if self.delete_forward(span) {
                    InputEffect::QueryChanged
                } else {
                    InputEffect::Navigate
                }
            }
            "a" if secondary => {
                self.select_all();
                InputEffect::Navigate
            }
            "space" if !secondary && !control => {
                self.insert_char(' ');
                InputEffect::QueryChanged
            }
            _ => {
                if secondary || control || alt {
                    return InputEffect::Ignore;
                }
                let Some(ch) = key_to_input_char(key, shift) else {
                    return InputEffect::Ignore;
                };
                self.insert_char(ch);
                InputEffect::QueryChanged
            }
        }
    }

    fn apply_flow_key(
        &mut self,
        key: &str,
        modifiers: &Modifiers,
        result_count: usize,
    ) -> InputEffect {
        if self.flow_detail_open() {
            return match key {
                "escape" | "esc" => InputEffect::FlowDetailClose,
                "enter" => InputEffect::FlowActivate,
                "up" => InputEffect::FlowDetailScrollUp,
                "down" => InputEffect::FlowDetailScrollDown,
                _ => InputEffect::Ignore,
            };
        }
        let secondary = modifiers.secondary();
        let control = modifiers.control;
        let shift = modifiers.shift;
        let alt = modifiers.alt;
        let span = text_edit::span(modifiers);
        let boost = (secondary || alt) && span == Span::Char;
        match key {
            "escape" | "esc" => InputEffect::FlowExit,
            "enter" => InputEffect::FlowDetail,
            "up" if !secondary => {
                self.move_up();
                InputEffect::Navigate
            }
            "down" if !secondary => {
                self.move_down(result_count);
                InputEffect::Navigate
            }
            "left" if !boost => {
                self.move_left(shift, span);
                InputEffect::Navigate
            }
            "right" if !boost => {
                self.move_right(shift, span);
                InputEffect::Navigate
            }
            "home" => {
                self.move_home(shift);
                InputEffect::Navigate
            }
            "end" => {
                self.move_end(shift);
                InputEffect::Navigate
            }
            "backspace" => {
                if self.backspace(span) {
                    InputEffect::FlowQueryChanged
                } else {
                    InputEffect::Navigate
                }
            }
            "delete" => {
                if self.delete_forward(span) {
                    InputEffect::FlowQueryChanged
                } else {
                    InputEffect::Navigate
                }
            }
            "x" if alt => InputEffect::FlowDislike,
            "a" if secondary => {
                self.select_all();
                InputEffect::Navigate
            }
            "space" if !secondary && !control => {
                self.insert_char(' ');
                InputEffect::FlowQueryChanged
            }
            _ => {
                if secondary || control || alt {
                    return InputEffect::Ignore;
                }
                let Some(ch) = key_to_input_char(key, shift) else {
                    return InputEffect::Ignore;
                };
                self.insert_char(ch);
                InputEffect::FlowQueryChanged
            }
        }
    }

    fn move_up(&mut self) {
        if self.scroll_list.selected == 0 {
            self.previous_selected = None;
            self.register_nav(NavDirection::Up);
            self.edge_hit = Some(EdgeHit::Top);
            return;
        }

        self.previous_selected = Some(self.scroll_list.selected);
        self.scroll_list.move_up();
        self.register_nav(NavDirection::Up);
        self.edge_hit = None;
    }

    fn move_down(&mut self, result_count: usize) {
        if result_count == 0 {
            return;
        }

        let max = result_count.saturating_sub(1);
        if self.scroll_list.selected >= max {
            self.previous_selected = None;
            self.register_nav(NavDirection::Down);
            self.edge_hit = Some(EdgeHit::Bottom);
            return;
        }

        self.previous_selected = Some(self.scroll_list.selected);
        self.scroll_list.move_down(result_count);
        self.register_nav(NavDirection::Down);
        self.edge_hit = None;
    }

    fn update_selection_anchor(&mut self, selecting: bool, old_cursor: usize) {
        if !selecting {
            self.clear_selection();
            return;
        }
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(old_cursor);
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        let idx = Self::char_to_byte_index(&self.query, self.cursor);
        self.query.insert(idx, ch);
        self.cursor += 1;
        self.clear_selection();
        self.clear_launch_error();
    }

    fn span_start(&self, span: Span) -> usize {
        match span {
            Span::Char => self.cursor.saturating_sub(1),
            Span::Word => word_start_before(&self.query, self.cursor),
            Span::Line => 0,
        }
    }

    fn span_end(&self, span: Span) -> usize {
        match span {
            Span::Char => (self.cursor + 1).min(self.query_len()),
            Span::Word => word_end_after(&self.query, self.cursor),
            Span::Line => self.query_len(),
        }
    }

    fn backspace(&mut self, span: Span) -> bool {
        if self.delete_selection() {
            return true;
        }
        self.delete_chars(self.span_start(span), self.cursor)
    }

    fn delete_forward(&mut self, span: Span) -> bool {
        if self.delete_selection() {
            return true;
        }
        self.delete_chars(self.cursor, self.span_end(span))
    }

    fn delete_chars(&mut self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }
        let start_b = Self::char_to_byte_index(&self.query, start);
        let end_b = Self::char_to_byte_index(&self.query, end);
        self.query.replace_range(start_b..end_b, "");
        self.cursor = start;
        self.clear_launch_error();
        true
    }

    fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.query_len();
    }

    fn move_left(&mut self, selecting: bool, span: Span) {
        let old = self.cursor;
        self.cursor = self.span_start(span);
        self.update_selection_anchor(selecting, old);
    }

    fn move_right(&mut self, selecting: bool, span: Span) {
        let old = self.cursor;
        self.cursor = self.span_end(span);
        self.update_selection_anchor(selecting, old);
    }

    fn move_home(&mut self, selecting: bool) {
        let old = self.cursor;
        self.cursor = 0;
        self.update_selection_anchor(selecting, old);
    }

    fn move_end(&mut self, selecting: bool) {
        let old = self.cursor;
        self.cursor = self.query_len();
        self.update_selection_anchor(selecting, old);
    }
}

pub fn key_to_input_char(key: &str, shift: bool) -> Option<char> {
    if key.chars().count() != 1 {
        return None;
    }
    let ch = key
        .chars()
        .next()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')?;
    Some(if shift { ch.to_ascii_uppercase() } else { ch })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::search::{Fuzziness, SearchMode};
    use crate::flow::FlowEntry;

    fn mods(secondary: bool, control: bool, shift: bool, alt: bool) -> Modifiers {
        let mut modifiers = if secondary {
            Modifiers::secondary_key()
        } else {
            Modifiers::none()
        };
        modifiers.control |= control;
        modifiers.shift = shift;
        modifiers.alt = alt;
        modifiers
    }

    fn word() -> Modifiers {
        [Modifiers::control(), Modifiers::alt(), Modifiers::command()]
            .into_iter()
            .find(|modifiers| text_edit::span(modifiers) == Span::Word)
            .expect("every platform maps a modifier to word motion")
    }

    fn typed(query: &str) -> LauncherState {
        let mut state = LauncherState::new();
        for ch in query.chars() {
            let key = if ch == ' ' {
                "space".to_string()
            } else {
                ch.to_string()
            };
            state.apply_key(&key, &mods(false, false, false, false), 0);
        }
        state
    }

    #[test]
    fn word_backspace_deletes_the_word_and_trailing_separators() {
        for (query, cursor, expect_query, expect_cursor) in [
            ("qol memory", 10, "qol ", 4),
            ("qol memory ", 11, "qol ", 4),
            ("qol-shot", 8, "qol-", 4),
            ("qol memory", 5, "qol emory", 4),
            ("qol memory", 0, "qol memory", 0),
        ] {
            let mut state = typed(query);
            state.cursor = cursor;
            let effect = state.apply_key("backspace", &word(), 0);
            assert_eq!(state.query, expect_query, "{query:?} at {cursor}");
            assert_eq!(state.cursor, expect_cursor, "{query:?} at {cursor}");
            let changed = query != expect_query;
            assert_eq!(effect == InputEffect::QueryChanged, changed);
        }
    }

    #[test]
    fn word_delete_removes_the_next_word() {
        for (query, cursor, expect_query) in [
            ("qol memory", 0, " memory"),
            ("qol memory", 3, "qol"),
            ("qol  memory", 3, "qol"),
            ("qol memory", 10, "qol memory"),
        ] {
            let mut state = typed(query);
            state.cursor = cursor;
            state.apply_key("delete", &word(), 0);
            assert_eq!(state.query, expect_query, "{query:?} at {cursor}");
            assert_eq!(state.cursor, cursor);
        }
    }

    #[test]
    fn word_backspace_deletes_a_word_in_flow_mode() {
        let mut state = LauncherState::new();
        state.enter_flow(flow_entry("qol memory"));
        state.apply_key("m", &mods(false, false, false, false), 3);
        state.apply_key("e", &mods(false, false, false, false), 3);
        assert_eq!(
            state.apply_key("backspace", &word(), 3),
            InputEffect::FlowQueryChanged
        );
        assert!(state.query.is_empty());
    }

    #[test]
    fn query_and_mode_changes_clear_launch_errors() {
        let mut state = LauncherState::new();
        state.set_launch_error("failed".to_string());

        assert_eq!(
            state.apply_key("x", &mods(false, false, false, false), 0),
            InputEffect::QueryChanged
        );
        assert!(state.launch_error.is_none());

        state.set_launch_error("failed again".to_string());
        assert_eq!(
            state.apply_key("tab", &mods(false, false, false, false), 0),
            InputEffect::QueryChanged
        );
        assert!(state.launch_error.is_none());
    }

    #[test]
    fn word_arrows_jump_words_and_shift_selects() {
        let mut state = typed("qol memory");
        assert_eq!(state.apply_key("left", &word(), 0), InputEffect::Navigate);
        assert_eq!(state.cursor, 4);
        state.apply_key("left", &word(), 0);
        assert_eq!(state.cursor, 0);
        state.apply_key("right", &word(), 0);
        assert_eq!(state.cursor, 3);
        let select_word = Modifiers {
            shift: true,
            ..word()
        };
        state.apply_key("right", &select_word, 0);
        assert_eq!(state.cursor, 10);
        assert_eq!(state.selected_range(), Some((3, 10)));
    }

    #[test]
    fn word_arrows_jump_words_in_flow_mode() {
        let mut state = LauncherState::new();
        state.enter_flow(flow_entry("qol memory"));
        for key in ["a", "b", "space", "c"] {
            state.apply_key(key, &mods(false, false, false, false), 3);
        }
        assert_eq!(state.apply_key("left", &word(), 3), InputEffect::Navigate);
        assert_eq!(state.cursor, 3);
        state.apply_key("left", &word(), 3);
        assert_eq!(state.cursor, 0);
    }

    fn boost_modifiers() -> Vec<Modifiers> {
        [Modifiers::secondary_key(), Modifiers::alt()]
            .into_iter()
            .filter(|modifiers| text_edit::span(modifiers) == Span::Char)
            .collect()
    }

    #[test]
    fn arrows_boost_under_modifiers_the_text_field_does_not_claim() {
        for modifiers in boost_modifiers() {
            let mut state = LauncherState::new();
            assert_eq!(
                state.apply_key("right", &modifiers, 0),
                InputEffect::BoostUp,
                "{modifiers:?}"
            );
        }
    }

    #[test]
    fn fast_direction_reversal_is_ignored_as_phantom() {
        let mut state = LauncherState::new();

        assert_eq!(
            state.apply_key("down", &mods(false, false, false, false), 7),
            InputEffect::Navigate
        );
        assert_eq!(state.scroll_list.selected, 1);

        assert_eq!(
            state.apply_key("up", &mods(false, false, false, false), 7),
            InputEffect::Ignore,
            "an up arriving within the phantom window must not move"
        );
        assert_eq!(
            state.scroll_list.selected, 1,
            "selection holds against the phantom up"
        );
    }

    #[test]
    fn deliberate_reversal_after_delay_still_navigates() {
        use std::time::{Duration, Instant};

        let mut state = LauncherState::new();
        state.apply_key("down", &mods(false, false, false, false), 7);
        assert_eq!(state.scroll_list.selected, 1);

        state.last_nav_at = Some(Instant::now() - Duration::from_millis(250));
        assert_eq!(
            state.apply_key("up", &mods(false, false, false, false), 7),
            InputEffect::Navigate,
            "a human-paced reversal is real navigation"
        );
        assert_eq!(state.scroll_list.selected, 0);
    }

    #[test]
    fn repeated_same_direction_is_never_treated_as_phantom() {
        let mut state = LauncherState::new();
        let cases = [1usize, 2, 3, 4];
        for expected in cases {
            assert_eq!(
                state.apply_key("down", &mods(false, false, false, false), 7),
                InputEffect::Navigate,
                "down #{expected} must navigate"
            );
            assert_eq!(
                state.scroll_list.selected, expected,
                "down #{expected} advances"
            );
        }
    }

    #[test]
    fn down_uses_freshly_filtered_count_not_stale_store() {
        use crate::discovery::entry_store::EntryStore;
        use crate::discovery::search::SearchMode;
        use crate::discovery::{AppEntry, FileEntry};
        use std::sync::Arc;

        let app = |name: &str| AppEntry {
            name: name.to_string(),
            exec: vec!["x".to_string()],
            path: std::path::PathBuf::from("/a/b/c"),
        };
        let apps = Arc::new(vec![
            app("Foobar"),
            app("Foobar Nightly"),
            app("Foobar Studio"),
        ]);
        let files: Arc<Vec<FileEntry>> = Arc::new(Vec::new());
        let flows: Arc<Vec<crate::flow::FlowEntry>> = Arc::new(Vec::new());
        let mut store = EntryStore::new(apps, files, flows);

        let mut state = LauncherState::new();
        state.mode = SearchMode::Apps;
        state.query = "foo".to_string();
        state.cursor = 3;

        assert_eq!(
            store.result_count(),
            0,
            "store stays unfiltered until ensure_filtered runs for the current query"
        );

        store.ensure_filtered(&state.query, state.mode, state.fuzziness);
        let result_count = store.result_count();
        assert!(
            result_count >= 2,
            "query should match multiple apps, got {result_count}"
        );

        let effect = state.apply_key("down", &mods(false, false, false, false), result_count);
        assert_eq!(effect, InputEffect::Navigate);
        assert_eq!(
            state.scroll_list.selected, 1,
            "down advances once navigation reads the live count, not the stale 0"
        );
    }

    #[test]
    fn secondary_shortcuts_still_work() {
        let mut state = LauncherState::new();
        state.query = "abc".to_string();
        state.cursor = 1;

        assert_eq!(
            state.apply_key("a", &mods(true, false, false, false), 0),
            InputEffect::Navigate
        );
        assert_eq!(state.selected_range(), Some((0, 3)));

        assert_eq!(state.fuzziness, Fuzziness::Balanced);
        assert_eq!(
            state.apply_key("down", &mods(true, false, false, false), 0),
            InputEffect::QueryChanged
        );
        assert_eq!(state.fuzziness, Fuzziness::Loose);
    }

    fn flow_entry(title: &str) -> FlowEntry {
        FlowEntry {
            plugin_id: "qol-memory".to_string(),
            title: title.to_string(),
            prompt: "Ask memory".to_string(),
            query: "rows".to_string(),
            row_actions: Vec::new(),
        }
    }

    #[test]
    fn flow_alt_x_reports_flow_dislike_and_bare_x_still_types() {
        let mut state = LauncherState::new();
        state.enter_flow(flow_entry("qol memory"));

        assert_eq!(
            state.apply_key("x", &mods(false, false, false, true), 3),
            InputEffect::FlowDislike
        );

        assert_eq!(
            state.apply_key("x", &mods(false, false, false, false), 3),
            InputEffect::FlowQueryChanged
        );
        assert_eq!(state.query, "x");
    }

    #[test]
    fn flow_escape_exits_and_enter_opens_detail() {
        let mut state = LauncherState::new();
        state.enter_flow(flow_entry("qol memory"));

        assert_eq!(
            state.apply_key("escape", &mods(false, false, false, false), 3),
            InputEffect::FlowExit
        );

        state.enter_flow(flow_entry("qol memory"));
        assert_eq!(
            state.apply_key("enter", &mods(false, false, false, false), 3),
            InputEffect::FlowDetail
        );

        state.exit_flow();
        assert!(state.flow.is_none());
    }

    #[test]
    fn open_detail_routes_keys_and_shields_the_query() {
        let mut state = LauncherState::new();
        state.enter_flow(flow_entry("qol memory"));
        state.query = "seed".to_string();
        state.cursor = 4;
        assert!(state.open_flow_detail());

        assert_eq!(
            state.apply_key("escape", &mods(false, false, false, false), 3),
            InputEffect::FlowDetailClose
        );
        assert_eq!(
            state.apply_key("enter", &mods(false, false, false, false), 3),
            InputEffect::FlowActivate
        );
        assert_eq!(
            state.apply_key("x", &mods(false, false, false, false), 3),
            InputEffect::Ignore
        );
        assert_eq!(state.query, "seed");
        assert_eq!(state.cursor, 4);

        assert_eq!(
            state.apply_key("down", &mods(false, false, false, false), 3),
            InputEffect::FlowDetailScrollDown
        );
        assert_eq!(
            state.apply_key("up", &mods(false, false, false, false), 3),
            InputEffect::FlowDetailScrollUp
        );
        assert_eq!(
            state.scroll_list.selected, 0,
            "arrows scroll the open memory, never move between memories"
        );
    }

    #[test]
    fn flow_tab_is_ignored() {
        let mut state = LauncherState::new();
        state.enter_flow(flow_entry("qol memory"));

        assert_eq!(
            state.apply_key("tab", &mods(false, false, false, false), 3),
            InputEffect::Ignore
        );
        for modifiers in boost_modifiers() {
            assert_eq!(
                state.apply_key("right", &modifiers, 3),
                InputEffect::Ignore,
                "{modifiers:?}"
            );
        }
        assert_eq!(state.mode, SearchMode::Apps);
        assert_eq!(state.fuzziness, Fuzziness::Balanced);
    }

    #[test]
    fn flow_typing_reports_flow_query_changed() {
        let mut state = LauncherState::new();
        state.enter_flow(flow_entry("qol memory"));

        assert_eq!(
            state.apply_key("m", &mods(false, false, false, false), 3),
            InputEffect::FlowQueryChanged
        );
        assert_eq!(state.query, "m");

        assert_eq!(
            state.apply_key("space", &mods(false, false, false, false), 3),
            InputEffect::FlowQueryChanged
        );
        assert_eq!(state.query, "m ");

        assert_eq!(
            state.apply_key("backspace", &mods(false, false, false, false), 3),
            InputEffect::FlowQueryChanged
        );
        assert_eq!(state.query, "m");

        assert_eq!(
            state.apply_key("backspace", &mods(false, false, false, false), 3),
            InputEffect::FlowQueryChanged
        );
        assert!(state.query.is_empty());

        assert_eq!(
            state.apply_key("backspace", &mods(false, false, false, false), 3),
            InputEffect::Navigate
        );
        assert!(state.query.is_empty());

        assert_eq!(
            state.apply_key("up", &mods(false, false, false, false), 3),
            InputEffect::Navigate
        );
        assert_eq!(state.scroll_list.selected, 0);
    }
}
