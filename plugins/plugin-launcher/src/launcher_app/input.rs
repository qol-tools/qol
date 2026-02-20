use gpui::KeyDownEvent;

use super::state::{EdgeHit, LauncherState, NavDirection};

pub enum InputEffect {
    Ignore,
    Navigate,
    QueryChanged,
    Launch,
    Dismiss,
}

impl LauncherState {
    pub fn apply_key(&mut self, event: &KeyDownEvent, result_count: usize) -> InputEffect {
        let key = &event.keystroke.key;
        let modifiers = &event.keystroke.modifiers;
        let ctrl = modifiers.control;
        let shift = modifiers.shift;

        match key.as_str() {
            "escape" | "esc" => InputEffect::Dismiss,
            "up" if ctrl => {
                if self.decrease_fuzziness() { InputEffect::QueryChanged } else { InputEffect::Navigate }
            }
            "down" if ctrl => {
                if self.increase_fuzziness() { InputEffect::QueryChanged } else { InputEffect::Navigate }
            }
            "tab" => {
                self.cycle_mode(shift);
                InputEffect::QueryChanged
            }
            "left" => { self.move_left(shift); InputEffect::Navigate }
            "right" => { self.move_right(shift); InputEffect::Navigate }
            "home" => { self.move_home(shift); InputEffect::Navigate }
            "end" => { self.move_end(shift); InputEffect::Navigate }
            "up" if !ctrl => { self.move_up(); InputEffect::Navigate }
            "down" if !ctrl => { self.move_down(result_count); InputEffect::Navigate }
            "enter" => InputEffect::Launch,
            "backspace" => {
                if self.backspace() { InputEffect::QueryChanged } else { InputEffect::Navigate }
            }
            "delete" => {
                if self.delete_forward() { InputEffect::QueryChanged } else { InputEffect::Navigate }
            }
            "a" if ctrl => { self.select_all(); InputEffect::Navigate }
            "space" if !ctrl => {
                self.insert_char(' ');
                InputEffect::QueryChanged
            }
            _ => {
                if ctrl || modifiers.alt {
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
        if selecting {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(old_cursor);
            }
        } else {
            self.clear_selection();
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
