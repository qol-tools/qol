use gpui::KeyDownEvent;

use super::state::LauncherState;

pub enum InputEffect {
    Ignore,
    Notify,
    Launch,
}

impl LauncherState {
    pub fn apply_key(&mut self, event: &KeyDownEvent, result_count: usize) -> InputEffect {
        let key = &event.keystroke.key;
        let modifiers = &event.keystroke.modifiers;
        let ctrl = modifiers.control;
        let shift = modifiers.shift;

        match key.as_str() {
            "left" => self.move_left(shift),
            "right" => self.move_right(shift),
            "home" => self.move_home(shift),
            "end" => self.move_end(shift),
            "up" => self.move_up(),
            "down" => self.move_down(result_count),
            "enter" => return InputEffect::Launch,
            "backspace" => self.backspace(),
            "delete" => self.delete_forward(),
            "a" if ctrl => self.select_all(),
            "space" if !ctrl => self.insert_char(' '),
            _ => {
                if ctrl || modifiers.alt {
                    return InputEffect::Ignore;
                }
                let Some(ch) = typeable_char(key, shift) else {
                    return InputEffect::Ignore;
                };
                self.insert_char(ch);
            }
        }

        self.selected = 0;
        InputEffect::Notify
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self, result_count: usize) {
        let max = result_count.saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
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

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = self.cursor - 1;
        let start_b = Self::char_to_byte_index(&self.query, start);
        let end_b = Self::char_to_byte_index(&self.query, self.cursor);
        self.query.replace_range(start_b..end_b, "");
        self.cursor = start;
    }

    fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let len = self.query_len();
        if self.cursor >= len {
            return;
        }
        let start_b = Self::char_to_byte_index(&self.query, self.cursor);
        let end_b = Self::char_to_byte_index(&self.query, self.cursor + 1);
        self.query.replace_range(start_b..end_b, "");
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

fn typeable_char(key: &str, shift: bool) -> Option<char> {
    let ch = key.chars().next().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')?;
    Some(if shift { ch.to_ascii_uppercase() } else { ch })
}
