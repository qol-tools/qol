use super::state::{EdgeHit, LauncherState, NavDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEffect {
    Ignore,
    Navigate,
    QueryChanged,
    Launch,
    Dismiss,
    BoostUp,
    BoostDown,
}

impl LauncherState {
    pub fn apply_key(
        &mut self,
        key: &str,
        secondary: bool,
        control: bool,
        shift: bool,
        alt: bool,
        result_count: usize,
    ) -> InputEffect {
        let boost = secondary || control || alt;
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
                self.move_left(shift);
                InputEffect::Navigate
            }
            "right" => {
                self.move_right(shift);
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
                if self.backspace() {
                    InputEffect::QueryChanged
                } else {
                    InputEffect::Navigate
                }
            }
            "delete" => {
                if self.delete_forward() {
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

    fn move_up(&mut self) {
        if self.selected == 0 {
            self.previous_selected = None;
            self.register_nav(NavDirection::Up);
            self.edge_hit = Some(EdgeHit::Top);
            return;
        }

        self.previous_selected = Some(self.selected);
        self.selected -= 1;
        self.register_nav(NavDirection::Up);
        self.edge_hit = None;
    }

    fn move_down(&mut self, result_count: usize) {
        if result_count == 0 {
            return;
        }

        let max = result_count.saturating_sub(1);
        if self.selected >= max {
            self.previous_selected = None;
            self.register_nav(NavDirection::Down);
            self.edge_hit = Some(EdgeHit::Bottom);
            return;
        }

        self.previous_selected = Some(self.selected);
        self.selected += 1;
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
    }

    fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        let start = self.cursor - 1;
        let start_b = Self::char_to_byte_index(&self.query, start);
        let end_b = Self::char_to_byte_index(&self.query, self.cursor);
        self.query.replace_range(start_b..end_b, "");
        self.cursor = start;
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let len = self.query_len();
        if self.cursor >= len {
            return false;
        }
        let start_b = Self::char_to_byte_index(&self.query, self.cursor);
        let end_b = Self::char_to_byte_index(&self.query, self.cursor + 1);
        self.query.replace_range(start_b..end_b, "");
        true
    }

    fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.query_len();
    }

    fn move_left(&mut self, selecting: bool) {
        let old = self.cursor;
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.update_selection_anchor(selecting, old);
    }

    fn move_right(&mut self, selecting: bool) {
        let old = self.cursor;
        let len = self.query_len();
        if self.cursor < len {
            self.cursor += 1;
        }
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
    use crate::discovery::search::Fuzziness;

    #[test]
    fn control_arrow_right_boosts() {
        let mut state = LauncherState::new();

        assert_eq!(
            state.apply_key("right", false, true, false, false, 0),
            InputEffect::BoostUp
        );
    }

    #[test]
    fn secondary_arrow_right_boosts() {
        let mut state = LauncherState::new();

        assert_eq!(
            state.apply_key("right", true, false, false, false, 0),
            InputEffect::BoostUp
        );
    }

    #[test]
    fn alt_arrow_right_still_boosts() {
        let mut state = LauncherState::new();

        assert_eq!(
            state.apply_key("right", false, false, false, true, 0),
            InputEffect::BoostUp
        );
    }

    #[test]
    fn fast_direction_reversal_is_ignored_as_phantom() {
        let mut state = LauncherState::new();

        assert_eq!(
            state.apply_key("down", false, false, false, false, 7),
            InputEffect::Navigate
        );
        assert_eq!(state.selected, 1);

        assert_eq!(
            state.apply_key("up", false, false, false, false, 7),
            InputEffect::Ignore,
            "an up arriving within the phantom window must not move"
        );
        assert_eq!(state.selected, 1, "selection holds against the phantom up");
    }

    #[test]
    fn deliberate_reversal_after_delay_still_navigates() {
        use std::time::{Duration, Instant};

        let mut state = LauncherState::new();
        state.apply_key("down", false, false, false, false, 7);
        assert_eq!(state.selected, 1);

        state.last_nav_at = Some(Instant::now() - Duration::from_millis(250));
        assert_eq!(
            state.apply_key("up", false, false, false, false, 7),
            InputEffect::Navigate,
            "a human-paced reversal is real navigation"
        );
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn repeated_same_direction_is_never_treated_as_phantom() {
        let mut state = LauncherState::new();
        let cases = [1usize, 2, 3, 4];
        for expected in cases {
            assert_eq!(
                state.apply_key("down", false, false, false, false, 7),
                InputEffect::Navigate,
                "down #{expected} must navigate"
            );
            assert_eq!(state.selected, expected, "down #{expected} advances");
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
        let mut store = EntryStore::new(apps, files);

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

        let effect = state.apply_key("down", false, false, false, false, result_count);
        assert_eq!(effect, InputEffect::Navigate);
        assert_eq!(
            state.selected, 1,
            "down advances once navigation reads the live count, not the stale 0"
        );
    }

    #[test]
    fn secondary_shortcuts_still_work() {
        let mut state = LauncherState::new();
        state.query = "abc".to_string();
        state.cursor = 1;

        assert_eq!(
            state.apply_key("a", true, false, false, false, 0),
            InputEffect::Navigate
        );
        assert_eq!(state.selected_range(), Some((0, 3)));

        assert_eq!(state.fuzziness, Fuzziness::Balanced);
        assert_eq!(
            state.apply_key("down", true, false, false, false, 0),
            InputEffect::QueryChanged
        );
        assert_eq!(state.fuzziness, Fuzziness::Loose);
    }
}
